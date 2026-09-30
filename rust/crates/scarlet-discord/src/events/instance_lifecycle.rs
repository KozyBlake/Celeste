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
use scarlet_core::settings::enums::{EnforcementAgeState, EnforcementListState};
use scarlet_core::settings::Settings;
use serde_json::Value;

use crate::data::SharedVrChatClient;
use crate::events::audit_emitter::DiscordAuditEmitter;

pub async fn run(
    vrchat: SharedVrChatClient,
    settings: Arc<Settings>,
    data: Arc<scarlet_core::data::ScarletData>,
    emitter: Arc<DiscordAuditEmitter>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let Some(client) = vrchat.read().await.clone() else { continue };
        let Some(group_id) = settings
            .store
            .get_string(scarlet_vrchat::client::GROUP_ID_KEY)
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

async fn reconcile_inactive(
    instances: &[Value],
    data: &scarlet_core::data::ScarletData,
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
    client: &scarlet_vrchat::client::VrChatClient,
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
}
