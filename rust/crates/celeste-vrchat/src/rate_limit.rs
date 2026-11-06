//! Port of `net.sybyline.scarlet.util.VrcRateLimiter`.
//!
//! A single global gate on all VRChat traffic, complementing the per-feature
//! pacers elsewhere. Those stop one feature spamming one operation; this
//! bounds the *aggregate* rate across every feature at once, which is the
//! situation that actually trips VRChat's limit — Java's own comment names
//! the scenario precisely: "joining a busy instance fires audit polling,
//! avatar hydration for dozens of players, and user/group lookups
//! simultaneously."
//!
//! The port had no 429 handling at all. The visible symptom was players
//! whose trust rank read "Unknown" and whose account age was blank, because
//! their profile lookup lost a race against the limit and the failure was
//! swallowed. Caching the join-path lookups cut the volume; this bounds it.
//!
//! Two mechanisms, both Java's:
//!
//! 1. **Burst smoothing** — a token bucket caps the sustained rate while
//!    letting short bursts through.
//! 2. **Global backoff** — a 429 anywhere eases the *whole* client off,
//!    honouring `Retry-After`, rather than only the feature that was
//!    unlucky enough to receive it.

use std::sync::Mutex;
use std::time::Duration;

/// Java: `new VrcRateLimiter(10.0, 20.0)` (`ScarletVRChat.java:326`).
pub const PERMITS_PER_SECOND: f64 = 10.0;
pub const BURST: f64 = 20.0;

/// Java's `MAX_SLEEP_CHUNK_MS`: never sleep longer than this in one go, so a
/// backoff that is lifted early is noticed promptly.
const MAX_SLEEP_CHUNK: Duration = Duration::from_millis(1_000);

/// Java's default when a 429 carries no usable `Retry-After`.
pub const DEFAULT_BACKOFF: Duration = Duration::from_secs(30);
/// Java clamps `Retry-After` into this range, so a hostile or mistaken
/// value cannot stall the client for minutes.
pub const MIN_BACKOFF: Duration = Duration::from_secs(1);
pub const MAX_BACKOFF: Duration = Duration::from_secs(300);

struct Bucket {
    available: f64,
    last_refill_ms: i64,
    backoff_until_ms: i64,
    last_penalty_ms: i64,
}

pub struct VrcRateLimiter {
    max_permits: f64,
    refill_per_ms: f64,
    bucket: Mutex<Bucket>,
}

impl Default for VrcRateLimiter {
    fn default() -> Self {
        Self::new(PERMITS_PER_SECOND, BURST)
    }
}

impl VrcRateLimiter {
    pub fn new(permits_per_second: f64, burst: f64) -> Self {
        Self {
            max_permits: burst.max(1.0),
            refill_per_ms: permits_per_second.max(0.0001) / 1000.0,
            bucket: Mutex::new(Bucket {
                available: burst.max(1.0),
                last_refill_ms: now_ms(),
                backoff_until_ms: 0,
                last_penalty_ms: 0,
            }),
        }
    }

    /// Waits until any global backoff has elapsed and a token is free.
    ///
    /// Unlike Java's blocking `acquire`, this yields to the async runtime:
    /// blocking a tokio worker here would stall unrelated tasks, and the
    /// whole point is that this sits on the hot path of every request.
    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
                let now = now_ms();
                let until = bucket.backoff_until_ms;
                if until > now {
                    Some(Duration::from_millis(
                        (until - now).min(MAX_SLEEP_CHUNK.as_millis() as i64) as u64,
                    ))
                } else {
                    let elapsed = (now - bucket.last_refill_ms) as f64;
                    if elapsed > 0.0 {
                        bucket.available =
                            (bucket.available + elapsed * self.refill_per_ms).min(self.max_permits);
                        bucket.last_refill_ms = now;
                    }
                    if bucket.available >= 1.0 {
                        bucket.available -= 1.0;
                        None
                    } else {
                        let needed = ((1.0 - bucket.available) / self.refill_per_ms).ceil() as u64;
                        Some(Duration::from_millis(
                            needed.max(1).min(MAX_SLEEP_CHUNK.as_millis() as u64),
                        ))
                    }
                }
            };
            match wait {
                None => return,
                Some(wait) => tokio::time::sleep(wait).await,
            }
        }
    }

    /// Applies a global backoff, e.g. from a 429. Java keeps the *longer* of
    /// the existing and new deadlines, so overlapping 429s don't shorten one
    /// another.
    pub fn penalize(&self, backoff: Duration) {
        if backoff.is_zero() {
            return;
        }
        let mut bucket = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        bucket.last_penalty_ms = now;
        let until = now + backoff.as_millis() as i64;
        if until > bucket.backoff_until_ms {
            bucket.backoff_until_ms = until;
        }
    }

    /// How much global backoff is left, zero when not backing off.
    pub fn backoff_remaining(&self) -> Duration {
        let bucket = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
        let remaining = bucket.backoff_until_ms - now_ms();
        Duration::from_millis(remaining.max(0) as u64)
    }

    pub fn is_backing_off(&self) -> bool {
        !self.backoff_remaining().is_zero()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Port of `ScarletVRChat.parseRetryAfterMillis`: seconds, or an HTTP-date,
/// clamped so a hostile value cannot stall the client.
pub fn parse_retry_after(header: Option<&str>) -> Duration {
    let raw = header.map(str::trim).filter(|value| !value.is_empty());
    let parsed = raw.and_then(|value| {
        if let Ok(seconds) = value.parse::<i64>() {
            return Some(Duration::from_millis(
                seconds.saturating_mul(1000).max(0) as u64
            ));
        }
        // An HTTP-date: how long until then.
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|when| {
                let millis = when.timestamp_millis() - now_ms();
                Duration::from_millis(millis.max(0) as u64)
            })
    });
    parsed
        .unwrap_or(DEFAULT_BACKOFF)
        .clamp(MIN_BACKOFF, MAX_BACKOFF)
}

/// Sends a request through a limiter.
///
/// An extension trait rather than a method on the client so that wiring it
/// in is a pure suffix change at each call site — `.send().await?` becomes
/// `.send_limited(&self.limiter).await?` — with no expression to capture
/// and rewrite. Java gets this for free from okhttp's interceptor chain;
/// reqwest has no equivalent, and a sweep that has to parse each call site
/// is a sweep that can corrupt one.
pub trait SendLimited {
    /// Acquires a token, sends, and on a 429 backs the whole client off for
    /// as long as `Retry-After` asks.
    fn send_limited(
        self,
        limiter: &VrcRateLimiter,
    ) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>>;
}

/// A duration as somebody reading their own log would write it.
///
/// Log lines are read by people trying to work out why Celeste felt slow,
/// so `1.4s` beats `1400ms` and `243ms` beats `0.243s`.
fn human(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", duration.as_secs_f64())
    } else {
        format!("{}m {}s", duration.as_secs() / 60, duration.as_secs() % 60)
    }
}

impl SendLimited for reqwest::RequestBuilder {
    async fn send_limited(self, limiter: &VrcRateLimiter) -> reqwest::Result<reqwest::Response> {
        // Describe the request before sending, because `send` consumes the
        // builder. The path only: a query string can carry a display name
        // somebody searched for, which has no business in a log file.
        let described = self
            .try_clone()
            .and_then(|builder| builder.build().ok())
            .map(|request| {
                (
                    request.method().to_string(),
                    request.url().path().to_string(),
                )
            });
        let (method, path) = described.unwrap_or_else(|| ("?".to_string(), "?".to_string()));

        let backoff_before = limiter.backoff_remaining();
        if !backoff_before.is_zero() {
            tracing::info!(
                "Holding {method} {path} for up to {} — VRChat asked this client to slow down and it is still waiting that out",
                human(backoff_before)
            );
        }

        let queued_at = std::time::Instant::now();
        limiter.acquire().await;
        let waited = queued_at.elapsed();

        let sent_at = std::time::Instant::now();
        let response = match self.send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    "Could not reach VRChat for {method} {path} after {}: {error}",
                    human(sent_at.elapsed())
                );
                return Err(error);
            }
        };
        let took = sent_at.elapsed();

        // A wait under a tenth of a second is the token bucket doing its
        // job invisibly; saying so on every request would bury the ones
        // that actually queued.
        let queueing = if waited >= Duration::from_millis(100) {
            format!(" (waited {} for the rate limit first)", human(waited))
        } else {
            String::new()
        };
        tracing::debug!(
            "Asked VRChat: {method} {path} — {} in {}{queueing}",
            response.status(),
            human(took)
        );

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let backoff = parse_retry_after(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok()),
            );
            let told_us = if response.headers().contains_key("retry-after") {
                "VRChat said how long to wait"
            } else {
                "VRChat did not say how long, so this is Celeste's own default"
            };
            tracing::warn!(
                "VRChat refused {method} {path} for going too fast (HTTP 429). Every request is \
                 held for {} — {told_us}",
                human(backoff)
            );
            limiter.penalize(backoff);
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_reads_seconds_and_falls_back() {
        assert_eq!(parse_retry_after(Some("12")), Duration::from_secs(12));
        assert_eq!(parse_retry_after(None), DEFAULT_BACKOFF);
        assert_eq!(parse_retry_after(Some("   ")), DEFAULT_BACKOFF);
        assert_eq!(parse_retry_after(Some("nonsense")), DEFAULT_BACKOFF);
    }

    /// Java clamps so a hostile or mistaken header cannot stall the client
    /// for minutes, or busy-loop it with zero.
    #[test]
    fn retry_after_is_clamped_at_both_ends() {
        assert_eq!(parse_retry_after(Some("0")), MIN_BACKOFF);
        assert_eq!(parse_retry_after(Some("99999")), MAX_BACKOFF);
        assert_eq!(parse_retry_after(Some("-5")), MIN_BACKOFF);
        assert_eq!(parse_retry_after(Some(&i64::MAX.to_string())), MAX_BACKOFF);
    }

    #[tokio::test]
    async fn the_bucket_lets_a_burst_through_then_paces() {
        let limiter = VrcRateLimiter::new(1000.0, 3.0);
        // The burst is free.
        for _ in 0..3 {
            limiter.acquire().await;
        }
        // The fourth has to wait for a refill, so it must take measurable
        // time rather than passing straight through.
        let started = std::time::Instant::now();
        limiter.acquire().await;
        assert!(started.elapsed() >= Duration::from_micros(500));
    }

    /// The property that matters: one 429 eases off the *whole* client, not
    /// just the caller that received it.
    #[test]
    fn a_penalty_backs_off_globally_and_takes_the_longer_deadline() {
        let limiter = VrcRateLimiter::default();
        assert!(!limiter.is_backing_off());
        limiter.penalize(Duration::from_secs(30));
        assert!(limiter.is_backing_off());
        let long = limiter.backoff_remaining();
        // A shorter overlapping penalty must not cut the longer one short.
        limiter.penalize(Duration::from_secs(1));
        assert!(limiter.backoff_remaining() >= long - Duration::from_millis(50));
        // A zero penalty is a no-op, as in Java.
        limiter.penalize(Duration::ZERO);
        assert!(limiter.is_backing_off());
    }
}
