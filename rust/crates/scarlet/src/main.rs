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
//! beyond `SCARLET_HOME`, the executor pool, and updater exit-code handling.
//! Application restart/self-relaunch is ported. See ../../PORTING.md.
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
    // Where Celeste's data lives. This used to fall back to the process's
    // working directory, which is why running the binary from a checkout
    // scattered settings.json, secure-prefs.json, logs/ and backups/ into
    // it — and why a run from a different directory started blank. Java
    // resolves a per-user data directory instead; `scarlet_core::paths`
    // ports that, and adds the "ask on a first run" step.
    let resolved = scarlet_core::paths::resolve_data_dir();
    let headless = scarlet_ui::platform::is_headless();
    let data_dir = match (&resolved.source, headless) {
        (scarlet_core::paths::DataDirSource::FirstRun, false) => {
            scarlet_ui::data_dir_prompt::ask_where_to_store_data(&resolved.path)
        }
        _ => resolved.path.clone(),
    };
    std::fs::create_dir_all(&data_dir)?;

    let _log_guard = scarlet_core::log::init(&data_dir)?;

    tracing::info!("App: {} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION);
    tracing::info!("Data folder resolved to: {} ({:?})", data_dir.display(), resolved.source);
    for other in &resolved.other_candidates {
        // Java prompts when a config exists in more than one known
        // location; at minimum, say so rather than silently preferring one.
        tracing::warn!("Another Celeste config also exists at {} - it is not being used", other.display());
    }

    // Java's `ScarletJsonCache` caches live at `<Scarlet.dir>/caches` and
    // are shared process-wide. Install them before anything builds a VRChat
    // client, or that client fetches every entity every time — which for
    // watched-entity autocomplete meant one API call per watched entity per
    // keystroke, a request rate VRChat suspends accounts over.
    scarlet_vrchat::entity_cache::install(&data_dir);

    let settings = Arc::new(Settings::open(&data_dir));
    if settings.check_has_version_changed_since_last_run(scarlet_core::identity::VERSION) {
        tracing::info!("Version changed since last run (or first run)");
    }
    settings.update_run_version_and_time(scarlet_core::identity::VERSION);

    let ui_state = scarlet_ui::UiState::new();
    ui_state.refresh_vrchat_api_status("VRChat: signing in...");

    let runtime = tokio::runtime::Runtime::new()?;

    // One avatar-search client for the whole process: the Discord command
    // and the player table's avatar status share its cache and back-off.
    let avatar_search = Arc::new(scarlet_discord::avatar_search::AvatarSearch::default());

    // Everything the Discord commands and the event listener must agree on.
    // Published by `connect_and_serve` once the gateway client exists, so
    // the event listener's audit emitter can post without a `Context`.
    let discord_http: scarlet_discord::data::SharedHttp = Arc::new(tokio::sync::RwLock::new(None));
    let discord_config = Arc::new(scarlet_discord::data::DiscordConfigStore::open(&data_dir));
    let voice_manager = songbird::Songbird::serenity();
    let (process_controller, mut process_requests) = scarlet_discord::data::ProcessController::new();

    let shared = scarlet_discord::data::SharedState {
        vrchat_client: Arc::new(tokio::sync::RwLock::new(None)),
        avatar_search: avatar_search.clone(),
        http: discord_http.clone(),
        discord_config: discord_config.clone(),
        voice_manager: voice_manager.clone(),
        process_controller,
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

    // Give Discord enough time to deliver the command acknowledgement, then
    // relaunch this exact executable with its original arguments. This is the
    // Rust equivalent of Java's exit-code-69/self-relaunch path and works in
    // both GUI and headless modes.
    runtime.spawn(async move {
        if let Some(scarlet_discord::data::ProcessRequest::Restart) = process_requests.recv().await {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if let Err(error) = std::fs::write("scarlet.version", scarlet_core::identity::VERSION) {
                tracing::warn!(%error, "Failed to write scarlet.version before restart");
            }
            let relaunched = std::env::current_exe().and_then(|executable| {
                let mut command = std::process::Command::new(executable);
                command.args(std::env::args_os().skip(1));
                command.spawn().map(|_| ())
            });
            match relaunched {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    tracing::error!(%error, "Failed to relaunch Celeste");
                    std::process::exit(69);
                }
            }
        }
    });

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
        tts: Arc::new(scarlet_ui::tts::TtsSinkAdapter::new(
            tts.clone(),
            settings.clone(),
            Some(scarlet_discord::voice_tts::DiscordVoiceTts::new(
                voice_manager,
                shared.discord_config.clone(),
                runtime.handle().clone(),
            )),
        )),
        avatar_search: Arc::new(scarlet_discord::avatar_search::ListenerAvatarSearch::new(
            avatar_search.clone(),
            settings.clone(),
        )),
        emitter: Arc::new(scarlet_discord::events::audit_emitter::DiscordAuditEmitter::new(
            discord_http,
            discord_config,
            shared.scarlet_data.clone(),
            settings.clone(),
        )),
    });
    runtime.spawn(scarlet_events::listener::run(listener, tailer_events));

    // Java's main loop continuously polls the VRChat group audit endpoint.
    // This is separate from the local output-log listener above: it is what
    // creates durable moderation records and Discord moderation threads.
    runtime.spawn(scarlet_discord::events::audit_poller::run(
        scarlet_discord::events::audit_poller::AuditPoller::new(
            shared.vrchat_client.clone(),
            shared.http.clone(),
            shared.discord_config.clone(),
            shared.scarlet_data.clone(),
            settings.clone(),
            shared.secret_staff_list.clone(),
            shared.moderation_tags.clone(),
        ),
    ));

    // Report the VRChat session in the status bar. Whether the session
    // arrives from the setup wizard or from an unattended login differs by
    // launch path, so this watches the shared slot both fill rather than
    // being wired into each of them. Without it a failed login is
    // invisible: the player table still fills from the log, but every
    // profile-derived column (account age, pronouns, rank, 18+) stays blank
    // with nothing saying why.
    runtime.spawn({
        let slot = shared.vrchat_client.clone();
        let ui_state = ui_state.clone();
        async move {
            // How long a login is given before "signing in..." becomes
            // "not signed in", so a slow 2FA prompt doesn't read as failure.
            const GRACE: std::time::Duration = std::time::Duration::from_secs(30);
            let started = std::time::Instant::now();
            let mut signed_in = false;
            loop {
                let client = slot.read().await.clone();
                match (client, signed_in) {
                    (Some(client), false) => {
                        signed_in = true;
                        let status = match client.get_current_user().await {
                            Ok(user) => match user.display_name {
                                Some(name) => format!("VRChat: signed in as {name}"),
                                None => "VRChat: signed in".to_string(),
                            },
                            Err(err) => format!("VRChat: session error ({err})"),
                        };
                        tracing::info!("{status}");
                        ui_state.refresh_vrchat_api_status(status);
                    }
                    (None, true) => {
                        signed_in = false;
                        ui_state.refresh_vrchat_api_status("VRChat: not signed in");
                    }
                    (None, false) if started.elapsed() > GRACE => {
                        ui_state.refresh_vrchat_api_status("VRChat: not signed in");
                    }
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    });

    if headless {
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
