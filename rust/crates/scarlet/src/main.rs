//! Rust port of `net.sybyline.scarlet.Main` / `Scarlet.main`.
//!
//! Startup sequence, in order:
//!
//! 1. Resolve the data directory and open the log.
//! 2. Open settings and the shared data objects (`ScarletData`, staff lists,
//!    moderation tags, watched lists) — **once**, so the GUI, the Discord
//!    commands and the event listener all read and write the same in-memory
//!    view of the same files.
//! 3. Start the VRChat log tailer and the ported `ScarletEventListener`,
//!    which is what turns those log lines into instance state, player rows,
//!    advisories and announcements.
//! 4. Either open the window (which also runs the first-run setup wizard) or,
//!    with no display, connect Discord headlessly.
//!
//! Not yet ported from `Scarlet.java`'s own startup: data-dir resolution
//! beyond `SCARLET_HOME`, the executor pool, and restart/update exit-code
//! handling. See ../../PORTING.md.
//!
//! ## Why this isn't `#[tokio::main] async fn main()`
//!
//! `eframe`'s window (via `winit`) needs the process's real main OS thread
//! on some platforms. So this builds the tokio runtime explicitly, spawns
//! the async work onto it, and reserves the main thread for the GUI event
//! loop.

use std::sync::Arc;

use scarlet_core::settings::Settings;

fn main() -> anyhow::Result<()> {
    let data_dir = std::env::var_os("SCARLET_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&data_dir)?;

    let _log_guard = scarlet_core::log::init(&data_dir)?;

    tracing::info!("App: {} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION);
    tracing::info!("Data folder resolved to: {}", data_dir.display());

    let settings = Arc::new(Settings::open(&data_dir));
    if settings.check_has_version_changed_since_last_run(scarlet_core::identity::VERSION) {
        tracing::info!("Version changed since last run (or first run)");
    }
    settings.update_run_version_and_time(scarlet_core::identity::VERSION);

    let ui_state = scarlet_ui::UiState::new();

    let runtime = tokio::runtime::Runtime::new()?;

    // Everything the Discord commands and the event listener must agree on.
    // Published by `connect_and_serve` once the gateway client exists, so
    // the event listener's audit emitter can post without a `Context`.
    let discord_http: scarlet_discord::data::SharedHttp = Arc::new(tokio::sync::RwLock::new(None));
    let discord_config = Arc::new(scarlet_discord::data::DiscordConfigStore::open(&data_dir));

    let shared = scarlet_discord::data::SharedState {
        vrchat_client: Arc::new(tokio::sync::RwLock::new(None)),
        http: discord_http.clone(),
        scarlet_data: Arc::new(scarlet_core::data::ScarletData::open(&data_dir)),
        settings: settings.clone(),
        staff_list: Arc::new(scarlet_core::staff_list::StaffList::open(&data_dir, "staff_list.json")),
        secret_staff_list: Arc::new(scarlet_core::staff_list::StaffList::open(&data_dir, "secret_staff_list.json")),
        moderation_tags: Arc::new(scarlet_core::moderation_tags::ModerationTags::open(&data_dir, "moderation_tags.json")),
        watched_groups: Arc::new(scarlet_core::watched_groups::WatchedGroups::open(&data_dir, "watched_groups.json")),
        watched_users: Arc::new(scarlet_core::watched_entities::WatchedEntities::open(
            &data_dir,
            "watched_users.json",
            scarlet_core::watched_entities::is_valid_user_id,
        )),
        watched_avatars: Arc::new(scarlet_core::watched_entities::WatchedEntities::open(
            &data_dir,
            "watched_avatars.json",
            scarlet_core::watched_entities::is_valid_avatar_id,
        )),
    };

    let tts = Arc::new(scarlet_ui::tts::TtsService::new());
    let pronoun_lists = Arc::new(scarlet_core::pronoun_lists::PronounLists::open(&data_dir));

    // The VRChat log tailer feeds the ported event listener, which drives
    // the instance table. `spawn_tailer` reports and then stops on a machine
    // with no VRChat client, so this costs nothing there.
    let tailer_events = scarlet_vrchat::log_tail::spawn_tailer(&runtime.handle().clone());
    let listener = scarlet_events::ScarletEventListener::new(scarlet_events::listener::ListenerDeps {
        ui: Arc::new(ui_state.clone()),
        vrchat: shared.vrchat_client.clone(),
        settings: settings.clone(),
        data: shared.scarlet_data.clone(),
        staff_list: shared.staff_list.clone(),
        watched_users: shared.watched_users.clone(),
        watched_avatars: shared.watched_avatars.clone(),
        watched_groups: shared.watched_groups.clone(),
        pronoun_lists,
        tts: Arc::new(scarlet_ui::tts::TtsSinkAdapter::new(tts.clone(), settings.clone())),
        emitter: Arc::new(scarlet_discord::events::audit_emitter::DiscordAuditEmitter::new(
            discord_http,
            discord_config,
            shared.scarlet_data.clone(),
            settings.clone(),
        )),
    });
    runtime.spawn(scarlet_events::listener::run(listener, tailer_events));

    if scarlet_ui::platform::is_headless() {
        tracing::info!("No display detected; running headless. Ctrl+C to exit.");
        // Headless can't run the interactive setup wizard (no window to show
        // it in), so — matching a scheduled/unattended Java run — connect
        // with whatever token is already on disk and attempt an unattended
        // VRChat login.
        let discord_token =
            scarlet_core::settings::registry::registry_string_encrypted(settings.encrypted.clone(), "discord.token")
                .get_or_null();
        runtime.spawn({
            let slot = shared.vrchat_client.clone();
            let settings = settings.clone();
            async move {
                if let Some(client) = scarlet_vrchat::client::login_with_stored_credentials(&settings).await {
                    *slot.write().await = Some(Arc::new(client));
                }
            }
        });
        runtime.spawn(scarlet_discord::client::connect_and_serve(discord_token, shared));
        runtime.block_on(std::future::pending::<()>());
    } else {
        // Blocks this (real OS main) thread until the window closes. The
        // setup wizard, if it needs to run, shares this one
        // `eframe::run_native` call; Discord connects once the wizard
        // resolves a token, or immediately if setup already ran.
        if let Err(err) = scarlet_ui::run(ui_state, shared, data_dir, runtime.handle().clone(), tts) {
            tracing::error!("UI error: {}", err);
        }
    }

    Ok(())
}
