//! Operational health notices sent to Discord's configured
//! `opsAlertChannelSf`. Incidents are stateful: one outage produces one alert
//! and one recovery notice even when the underlying retry loop fails many
//! times. This keeps an operations channel useful instead of turning it into
//! a retry log.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serenity::builder::CreateMessage;
use serenity::http::Http;
use serenity::model::id::ChannelId;

use crate::data::{DiscordConfigStore, SharedHttp};

const OPS_ALERT_CHANNEL_FIELD: &str = "opsAlertChannelSf";

#[derive(Debug, Clone)]
struct Incident {
    started: Instant,
    attempts: u64,
    detail: String,
}

#[derive(Default)]
struct HealthState {
    startup_announced: bool,
    incidents: HashMap<&'static str, Incident>,
    pending_messages: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum IncidentChange {
    Opened,
    Updated,
}

impl HealthState {
    fn open_incident(&mut self, subsystem: &'static str, detail: &str) -> IncidentChange {
        match self.incidents.get_mut(subsystem) {
            Some(incident) => {
                incident.attempts = incident.attempts.saturating_add(1);
                incident.detail = detail.to_string();
                IncidentChange::Updated
            }
            None => {
                self.incidents.insert(
                    subsystem,
                    Incident {
                        started: Instant::now(),
                        attempts: 1,
                        detail: detail.to_string(),
                    },
                );
                IncidentChange::Opened
            }
        }
    }

    fn close_incident(&mut self, subsystem: &'static str) -> Option<Incident> {
        self.incidents.remove(subsystem)
    }
}

/// Shared health reporter used by both the Discord supervisor and Celeste's
/// VRChat session watchdog.
pub struct OpsHealthReporter {
    http: SharedHttp,
    config: Arc<DiscordConfigStore>,
    state: Mutex<HealthState>,
}

impl OpsHealthReporter {
    pub fn new(http: SharedHttp, config: Arc<DiscordConfigStore>) -> Self {
        Self {
            http,
            config,
            state: Mutex::new(HealthState::default()),
        }
    }

    /// Confirms that the configured route works. Called from serenity's
    /// `ready` event, so it does not mistake a merely-built HTTP client for a
    /// live Discord gateway.
    pub async fn discord_ready(&self, http: Arc<Http>) {
        let route_is_configured = self.configured_channel().is_some();
        let recovery = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let pending = std::mem::take(&mut state.pending_messages);
            let recovery = state.close_incident("Discord gateway");
            let announce_startup = route_is_configured && !state.startup_announced;
            state.startup_announced |= route_is_configured;
            (pending, recovery, announce_startup)
        };

        let mut outbound = recovery.0;
        if let Some(incident) = recovery.1 {
            outbound.push(format!(
                "🟢 **Discord gateway recovered** after {} ({} connection {}). Celeste commands and event delivery are available again.",
                format_duration(incident.started.elapsed()),
                incident.attempts,
                if incident.attempts == 1 { "attempt" } else { "attempts" },
            ));
        } else if recovery.2 {
            outbound.push(format!(
                "🟢 **Celeste health reporting is active** · version {}\nDiscord is connected and operational incidents will be reported here.",
                scarlet_core::identity::VERSION,
            ));
        }

        let mut outbound = outbound.into_iter();
        while let Some(message) = outbound.next() {
            if !self.send_with(&http, message.clone()).await {
                self.queue(message);
                for unsent in outbound {
                    self.queue(unsent);
                }
                break;
            }
        }
    }

    /// Opens or updates an incident. Only the first failure is posted; later
    /// calls increment the attempt count included in the recovery notice.
    pub async fn incident(&self, subsystem: &'static str, detail: impl Into<String>) {
        let detail = detail.into();
        let opened = {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .open_incident(subsystem, &detail)
                == IncidentChange::Opened
        };
        if opened {
            self.send(format!("🔴 **{subsystem} incident**\n{detail}"))
                .await;
        }
    }

    /// Resolves an active incident. Healthy probes with no preceding incident
    /// are deliberately silent.
    pub async fn recovered(&self, subsystem: &'static str, detail: impl Into<String>) {
        let incident = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .close_incident(subsystem);
        let Some(incident) = incident else { return };
        self.send(format!(
            "🟢 **{subsystem} recovered** after {} ({} recovery {}).\n{}",
            format_duration(incident.started.elapsed()),
            incident.attempts,
            if incident.attempts == 1 {
                "attempt"
            } else {
                "attempts"
            },
            detail.into(),
        ))
        .await;
    }

    async fn send(&self, content: String) {
        let Some(http) = self.http.read().await.clone() else {
            tracing::warn!("Could not deliver ops health alert while Discord is disconnected");
            self.queue(content);
            return;
        };
        if !self.send_with(&http, content.clone()).await {
            self.queue(content);
        }
    }

    async fn send_with(&self, http: &Http, content: String) -> bool {
        let Some(channel) = self.configured_channel() else {
            return true;
        };
        if let Err(error) = channel
            .send_message(http, CreateMessage::new().content(content))
            .await
        {
            tracing::warn!(%error, channel_id = channel.get(), "Could not deliver ops health alert");
            return false;
        }
        true
    }

    fn queue(&self, content: String) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending_messages
            .push(content);
    }

    fn configured_channel(&self) -> Option<ChannelId> {
        self.config
            .string(OPS_ALERT_CHANNEL_FIELD)
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|id| *id != 0)
            .map(ChannelId::new)
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_failures_stay_one_incident_and_count_attempts() {
        let mut state = HealthState::default();
        assert_eq!(
            state.open_incident("Discord gateway", "first"),
            IncidentChange::Opened
        );
        assert_eq!(
            state.open_incident("Discord gateway", "second"),
            IncidentChange::Updated
        );
        let incident = state
            .close_incident("Discord gateway")
            .expect("incident remains open");
        assert_eq!(incident.attempts, 2);
        assert_eq!(incident.detail, "second");
        assert!(state.close_incident("Discord gateway").is_none());
    }

    #[test]
    fn duration_text_stays_compact_for_discord() {
        assert_eq!(format_duration(Duration::from_secs(9)), "9s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2m 5s");
        assert_eq!(format_duration(Duration::from_secs(7_500)), "2h 5m");
    }
}
