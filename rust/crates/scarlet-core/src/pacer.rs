//! Port of `net.sybyline.scarlet.util.Pacer`.
//!
//! A minimum-interval gate: each [`Pacer::await_turn`] returns no sooner than
//! `millis` after the previous one did. `ScarletEventListener` uses one to
//! keep catch-up player checks from bursting dozens of VRChat API calls the
//! moment Celeste starts against a log with a full instance in it.
//!
//! Java blocks the calling thread with `Thread.sleep`. This port keeps the
//! blocking shape (`await_turn`) for callers already on a worker thread, and
//! adds an async [`Pacer::wait`] for callers on a tokio task — the log-event
//! pipeline here is async, and blocking a runtime worker to pace API calls
//! would stall every other task on that worker.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Pacer {
    interval: Duration,
    /// When the previous turn was taken. Starts one full interval in the
    /// past so the first call never waits, matching Java's
    /// `epoch = currentTimeMillis() - millis`.
    epoch: Mutex<Instant>,
}

impl Pacer {
    /// `millis` is clamped to at least 1, as Java does.
    pub fn new(millis: u64) -> Self {
        let interval = Duration::from_millis(millis.max(1));
        Pacer { interval, epoch: Mutex::new(Instant::now() - interval) }
    }

    /// How long the next turn must wait, and reserves that slot. Splitting
    /// this out is what lets the sync and async waits share one policy.
    fn reserve(&self) -> Duration {
        let mut epoch = self.epoch.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let next = *epoch + self.interval;
        let layover = next.saturating_duration_since(now);
        *epoch = now + layover;
        layover
    }

    /// Blocking form, matching Java's `await()`.
    pub fn await_turn(&self) {
        let layover = self.reserve();
        if !layover.is_zero() {
            std::thread::sleep(layover);
        }
    }

    /// Async form for callers running on the tokio runtime.
    pub async fn wait(&self) {
        let layover = self.reserve();
        if !layover.is_zero() {
            tokio::time::sleep(layover).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_does_not_wait_and_later_calls_are_spaced() {
        let pacer = Pacer::new(50);
        let started = Instant::now();
        pacer.await_turn();
        assert!(started.elapsed() < Duration::from_millis(25), "the first turn is free");
        pacer.await_turn();
        pacer.await_turn();
        // Two further turns at 50ms apart each.
        assert!(started.elapsed() >= Duration::from_millis(100), "turns should be paced apart");
    }

    #[test]
    fn interval_is_clamped_to_at_least_one_millisecond() {
        let pacer = Pacer::new(0);
        pacer.await_turn();
        pacer.await_turn();
    }
}
