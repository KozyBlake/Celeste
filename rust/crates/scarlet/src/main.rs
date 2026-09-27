//! Rust port of `net.sybyline.scarlet.Main` / `Scarlet.main`.
//!
//! **Status: headless bootstrap only.** Ported so far: settings load,
//! Discord connect-if-a-token-is-configured (mirroring
//! `ScarletDiscordJDA`'s `if (token0 != null && !token0.isEmpty())` guard),
//! and — when a display is available — a `scarlet-ui` window that opens
//! with `scarlet_ui::setup`'s first-run wizard (vibe-coded notice, Discord
//! bot token/guild, VRChat login) when it hasn't run yet. Not yet ported:
//! the rest of `Scarlet.java`'s startup sequence (data-dir resolution
//! beyond `SCARLET_HOME`, executors, restart/update exit-code handling).
//! See ../../PORTING.md.
//!
//! ## Why this isn't `#[tokio::main] async fn main()`
//!
//! `eframe`'s window (via `winit`) needs to run its event loop on the
//! process's real main OS thread on some platforms (notably macOS) — it
//! can't be handed to a `tokio::task::spawn_blocking` worker thread. So
//! this builds the tokio runtime explicitly, spawns the async work
//! (currently just Discord and the VRChat log tailer) onto it, and reserves
//! the actual main thread for the GUI event loop when one is opened. Not
//! visually verified in this environment (no display server here — see
//! `scarlet_ui::app`'s module docs), so double-check this threading split
//! on a machine with a display before trusting it further.

use std::sync::Arc;

use scarlet_core::settings::Settings;

fn main() -> anyhow::Result<()> {
    let data_dir = std::env::var_os("SCARLET_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&data_dir)?;

    let _log_guard = scarlet_core::log::init(&data_dir)?;

    tracing::info!("App: {} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION);

    let settings = Arc::new(Settings::open(&data_dir));
    if settings.check_has_version_changed_since_last_run(scarlet_core::identity::VERSION) {
        tracing::info!("Version changed since last run (or first run)");
    }
    settings.update_run_version_and_time(scarlet_core::identity::VERSION);

    let ui_state = scarlet_ui::UiState::new();
    ui_state.refresh_vrchat_api_status("not connected (VRChat client not wired up yet)");

    let runtime = tokio::runtime::Runtime::new()?;

    let vrchat_log_events = scarlet_vrchat::log_tail::spawn_tailer(&runtime.handle().clone());
    runtime.spawn(dispatch_vrchat_log_events(vrchat_log_events, ui_state.clone()));

    if scarlet_ui::platform::is_headless() {
        tracing::info!("No display detected; running headless. Ctrl+C to exit.");
        // Headless can't run the interactive setup wizard (no window to
        // show it in), so — matching a scheduled/unattended Java run —
        // just connect with whatever token is already on disk, if any, and
        // attempt an unattended VRChat login (fails if 2FA is required;
        // see login_with_stored_credentials's docs).
        let discord_token =
            scarlet_core::settings::registry::registry_string_encrypted(settings.encrypted.clone(), "discord.token")
                .get_or_null();
        let scarlet_data = Arc::new(scarlet_core::data::ScarletData::open(&data_dir));
        let staff_list = Arc::new(scarlet_core::staff_list::StaffList::open(&data_dir, "staff_list.json"));
        let secret_staff_list = Arc::new(scarlet_core::staff_list::StaffList::open(&data_dir, "secret_staff_list.json"));
        let moderation_tags = Arc::new(scarlet_core::moderation_tags::ModerationTags::open(&data_dir, "moderation_tags.json"));
        let watched_groups = Arc::new(scarlet_core::watched_groups::WatchedGroups::open(&data_dir, "watched_groups.json"));
        let watched_users = Arc::new(scarlet_core::watched_entities::WatchedEntities::open(
            &data_dir,
            "watched_users.json",
            scarlet_core::watched_entities::is_valid_user_id,
        ));
        let watched_avatars = Arc::new(scarlet_core::watched_entities::WatchedEntities::open(
            &data_dir,
            "watched_avatars.json",
            scarlet_core::watched_entities::is_valid_avatar_id,
        ));
        let vrchat_slot: scarlet_discord::data::SharedVrChatClient = Arc::new(tokio::sync::RwLock::new(None));
        runtime.spawn({
            let slot = vrchat_slot.clone();
            let settings = settings.clone();
            async move {
                if let Some(client) = scarlet_vrchat::client::login_with_stored_credentials(&settings).await {
                    *slot.write().await = Some(Arc::new(client));
                }
            }
        });
        runtime.spawn(scarlet_discord::client::connect_and_serve(
            discord_token,
            scarlet_discord::data::SharedState {
                vrchat_client: vrchat_slot,
                scarlet_data,
                settings,
                staff_list,
                secret_staff_list,
                moderation_tags,
                watched_groups,
                watched_users,
                watched_avatars,
            },
        ));
        runtime.block_on(std::future::pending::<()>());
    } else {
        // Blocks this (real OS main) thread until the window closes. The
        // setup wizard (if it needs to run) and the main window share this
        // one `eframe::run_native` call; Discord only connects once the
        // wizard resolves a token (or immediately, if setup already ran on
        // a previous launch) — see `scarlet_ui::setup`.
        if let Err(err) = scarlet_ui::run(ui_state, Some(settings), data_dir, runtime.handle().clone()) {
            tracing::error!("UI error: {}", err);
        }
    }

    Ok(())
}

/// Drains parsed VRChat local-log events (see `scarlet_vrchat::log_tail`)
/// and translates them into real `UiState` player join/update/leave calls,
/// which is what actually makes the Instance tab's table populate. Runs
/// until the sender side (the tailer task) is dropped, which only happens
/// if the whole process is shutting down.
async fn dispatch_vrchat_log_events(
    mut events: tokio::sync::mpsc::UnboundedReceiver<scarlet_vrchat::log_tail::LogEvent>,
    state: scarlet_ui::UiState,
) {
    use scarlet_vrchat::log_tail::LogEventKind;

    while let Some(event) = events.recv().await {
        let scarlet_vrchat::log_tail::LogEvent { preamble, timestamp, kind } = event;
        match kind {
            LogEventKind::UserAuthenticated { display_name, user_id } => {
                state.append_cli_output(format!("Authenticated as {display_name} ({user_id})"));
            }
            LogEventKind::UserJoined { location } => {
                // Matches the game's own log semantics: a room join always
                // starts a fresh instance (an `OnLeftRoom` always precedes
                // the next `Joining`), so this is the right moment to clear
                // out the previous instance's roster rather than mixing
                // rosters across instances.
                state.clear_instance();
                state.append_cli_output(format!("Joined instance: {location}"));
            }
            LogEventKind::UserLeft => {
                state.append_cli_output("Left instance");
            }
            LogEventKind::UserQuit { lifetime_seconds } => {
                state.append_cli_output(format!("VRChat closed after {lifetime_seconds:.0}s this session"));
            }
            LogEventKind::PlayerJoined { display_name, user_id } => {
                state.player_join(preamble, scarlet_ui::ConnectedPlayer::new(user_id, display_name, timestamp));
            }
            LogEventKind::PlayerLeft { display_name, user_id } => {
                let _ = display_name;
                state.player_leave(preamble, &user_id, timestamp);
            }
            LogEventKind::PlayerSwitchAvatar { display_name, avatar_display_name } => {
                state.player_update_by_name(&display_name, |p| p.avatar_name = Some(avatar_display_name));
            }
            LogEventKind::VtkInit { target_display_name, actor_display_name } => {
                state.append_cli_output(match actor_display_name {
                    Some(actor) => format!("Vote kick initiated against {target_display_name} by {actor}"),
                    None => format!("Vote kick initiated against {target_display_name}"),
                });
            }
            LogEventKind::PlayerSpawnPedestal { display_name, user_id, content_type, content_id } => {
                state.append_cli_output(format!(
                    "{display_name} ({user_id}) created a {content_type} pedestal: {content_id}"
                ));
            }
            LogEventKind::PlayerSpawnSticker { display_name, user_id, sticker_id } => {
                state.append_cli_output(format!("{display_name} ({user_id}) spawned sticker {sticker_id}"));
            }
            LogEventKind::PlayerSpawnProp { user_id, prop_id } => {
                state.append_cli_output(format!("{user_id} spawned prop {prop_id}"));
            }
            LogEventKind::ApiRequest { index, method, url } => {
                state.append_cli_output(format!("[API #{index}] {method} {url}"));
            }
            LogEventKind::VideoLoad { display_name, url, title } => {
                let _ = title;
                state.append_cli_output(format!("{display_name} loaded video: {url}"));
            }
        }
    }
}
