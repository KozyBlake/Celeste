//! Desired-state supervisor for explicitly configured VRChat coverage slots.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use scarlet_vrchat::managed_clients::{
    discover_log_sessions, location_from_instance, split_location, ClientLogSession,
    ManagedClientSlot, ManagedClientStore,
};

const POLL: Duration = Duration::from_secs(15);
const OUTAGE_BACKOFF: Duration = Duration::from_secs(60);
const ACTION_COOLDOWN: Duration = Duration::from_secs(90);
const CREATE_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const RESTART_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_RESTARTS: usize = 3;

#[derive(Default)]
struct SlotRuntime {
    next_action: Option<Instant>,
    launches: VecDeque<Instant>,
}

pub fn spawn(
    handle: &tokio::runtime::Handle,
    vrchat: scarlet_discord::data::SharedVrChatClient,
    settings: Arc<scarlet_core::settings::Settings>,
    store: Arc<ManagedClientStore>,
) {
    handle.spawn(run(vrchat, settings, store));
}

async fn run(
    vrchat: scarlet_discord::data::SharedVrChatClient,
    settings: Arc<scarlet_core::settings::Settings>,
    store: Arc<ManagedClientStore>,
) {
    let mut runtime: HashMap<String, SlotRuntime> = HashMap::new();
    loop {
        let slots = store.list();
        let enabled = slots.into_iter().filter(|slot| slot.enabled).collect::<Vec<_>>();
        if enabled.is_empty() {
            tokio::time::sleep(POLL).await;
            continue;
        }
        let Some(client) = vrchat.read().await.clone() else {
            for slot in enabled {
                update(&store, &slot.id, None, "Waiting for Celeste's VRChat API sign-in", "");
            }
            tokio::time::sleep(POLL).await;
            continue;
        };
        let Some(group_id) = settings
            .store
            .get_string(scarlet_vrchat::client::GROUP_ID_KEY)
            .filter(|id| !id.trim().is_empty())
        else {
            for slot in enabled {
                update(&store, &slot.id, None, "Paused", "Configure a VRChat group ID before managing clients");
            }
            tokio::time::sleep(OUTAGE_BACKOFF).await;
            continue;
        };

        // One shared health gate. If this call fails, assume connectivity or
        // VRChat is unavailable and do not turn a global outage into a wave
        // of process restarts.
        let group_instances = match client.get_group_instances(&group_id).await {
            Ok(instances) => instances,
            Err(error) => {
                for slot in enabled {
                    update(&store, &slot.id, None, "Waiting for VRChat connectivity", error.to_string());
                }
                tokio::time::sleep(OUTAGE_BACKOFF).await;
                continue;
            }
        };
        let active = group_instances
            .iter()
            .filter_map(|instance| instance.get("location").and_then(|value| value.as_str()))
            .collect::<HashSet<_>>();
        let log_sessions = discover_log_sessions(
            &scarlet_vrchat::log_tail::resolve_vrc_app_data_dir(),
        );

        for slot in enabled {
            let state = runtime.entry(slot.id.clone()).or_default();
            if state.next_action.is_some_and(|next| Instant::now() < next) {
                continue;
            }
            if let Err(error) = reconcile_slot(
                &client,
                &group_id,
                &store,
                &slot,
                &active,
                &log_sessions,
                state,
            )
            .await
            {
                tracing::warn!(slot = slot.label, profile = slot.profile, %error, "Managed VRChat client reconciliation failed");
                update(&store, &slot.id, None, "Needs attention", error.to_string());
                state.next_action = Some(Instant::now() + ACTION_COOLDOWN);
            }
        }
        let configured = store
            .list()
            .into_iter()
            .map(|slot| slot.id)
            .collect::<HashSet<_>>();
        runtime.retain(|id, _| configured.contains(id));
        tokio::time::sleep(POLL).await;
    }
}

async fn reconcile_slot(
    client: &scarlet_vrchat::client::VrChatClient,
    group_id: &str,
    store: &ManagedClientStore,
    slot: &ManagedClientSlot,
    active: &HashSet<&str>,
    log_sessions: &[ClientLogSession],
    state: &mut SlotRuntime,
) -> anyhow::Result<()> {
    let location_active = !slot.desired_location.is_empty() && active.contains(slot.desired_location.as_str());
    let location = if location_active {
        slot.desired_location.clone()
    } else if slot.auto_recreate_instance {
        let current = client.get_current_user().await?;
        let permission = match slot.template.group_access_type.as_str() {
            "plus" => scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_PLUS_CREATE,
            "members" => scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_OPEN_CREATE,
            _ => scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_PUBLIC_CREATE,
        };
        anyhow::ensure!(
            client.self_has_group_permission(group_id, &current.id, permission).await?,
            "Celeste's VRChat account lacks `{permission}` for automatic replacement"
        );
        if slot.template.age_gate {
            anyhow::ensure!(
                client
                    .self_has_group_permission(
                        group_id,
                        &current.id,
                        scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_AGE_GATED_CREATE,
                    )
                    .await?,
                "Celeste's VRChat account cannot create age-gated group instances"
            );
        }
        update(store, &slot.id, None, "Creating replacement instance", "");
        let instance = client.create_instance(&slot.creation_request(group_id)).await?;
        let location = location_from_instance(&instance, &slot.template.world_id)
            .ok_or_else(|| anyhow::anyhow!("VRChat created an instance but returned no instance ID"))?;
        update(store, &slot.id, Some(location.clone()), "Replacement created", "");
        // Instance lists are eventually consistent. Give the newly created
        // record time to appear so a slow API response cannot create a row
        // of replacement instances.
        state.next_action = Some(Instant::now() + CREATE_COOLDOWN);
        location
    } else {
        let problem = if slot.desired_location.is_empty() {
            "Assign an existing instance or enable automatic replacement"
        } else {
            "The assigned instance is no longer active; select a replacement or enable automatic replacement"
        };
        update(store, &slot.id, None, "Needs instance assignment", problem);
        return Ok(());
    };

    if scarlet_vrchat::launch::profile_is_running(slot.profile) {
        if slot.expected_user_id.trim().is_empty() {
            update(
                store,
                &slot.id,
                None,
                "Running; crash recovery armed",
                "Live location checks are off. Add the optional usr_ account ID to let Celeste verify and move this running profile",
            );
            return Ok(());
        }
        let session = log_sessions.iter().find(|session| {
            session.user_id == slot.expected_user_id
                && session.recently_active
                && !session.clean_quit
        });
        match session.and_then(|session| session.location.as_deref()) {
            Some(current) if current == location => {
                update(store, &slot.id, None, "Running in assigned coverage slot", "");
                return Ok(());
            }
            Some(current) => {
                update(
                    store,
                    &slot.id,
                    None,
                    "Moving to assigned instance",
                    format!("The expected account is currently in {current}"),
                );
                let profile = slot.profile;
                tokio::task::spawn_blocking(move || scarlet_vrchat::launch::stop_profile(profile))
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string()))??;
                state.next_action = Some(Instant::now() + Duration::from_secs(10));
                return Ok(());
            }
            None => {
                update(
                    store,
                    &slot.id,
                    None,
                    "Running; waiting for VRChat log confirmation",
                    "Celeste will not stop an unconfirmed process",
                );
                return Ok(());
            }
        }
    }
    if !slot.auto_restart {
        update(store, &slot.id, None, "Stopped", "Automatic restart is disabled");
        return Ok(());
    }

    let now = Instant::now();
    while state.launches.front().is_some_and(|time| now.duration_since(*time) > RESTART_WINDOW) {
        state.launches.pop_front();
    }
    anyhow::ensure!(
        state.launches.len() < MAX_RESTARTS,
        "restart protection stopped this slot after {MAX_RESTARTS} launches in 15 minutes"
    );
    let (world_id, instance_id) = split_location(&location)
        .ok_or_else(|| anyhow::anyhow!("the assigned instance location is malformed"))?;
    update(store, &slot.id, None, "Obtaining secure join token", "");
    let secure_name = match client.get_instance_secure_name(world_id, instance_id).await {
        Ok(name) => name,
        Err(error) => {
            tracing::warn!(%error, location, "Dedicated secure-name lookup failed; trying the instance response");
            client.get_instance(world_id, instance_id).await.ok().and_then(|instance| {
                instance
                    .secure_name
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| instance.short_name.filter(|name| !name.trim().is_empty()))
            })
        }
    };
    let profile = slot.profile;
    let launch_location = location.clone();
    tokio::task::spawn_blocking(move || {
        scarlet_vrchat::launch::launch_desktop(profile, &launch_location, secure_name.as_deref())
    })
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))??;
    state.launches.push_back(now);
    state.next_action = Some(now + ACTION_COOLDOWN);
    update(store, &slot.id, None, "Launch requested through Steam/EAC", "");
    Ok(())
}

fn update(
    store: &ManagedClientStore,
    id: &str,
    location: Option<String>,
    status: impl Into<String>,
    error: impl Into<String>,
) {
    if let Err(save_error) = store.update_runtime(id, location, status, error) {
        tracing::error!(%save_error, slot = id, "Could not persist managed-client state");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_window_keeps_only_recent_launches() {
        let now = Instant::now();
        let mut state = SlotRuntime::default();
        state.launches.push_back(now - RESTART_WINDOW - Duration::from_secs(1));
        state.launches.push_back(now);
        while state.launches.front().is_some_and(|time| now.duration_since(*time) > RESTART_WINDOW) {
            state.launches.pop_front();
        }
        assert_eq!(state.launches.len(), 1);
    }
}
