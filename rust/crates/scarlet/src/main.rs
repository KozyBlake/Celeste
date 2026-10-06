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
//! beyond `SCARLET_HOME` and the executor pool. The update check and its
//! exit-code handling are deliberately dropped, not pending: Celeste has no
//! release feed to check against.
//! Application restart/self-relaunch is ported. See ../../PORTING.md.
//!
//! ## Why this isn't `#[tokio::main] async fn main()`
//!
//! `eframe`'s window (via `winit`) needs the process's real main OS thread
//! on some platforms. So this builds the tokio runtime explicitly, spawns
//! the async work onto it, and reserves the main thread for the GUI event
//! loop.

// Release builds are desktop applications on Windows. Debug builds retain a
// console so developers can still see startup failures while diagnosing them.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
    let data_dir = if headless {
        resolved.path.clone()
    } else {
        scarlet_ui::data_dir_prompt::choose_startup_data_dir(&resolved)
    };
    std::fs::create_dir_all(&data_dir)?;

    let _log_guard = scarlet_core::log::init(&data_dir)?;
    install_panic_hook();

    // Existing-install imports are queued from Settings and applied during
    // restart before any store has cached the destination files. Otherwise
    // shutdown saving could overwrite the newly merged records.
    if let Some(result) = scarlet_core::legacy_import::run_pending(&data_dir) {
        match result {
            Ok(report) => tracing::info!("{}", report.summary()),
            Err(error) => tracing::error!(%error, "Queued Scarlet import failed"),
        }
    }

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
    // Startup login and the health watchdog share one UI-bound 2FA queue.
    // Keeping the receiver alive for the whole window lifetime lets an
    // expired mid-run session request a code instead of requiring a restart.
    let (two_factor_tx, two_factor_rx) = std::sync::mpsc::channel();

    // One avatar-search client for the whole process: the Discord command
    // and the player table's avatar status share its cache and back-off.
    let avatar_search = Arc::new(scarlet_discord::avatar_search::AvatarSearch::default());

    // Everything the Discord commands and the event listener must agree on.
    // Published by `connect_and_serve` once the gateway client exists, so
    // the event listener's audit emitter can post without a `Context`.
    let discord_http: scarlet_discord::data::SharedHttp = Arc::new(tokio::sync::RwLock::new(None));
    let discord_config = Arc::new(scarlet_discord::data::DiscordConfigStore::open(&data_dir));
    let health_reporter = Arc::new(scarlet_discord::health::OpsHealthReporter::new(
        discord_http.clone(),
        discord_config.clone(),
    ));
    if let Err(error) = scarlet_discord::data::synchronize_guild_id(&settings, &discord_config) {
        tracing::error!(%error, "Failed to synchronize Discord guild configuration");
    }
    let voice_manager = songbird::Songbird::serenity();
    let (process_controller, mut process_requests) = scarlet_discord::data::ProcessController::new();

    let shared = scarlet_discord::data::SharedState {
        vrchat_client: Arc::new(tokio::sync::RwLock::new(None)),
        avatar_search: avatar_search.clone(),
        http: discord_http.clone(),
        discord_config: discord_config.clone(),
        health_reporter: health_reporter.clone(),
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

    // Java calls `maybeSaveData` once per main-loop iteration
    // (`Scarlet.java:1841`, `:3467-3472`) and `saveAll` during an orderly
    // shutdown (`:945-947`). Most Rust stores write through immediately,
    // but ScarletData's singleton global.json and live.json deliberately
    // retain Java's dirty-tracked behavior. Without this task, Discord ↔
    // VRChat account links and live-instance routing could remain only in
    // memory for the entire process lifetime and be lost after a crash.
    runtime.spawn({
        let data = shared.scarlet_data.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            // `interval`'s first tick completes immediately; the stores are
            // clean at startup, so consume it before entering the loop.
            interval.tick().await;
            loop {
                interval.tick().await;
                data.save_dirty();
            }
        }
    });

    spawn_vrchat_session_watchdog(
        runtime.handle(),
        shared.vrchat_client.clone(),
        settings.clone(),
        ui_state.clone(),
        (!headless).then_some(two_factor_tx.clone()),
        health_reporter,
    );
    spawn_cache_cleanup(runtime.handle(), data_dir.clone(), settings.clone());

    let tts = Arc::new(scarlet_ui::tts::TtsService::new());
    let pronoun_lists = Arc::new(scarlet_core::pronoun_lists::PronounLists::open(&data_dir));

    // Give Discord enough time to deliver the command acknowledgement, then
    // relaunch this exact executable with its original arguments. This is the
    // Rust equivalent of Java's exit-code-69/self-relaunch path and works in
    // both GUI and headless modes.
    runtime.spawn({
        let data = shared.scarlet_data.clone();
        async move {
            if let Some(scarlet_discord::data::ProcessRequest::Restart) = process_requests.recv().await {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                // `process::exit` skips destructors and the GUI close path,
                // so persist the same data Java saves before its restart
                // exit code.
                data.save_all();
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
        }
    });

    // The VRChat log tailer feeds the ported event listener, which drives
    // the instance table. `spawn_tailer` reports and then stops on a machine
    // with no VRChat client, so this costs nothing there.
    let tailer_events = scarlet_vrchat::log_tail::spawn_tailer(&runtime.handle().clone());
    let audit_emitter = Arc::new(scarlet_discord::events::audit_emitter::DiscordAuditEmitter::new(
        discord_http,
        discord_config,
        shared.scarlet_data.clone(),
        settings.clone(),
        shared.staff_list.clone(),
        shared.watched_users.clone(),
    ));
    let tts_route = Arc::new(scarlet_ui::tts::TtsSinkAdapter::new(
        tts.clone(),
        settings.clone(),
        Some(scarlet_discord::voice_tts::DiscordVoiceTts::new(
            voice_manager,
            shared.discord_config.clone(),
            settings.clone(),
            runtime.handle().clone(),
        )),
        Some(ui_state.clone()),
    ));
    let listener = scarlet_events::ScarletEventListener::new(scarlet_events::listener::ListenerDeps {
        ui: Arc::new(ui_state.clone()),
        vrchat: shared.vrchat_client.clone(),
        settings: settings.clone(),
        data: shared.scarlet_data.clone(),
        staff_list: shared.staff_list.clone(),
        watched_users: shared.watched_users.clone(),
        watched_avatars: shared.watched_avatars.clone(),
        watched_groups: shared.watched_groups.clone(),
        pronoun_lists: pronoun_lists.clone(),
        tts: tts_route.clone(),
        avatar_search: Arc::new(scarlet_discord::avatar_search::ListenerAvatarSearch::new(
            avatar_search.clone(),
            settings.clone(),
        )),
        emitter: audit_emitter.clone(),
    });
    runtime.spawn(scarlet_events::listener::run(listener, tailer_events));

    // Java independently reconciles tracked instance monitors every five
    // minutes and enforces the configured world/18+ policy every minute.
    runtime.spawn(scarlet_discord::events::instance_lifecycle::run(
        shared.vrchat_client.clone(),
        settings.clone(),
        shared.scarlet_data.clone(),
        audit_emitter,
        Arc::new(ui_state.clone()),
    ));
    runtime.spawn(scarlet_discord::events::instance_lifecycle::run_fast_follow(
        shared.vrchat_client.clone(),
        settings.clone(),
        Arc::new(ui_state.clone()),
    ));

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
            shared.watched_users.clone(),
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
        let settings = settings.clone();
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

                        // The window title names the group being watched.
                        // Re-read the setting each pass rather than once at
                        // startup, so switching groups retitles the window
                        // without a restart; the lookup goes through the
                        // day-old group cache, so repeating it costs
                        // nothing after the first.
                        let configured: Option<String> = settings
                            .store
                            .get_object(scarlet_vrchat::client::GROUP_ID_KEY);
                        match configured.filter(|id| !id.trim().is_empty()) {
                            Some(group_id) => {
                                let name = client
                                    .get_group_cached(
                                        &group_id,
                                        false,
                                        scarlet_vrchat::entity_cache::within_1_day(),
                                    )
                                    .await
                                    .ok()
                                    .map(|group| group.name)
                                    .filter(|name| !name.trim().is_empty());
                                ui_state.set_group_name(name);
                            }
                            None => ui_state.set_group_name(None),
                        }
                    }
                    (None, true) => {
                        signed_in = false;
                        ui_state.refresh_vrchat_api_status("VRChat: not signed in");
                        ui_state.set_group_name(None);
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

    // Both launch paths move `shared` into their long-lived owner. Retain
    // the process-wide store so returning from either one can still perform
    // Java's final `saveAll`.
    let shutdown_data = shared.scarlet_data.clone();
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
        runtime.block_on(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "Failed to listen for Ctrl+C");
            }
        });
    } else {
        // Blocks this (real OS main) thread until the window closes. The
        // setup wizard, if it needs to run, shares this one
        // `eframe::run_native` call; Discord connects once the wizard
        // resolves a token, or immediately if setup already ran.
        if let Err(err) = scarlet_ui::run(
            ui_state,
            shared,
            data_dir,
            runtime.handle().clone(),
            tts,
            tts_route,
            pronoun_lists,
            two_factor_tx,
            two_factor_rx,
        ) {
            tracing::error!("UI error: {}", err);
        }
    }

    // Headless mode has no eframe close callback, and this also provides a
    // final safety net if a platform closes the GUI without its explicit
    // confirmation path. The periodic saver limits hard-crash loss; this
    // makes every orderly exit durable.
    shutdown_data.save_all();

    Ok(())
}

/// Send panics through the normal tracing subscriber so failures in spawned
/// tasks and worker threads are preserved in Celeste's log file. The default
/// hook only writes to stderr, which is normally invisible for a desktop
/// launch.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = if let Some(message) = info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "non-string panic payload".to_string()
        };
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");

        if let Some(location) = info.location() {
            tracing::error!(
                target: "celeste::panic",
                thread = thread_name,
                file = location.file(),
                line = location.line(),
                column = location.column(),
                panic = %message,
                "thread panicked"
            );
        } else {
            tracing::error!(
                target: "celeste::panic",
                thread = thread_name,
                panic = %message,
                "thread panicked"
            );
        }
    }));
}

/// Java's session half of `healthWatchdogTick` (`Scarlet.java:2612-2719`). A
/// failed network request is deliberately inconclusive; only HTTP 401 proves
/// that the cookie is dead. Recovery reuses the normal
/// saved-cookie/credentials/TOTP path, and hands an interactive code request
/// back to the existing setup prompt when necessary.
///
/// Java's separate 72-hour `maybeRefresh` is intentionally *not* copied: it
/// implements refresh by logging out a healthy session and signing in again.
/// On an account without a stored TOTP secret that guarantees an unsolicited
/// 2FA prompt, even though VRChat's auth cookie is still valid for months.
fn spawn_vrchat_session_watchdog(
    runtime: &tokio::runtime::Handle,
    slot: scarlet_discord::data::SharedVrChatClient,
    settings: Arc<Settings>,
    ui_state: scarlet_ui::UiState,
    two_factor_tx: Option<
        std::sync::mpsc::Sender<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
    >,
    health_reporter: Arc<scarlet_discord::health::OpsHealthReporter>,
) {
    runtime.spawn(async move {
        const INITIAL_DELAY: std::time::Duration = std::time::Duration::from_secs(90);
        const PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
        const RECOVERY_RETRY: std::time::Duration = std::time::Duration::from_secs(10 * 60);

        tokio::time::sleep(INITIAL_DELAY).await;
        let mut session_was_established = false;
        let mut waiting_for_two_factor = false;
        let mut last_recovery_attempt: Option<std::time::Instant> = None;

        loop {
            let current = slot.read().await.clone();
            let mut must_recover = false;

            if let Some(client) = current.as_ref() {
                session_was_established = true;
                waiting_for_two_factor = false;
                match client.get_current_user().await {
                    Ok(_) => {
                        // `/auth/user` may rotate cookies; keep the durable
                        // jar current after every successful probe.
                        scarlet_vrchat::client::save_session(&settings, client);
                        let now = chrono::Utc::now();
                        // Keep the informational timestamp useful without
                        // rewriting registry.json every five minutes.
                        if settings
                            .last_auth_refresh
                            .get_or_null()
                            .is_none_or(|last| {
                                now.signed_duration_since(last) >= chrono::Duration::hours(24)
                            })
                        {
                            settings.last_auth_refresh.set(Some(now));
                        }
                        health_reporter
                            .recovered(
                                "VRChat session",
                                "The moderation account is authenticated again.",
                            )
                            .await;
                    }
                    Err(error) if error.is_unauthorized() => {
                        must_recover = true;
                    }
                    Err(error) => {
                        // Java treats non-401 responses and transport errors
                        // as inconclusive so a network outage never destroys
                        // a usable saved session.
                        tracing::debug!(%error, "VRChat session health probe was inconclusive");
                    }
                }
            } else if session_was_established && !waiting_for_two_factor {
                must_recover = true;
            }

            if must_recover {
                let now = std::time::Instant::now();
                let retry_ready = last_recovery_attempt
                    .map_or(true, |last| now.duration_since(last) >= RECOVERY_RETRY);
                if retry_ready {
                    last_recovery_attempt = Some(now);
                    tracing::warn!("VRChat session is no longer authorized; attempting unattended recovery");
                    health_reporter
                        .incident(
                            "VRChat session",
                            "VRChat rejected Celeste's saved session. Automatic sign-in recovery has started.",
                        )
                        .await;

                    ui_state.refresh_vrchat_api_status("VRChat: session expired; reconnecting...");
                    match scarlet_vrchat::client::login_with_stored_credentials_detailed(&settings).await {
                        scarlet_vrchat::client::StoredLogin::LoggedIn(client) => {
                            *slot.write().await = Some(Arc::new(client));
                            last_recovery_attempt = None;
                            tracing::info!("VRChat session recovery succeeded");
                            ui_state.refresh_vrchat_api_status("VRChat: session recovered");
                            health_reporter
                                .recovered(
                                    "VRChat session",
                                    "The moderation account is authenticated again.",
                                )
                                .await;
                        }
                        scarlet_vrchat::client::StoredLogin::RequiresTwoFactor { client, methods } => {
                            *slot.write().await = None;
                            waiting_for_two_factor = true;
                            ui_state.refresh_vrchat_api_status("VRChat: two-factor code required");
                            if let Some(tx) = two_factor_tx.as_ref() {
                                if tx.send((Arc::new(client), methods)).is_err() {
                                    tracing::warn!("Could not open the VRChat two-factor prompt");
                                }
                            } else {
                                tracing::warn!("VRChat session recovery requires two-factor authentication; restart with the UI to enter a code");
                            }
                        }
                        scarlet_vrchat::client::StoredLogin::Unavailable => {
                            *slot.write().await = None;
                            ui_state.refresh_vrchat_api_status(
                                "VRChat: reconnect failed; retrying in 10 minutes",
                            );
                            tracing::warn!("VRChat unattended session recovery failed; retrying in 10 minutes");
                        }
                    }
                }
            }

            tokio::time::sleep(PROBE_INTERVAL).await;
        }
    });
}

/// Port of `ScarletCacheCleanup.maybeCleanup` and its six-hour schedule.
/// Unlike Java's original implementation, this also includes old VRChat
/// output logs, as explicitly requested for Celeste. The collector always
/// protects the newest Celeste and VRChat log, so a running writer/tailer is
/// never selected.
fn spawn_cache_cleanup(
    runtime: &tokio::runtime::Handle,
    data_dir: std::path::PathBuf,
    settings: Arc<Settings>,
) {
    runtime.spawn(async move {
        const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);
        const MIN_BETWEEN: chrono::Duration = chrono::Duration::hours(1);

        loop {
            if settings.cache_cleanup_enabled.get() {
                let now = chrono::Utc::now();
                let recently_cleaned = settings
                    .last_cache_cleanup
                    .get_or_null()
                    .is_some_and(|last| now.signed_duration_since(last) < MIN_BETWEEN);
                if !recently_cleaned {
                    let days = settings.cache_cleanup_days.get().clamp(1, 30) as u64;
                    let cutoff = std::time::SystemTime::now()
                        - std::time::Duration::from_secs(days * 24 * 60 * 60);
                    let vrchat_logs = scarlet_vrchat::log_tail::resolve_vrc_app_data_dir();
                    let eligible = scarlet_core::cache_cleanup::collect_eligible(
                        &data_dir,
                        &vrchat_logs,
                        cutoff,
                    );
                    let report = scarlet_core::cache_cleanup::delete_files(&eligible);
                    settings.last_cache_cleanup.set(Some(now));
                    tracing::info!(
                        deleted = report.deleted,
                        failed = report.failed,
                        bytes_freed = report.bytes_freed,
                        retention_days = days,
                        "Cache cleanup complete"
                    );
                }
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}
