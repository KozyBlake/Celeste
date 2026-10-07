//! Background group-instance reconciliation and enforcement.
//!
//! Java runs `maybeCheckInstances` every five minutes and
//! `maybeEnforceInstances` every minute from its main loop. Without this
//! worker, Celeste could create instance-monitor threads and expose all the
//! enforcement settings while never noticing an instance disappeared and
//! never enforcing those settings.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration, Utc};
use celeste_core::ui::CelesteUi;
use celeste_core::settings::enums::{EnforcementAgeState, EnforcementListState};
use celeste_core::settings::Settings;
use serde_json::Value;

use crate::data::SharedVrChatClient;
use crate::events::audit_emitter::DiscordAuditEmitter;

pub async fn run(
    vrchat: SharedVrChatClient,
    settings: Arc<Settings>,
    data: Arc<celeste_core::data::CelesteData>,
    emitter: Arc<DiscordAuditEmitter>,
    ui: Arc<dyn CelesteUi>,
) {
    let mut auto_close = AutoCloseTracker::default();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let Some(client) = vrchat.read().await.clone() else { continue };
        maybe_auto_close_bot_instance(&client, &settings, &emitter, ui.as_ref(), &mut auto_close).await;
        let Some(group_id) = settings
            .store
            .get_string(celeste_vrchat::client::GROUP_ID_KEY)
            .filter(|id| !id.trim().is_empty())
        else {
            continue;
        };

        let now = Utc::now();
        let check_inactive = settings
            .last_instances_check
            .get_or_supply()
            .is_none_or(|last| now > last + Duration::minutes(5));
        if check_inactive {
            // Java advances the cursor before attempting the API call, so a
            // temporary failure does not turn into a tight retry loop.
            settings.last_instances_check.set(Some(now));
        }

        let instances = match client.get_group_instances(&group_id).await {
            Ok(instances) => instances,
            Err(error) => {
                tracing::warn!(%error, group_id, "Could not reconcile live group instances");
                continue;
            }
        };

        if check_inactive {
            reconcile_inactive(&instances, &data, &emitter).await;
        }
        enforce_instances(&instances, &client, &settings, &emitter).await;
    }
}

/// Java's low-latency `maybeFastDetectNewInstance`: use the live instance
/// endpoint as a short-cadence signal, but adopt the first successful result
/// as a baseline so enabling Celeste never launches into an old instance.
pub async fn run_fast_follow(
    vrchat: SharedVrChatClient,
    settings: Arc<Settings>,
    ui: Arc<dyn CelesteUi>,
) {
    let mut known_locations = HashSet::new();
    let mut baseline_seeded = false;
    loop {
        if !settings.vrchat_client_launch_on_instance_create.get() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }
        let configured_seconds = settings.instance_follow_fast_poll_seconds.get();
        if configured_seconds <= 0 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }
        let Some(group_id) = settings
            .store
            .get_string(celeste_vrchat::client::GROUP_ID_KEY)
            .filter(|id| !id.trim().is_empty())
        else {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            continue;
        };
        let Some(client) = vrchat.read().await.clone() else {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            continue;
        };
        let instances = match client.get_group_instances(&group_id).await {
            Ok(instances) => instances,
            Err(error) => {
                tracing::warn!(%error, group_id, "Fast instance detection failed; backing off for one minute");
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
        };
        let current: HashSet<String> = instances
            .iter()
            .filter_map(instance_location)
            .map(str::to_string)
            .collect();
        if !baseline_seeded {
            known_locations = current;
            baseline_seeded = true;
        } else {
            let bot_location = ui.current_instance().map(|instance| instance.location);
            let new_location = newly_opened_location(&current, &known_locations, bot_location.as_deref());
            known_locations = current;
            if let Some(location) = new_location {
                tracing::info!(%location, "Fast instance detection found a new group instance; following");
                launch_client_into_instance_if_enabled(&client, &settings, &location).await;
                super::audit_poller::poll_soon();
            }
        }
        let period = u64::try_from(configured_seconds.max(15)).unwrap_or(15);
        tokio::time::sleep(std::time::Duration::from_secs(period)).await;
    }
}

fn newly_opened_location(
    current: &HashSet<String>,
    known: &HashSet<String>,
    bot_location: Option<&str>,
) -> Option<String> {
    current
        .iter()
        .find(|location| !known.contains(*location) && bot_location != Some(location.as_str()))
        .cloned()
}

pub async fn launch_client_into_instance_if_enabled(
    client: &celeste_vrchat::client::VrChatClient,
    settings: &Settings,
    location: &str,
) {
    if !settings.vrchat_client_launch_on_instance_create.get()
        || !celeste_vrchat::launch::claim_recent_launch(location)
    {
        return;
    }
    let Some((world_id, instance_id)) = concrete_location_parts(location) else {
        tracing::warn!(location, "Could not parse new instance location for automatic follow");
        return;
    };
    let secure_name = match client.get_instance_secure_name(world_id, instance_id).await {
        Ok(name) => name,
        Err(error) => {
            tracing::warn!(%error, location, "Could not obtain the instance secure join token; trying the general instance response");
            client.get_instance(world_id, instance_id).await.ok().and_then(|instance| {
                instance
                    .secure_name
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| instance.short_name.filter(|name| !name.trim().is_empty()))
            })
        }
    };
    let location = location.to_string();
    if let Err(error) = tokio::task::spawn_blocking(move || {
        // The legacy single-client follow option owns local profile zero.
        // Managed multi-client slots use their own explicit numeric profile.
        celeste_vrchat::launch::relaunch_desktop(0, &location, secure_name.as_deref())
    })
    .await
    .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())))
    {
        tracing::error!(%error, "Could not launch VRChat into the new group instance");
    }
}

#[derive(Debug, Default)]
struct AutoCloseTracker {
    location: Option<String>,
    self_user_id: Option<String>,
    idle_since: Option<chrono::DateTime<Utc>>,
}

impl AutoCloseTracker {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Returns true once the same location has remained bot-only for the
    /// configured duration. Any other player resets the full timer, matching
    /// Java's `maybeAutoCloseBotInstance` exactly.
    fn observe(
        &mut self,
        location: &str,
        self_user_id: &str,
        active_player_ids: &[String],
        minutes: i32,
        now: chrono::DateTime<Utc>,
    ) -> bool {
        if self.location.as_deref() != Some(location) {
            self.location = Some(location.to_string());
            self.self_user_id = Some(self_user_id.to_string());
            self.idle_since = None;
        }
        if active_player_ids.iter().any(|id| id != self_user_id) {
            self.idle_since = None;
            return false;
        }
        let Some(idle_since) = self.idle_since else {
            self.idle_since = Some(now);
            return false;
        };
        now >= idle_since + Duration::minutes(i64::from(minutes))
    }
}

async fn maybe_auto_close_bot_instance(
    client: &celeste_vrchat::client::VrChatClient,
    settings: &Settings,
    emitter: &DiscordAuditEmitter,
    ui: &dyn CelesteUi,
    tracker: &mut AutoCloseTracker,
) {
    let minutes = settings.auto_close_bot_instance_minutes.get();
    let Some(instance) = ui.current_instance() else {
        tracker.reset();
        return;
    };
    if minutes <= 0 {
        tracker.reset();
        return;
    }

    if tracker.location.as_deref() != Some(instance.location.as_str()) {
        let Ok(current_user) = client.get_current_user().await else {
            // Never guess which roster entry is Celeste itself. A failed
            // identity lookup postpones closing rather than risking an
            // occupied instance.
            tracker.reset();
            return;
        };
        tracker.self_user_id = Some(current_user.id);
    }
    let Some(self_user_id) = tracker.self_user_id.clone() else { return };
    let players = ui.active_player_ids();
    if !tracker.observe(&instance.location, &self_user_id, &players, minutes, Utc::now()) {
        return;
    }
    // Java performs one final roster check immediately before acting.
    if ui.active_player_ids().iter().any(|id| id != &self_user_id) {
        tracker.idle_since = None;
        return;
    }
    let Some((world_id, instance_id)) = concrete_location_parts(&instance.location) else {
        tracing::warn!(location = instance.location, "Auto-close could not parse the bot's instance location");
        tracker.reset();
        return;
    };
    let world_name = instance.world_name.as_deref().unwrap_or(world_id);
    match client.close_instance(world_id, instance_id, Some(true), None).await {
        Ok(_) => {
            let reason = format!("Auto-closed: idle ({minutes} min, bot only)");
            tracing::info!(location = instance.location, world_name, minutes, "Auto-closed idle bot instance");
            emitter.emit_instance_enforcement(&instance.location, world_name, &reason).await;
        }
        Err(error) => {
            tracing::warn!(%error, location = instance.location, world_name, "Failed to auto-close idle bot instance");
        }
    }
    tracker.reset();
}

fn concrete_location_parts(location: &str) -> Option<(&str, &str)> {
    let (world_id, instance_id) = location.split_once(':')?;
    (!world_id.is_empty() && !instance_id.is_empty()).then_some((world_id, instance_id))
}

async fn reconcile_inactive(
    instances: &[Value],
    data: &celeste_core::data::CelesteData,
    emitter: &DiscordAuditEmitter,
) {
    let active: HashSet<&str> = instances.iter().filter_map(instance_location).collect();
    for location in data.live_instances_get_locations() {
        if active.contains(location.as_str()) {
            continue;
        }
        // Java removes both durable routing records before emitting the
        // inactive notice; the notice therefore goes to the configured
        // parent channel rather than a thread that has just ended.
        data.live_instances_get_location_audit(&location, true);
        data.live_instances_get_location_instance_embed_message(&location, true);
        emitter.emit_instance_inactive(&location).await;
    }
}

async fn enforce_instances(
    instances: &[Value],
    client: &celeste_vrchat::client::VrChatClient,
    settings: &Settings,
    emitter: &DiscordAuditEmitter,
) {
    let age_state = settings.enforce_instances_18_plus.get();
    let list_state = settings.enforce_instances_worlds.get();
    if age_state == EnforcementAgeState::Disabled && list_state == EnforcementListState::Disabled {
        return;
    }
    let world_list = settings.enforce_instances_world_list.get();
    for instance in instances {
        let Some(location) = instance_location(instance) else { continue };
        let Some(world_id) = instance.pointer("/world/id").and_then(Value::as_str) else { continue };
        let Some(instance_id) = instance.get("instanceId").and_then(Value::as_str) else { continue };
        let world_name = instance
            .pointer("/world/name")
            .and_then(Value::as_str)
            .unwrap_or(world_id);
        let Some(reason) = enforcement_reason(location, world_id, age_state, list_state, &world_list) else { continue };

        match client.close_instance(world_id, instance_id, Some(true), None).await {
            Ok(_) => {
                tracing::info!(location, world_name, reason, "Closed group instance by enforcement policy");
                emitter.emit_instance_enforcement(location, world_name, reason).await;
            }
            Err(error) => {
                tracing::warn!(%error, location, world_name, reason, "Failed to close group instance by enforcement policy");
            }
        }
    }
}

fn instance_location(instance: &Value) -> Option<&str> {
    instance.get("location").and_then(Value::as_str).filter(|location| !location.is_empty())
}

fn enforcement_reason(
    location: &str,
    world_id: &str,
    age_state: EnforcementAgeState,
    list_state: EnforcementListState,
    world_list: &[String],
) -> Option<&'static str> {
    let listed = world_list.iter().any(|listed| listed == world_id);
    match list_state {
        EnforcementListState::EnabledWhitelist if !listed => return Some("World not whitelisted"),
        EnforcementListState::EnabledBlacklist if listed => return Some("World blacklisted"),
        _ => {}
    }
    let age_gated = location.split('~').any(|part| part == "ageGate");
    match age_state {
        EnforcementAgeState::EnabledNever18Plus if age_gated => Some("Age-gating disallowed"),
        EnforcementAgeState::EnabledOnly18Plus if !age_gated => Some("Age-gating mandatory"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_policy_takes_precedence_over_age_policy_like_java() {
        let list = vec!["wrld_allowed".to_string()];
        assert_eq!(
            enforcement_reason(
                "wrld_other:1~ageGate",
                "wrld_other",
                EnforcementAgeState::EnabledNever18Plus,
                EnforcementListState::EnabledWhitelist,
                &list,
            ),
            Some("World not whitelisted")
        );
    }

    #[test]
    fn age_gate_is_read_from_a_complete_location_segment() {
        let disabled = EnforcementListState::Disabled;
        let empty = Vec::new();
        assert_eq!(
            enforcement_reason(
                "wrld_1:1~group(grp_1)~ageGate~region(us)",
                "wrld_1",
                EnforcementAgeState::EnabledNever18Plus,
                disabled,
                &empty,
            ),
            Some("Age-gating disallowed")
        );
        assert_eq!(
            enforcement_reason(
                "wrld_1:1~group(grp_1)~region(us)",
                "wrld_1",
                EnforcementAgeState::EnabledOnly18Plus,
                disabled,
                &empty,
            ),
            Some("Age-gating mandatory")
        );
    }

    #[test]
    fn auto_close_requires_a_full_uninterrupted_bot_only_period() {
        let mut tracker = AutoCloseTracker::default();
        let start = Utc::now();
        let location = "wrld_1:123~group(grp_1)";
        let self_id = "usr_self";
        assert!(!tracker.observe(location, self_id, &[self_id.into()], 5, start));
        assert!(!tracker.observe(location, self_id, &[self_id.into(), "usr_other".into()], 5, start + Duration::minutes(4)));
        assert!(!tracker.observe(location, self_id, &[self_id.into()], 5, start + Duration::minutes(5)));
        assert!(tracker.observe(location, self_id, &[self_id.into()], 5, start + Duration::minutes(10)));
    }

    #[test]
    fn changing_location_restarts_the_idle_timer() {
        let mut tracker = AutoCloseTracker::default();
        let start = Utc::now();
        assert!(!tracker.observe("wrld_1:one", "usr_self", &[], 1, start));
        assert!(!tracker.observe("wrld_1:two", "usr_self", &[], 1, start + Duration::minutes(2)));
        assert!(tracker.observe("wrld_1:two", "usr_self", &[], 1, start + Duration::minutes(3)));
        assert_eq!(concrete_location_parts("wrld_1:two~group(grp_1)"), Some(("wrld_1", "two~group(grp_1)")));
        assert_eq!(concrete_location_parts("wrld_1"), None);
    }

    #[test]
    fn fast_follow_ignores_the_baseline_and_the_bots_current_location() {
        let known = HashSet::from(["wrld_1:old".to_string()]);
        let current = HashSet::from([
            "wrld_1:old".to_string(),
            "wrld_1:bot".to_string(),
            "wrld_1:new".to_string(),
        ]);
        assert_eq!(
            newly_opened_location(&current, &known, Some("wrld_1:bot")).as_deref(),
            Some("wrld_1:new")
        );
        assert_eq!(newly_opened_location(&known, &known, None), None);
    }
}
