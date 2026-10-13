//! Rust port of `net.sybyline.scarlet.Main` / `Scarlet.main`.
//!
//! Startup sequence, in order:
//!
//! 1. Resolve the data directory and open the log.
//! 2. Open settings and the shared data objects (`CelesteData`, staff lists,
//!    moderation tags, watched lists) — **once**, so the GUI, the Discord
//!    commands and the event listener all read and write the same in-memory
//!    view of the same files.
//! 3. Start the VRChat log tailer and the ported `CelesteEventListener`,
//!    which is what turns those log lines into instance state, player rows,
//!    advisories and announcements.
//! 4. Either open the window (which also runs the first-run setup wizard) or,
//!    with no display, connect Discord headlessly.
//!
//! Celeste's data-directory resolution uses `CELESTE_HOME` while retaining
//! `SCARLET_HOME` as a compatibility alias. Scarlet's update check and its
//! exit-code handling are deliberately dropped: Celeste has no release feed
//! to check against.
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

use celeste_core::settings::Settings;

mod managed_supervisor;

fn main() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => show_startup_failure(&format!("{error:#}")),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "Celeste encountered an unknown panic.".to_string());
            show_startup_failure(&message);
        }
    }
}

fn run() -> anyhow::Result<()> {
    // A splash helper is the same executable on a deliberately tiny code
    // path. Keeping its eframe event loop in a separate process is reliable
    // on both Windows and Linux, where creating a splash event loop and then
    // trying to create the main one in this process is not.
    if let Some(status_file) = std::env::var_os(celeste_ui::splash::STATUS_ENV) {
        celeste_ui::splash::run(std::path::PathBuf::from(status_file))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        return Ok(());
    }

    let headless = celeste_ui::platform::is_headless();
    let mut startup_splash = StartupSplashController::spawn(!headless);
    startup_splash.status("Finding Celeste data...");

    // Where Celeste's data lives. This used to fall back to the process's
    // working directory, which is why running the binary from a checkout
    // scattered settings.json, secure-prefs.json, logs/ and backups/ into
    // it — and why a run from a different directory started blank. Java
    // resolves a per-user data directory instead; `celeste_core::paths`
    // ports that, and adds the "ask on a first run" step.
    let resolved = celeste_core::paths::resolve_data_dir();
    let data_dir = if headless {
        if matches!(resolved.source, celeste_core::paths::DataDirSource::Discovered) {
            // A discovered directory is Java Scarlet. Never open its
            // settings directly; GUI setup can import it additively, while
            // headless Celeste starts clean in its own platform directory.
            celeste_core::paths::default_data_dir()
        } else {
            resolved.path.clone()
        }
    } else {
        celeste_ui::data_dir_prompt::choose_startup_data_dir(&resolved)
    };
    std::fs::create_dir_all(&data_dir)?;

    startup_splash.status("Opening logs and checking imports...");
    let _log_guard = celeste_core::log::init(&data_dir)?;
    install_panic_hook();

    // Existing-install imports are queued from Settings and applied during
    // restart before any store has cached the destination files. Otherwise
    // shutdown saving could overwrite the newly merged records.
    if let Some(result) = celeste_core::legacy_import::run_pending_with_progress(
        &data_dir,
        |fraction, label| startup_splash.progress(fraction, label),
    ) {
        match result {
            Ok(report) => tracing::info!("{}", report.summary()),
            Err(error) => tracing::error!(%error, "Queued Scarlet import failed"),
        }
    }

    tracing::info!("App: {} {}", celeste_core::identity::APP_NAME, celeste_core::identity::VERSION);
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
    celeste_vrchat::entity_cache::install(&data_dir);

    startup_splash.status("Loading settings and caches...");
    let settings = Arc::new(Settings::open(&data_dir));
    let renderer_preference =
        celeste_ui::platform::renderer_preference(&settings.graphics_renderer.get())
            .map_err(anyhow::Error::msg)?;
    let managed_clients = Arc::new(celeste_vrchat::managed_clients::ManagedClientStore::open(&data_dir));
    if settings.check_has_version_changed_since_last_run(celeste_core::identity::VERSION) {
        tracing::info!("Version changed since last run (or first run)");
    }
    settings.update_run_version_and_time(celeste_core::identity::VERSION);

    let ui_state = celeste_ui::UiState::new();
    ui_state.refresh_vrchat_api_status("VRChat: signing in...");

    let runtime = tokio::runtime::Runtime::new()?;
    // Startup login and the health watchdog share one UI-bound 2FA queue.
    // Keeping the receiver alive for the whole window lifetime lets an
    // expired mid-run session request a code instead of requiring a restart.
    let (two_factor_tx, two_factor_rx) = std::sync::mpsc::channel();

    // One avatar-search client for the whole process: the Discord command
    // and the player table's avatar status share its cache and back-off.
    let avatar_search = Arc::new(celeste_discord::avatar_search::AvatarSearch::default());

    // Everything the Discord commands and the event listener must agree on.
    // Published by `connect_and_serve` once the gateway client exists, so
    // the event listener's audit emitter can post without a `Context`.
    let discord_http: celeste_discord::data::SharedHttp = Arc::new(tokio::sync::RwLock::new(None));
    let discord_config = Arc::new(celeste_discord::data::DiscordConfigStore::open(&data_dir));
    let health_reporter = Arc::new(celeste_discord::health::OpsHealthReporter::new(
        discord_http.clone(),
        discord_config.clone(),
    ));
    if let Err(error) = celeste_discord::data::synchronize_guild_id(&settings, &discord_config) {
        tracing::error!(%error, "Failed to synchronize Discord guild configuration");
    }
    let voice_manager = songbird::Songbird::serenity();
    let (process_controller, mut process_requests) = celeste_discord::data::ProcessController::new();
    let lifecycle_process_controller = process_controller.clone();

    startup_splash.status("Loading moderation data...");
    let shared = celeste_discord::data::SharedState {
        vrchat_client: Arc::new(tokio::sync::RwLock::new(None)),
        avatar_search: avatar_search.clone(),
        http: discord_http.clone(),
        discord_config: discord_config.clone(),
        health_reporter: health_reporter.clone(),
        voice_manager: voice_manager.clone(),
        process_controller,
        celeste_data: Arc::new(celeste_core::data::CelesteData::open(&data_dir)),
        settings: settings.clone(),
        staff_list: Arc::new(celeste_core::staff_list::StaffList::open(&data_dir, "staff_list.json")),
        secret_staff_list: Arc::new(celeste_core::staff_list::StaffList::open(&data_dir, "secret_staff_list.json")),
        moderation_tags: Arc::new(celeste_core::moderation_tags::ModerationTags::open(&data_dir, "moderation_tags.json")),
        watched_groups: Arc::new(celeste_core::watched_groups::WatchedGroups::open(&data_dir, "watched_groups.json")),
        watched_users: Arc::new(celeste_core::watched_entities::WatchedEntities::open(
            &data_dir,
            "watched_users.json",
            celeste_core::watched_entities::is_valid_user_id,
        )),
        watched_avatars: Arc::new(celeste_core::watched_entities::WatchedEntities::open(
            &data_dir,
            "watched_avatars.json",
            celeste_core::watched_entities::is_valid_avatar_id,
        )),
    };

    managed_supervisor::spawn(
        runtime.handle(),
        shared.vrchat_client.clone(),
        settings.clone(),
        managed_clients.clone(),
    );

    // Java calls `maybeSaveData` once per main-loop iteration
    // (`Scarlet.java:1841`, `:3467-3472`) and `saveAll` during an orderly
    // shutdown (`:945-947`). Most Rust stores write through immediately,
    // but CelesteData's singleton global.json and live.json deliberately
    // retain Java's dirty-tracked behavior. Without this task, Discord ↔
    // VRChat account links and live-instance routing could remain only in
    // memory for the entire process lifetime and be lost after a crash.
    runtime.spawn({
        let data = shared.celeste_data.clone();
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

    let tts = Arc::new(celeste_ui::tts::TtsService::new());
    let pronoun_lists = Arc::new(celeste_core::pronoun_lists::PronounLists::open(&data_dir));

    // Give Discord enough time to deliver the command acknowledgement, then
    // relaunch this exact executable with its original arguments. This is the
    // Rust equivalent of Java's exit-code-69/self-relaunch path and works in
    // both GUI and headless modes.
    // Headless mode has no graphics event loop to shut down, so it still
    // handles a Discord-requested restart directly. The graphical path is
    // handled after `eframe::run_native` returns below; this avoids exiting
    // from a Tokio worker while EGL/WGL is in the middle of a frame.
    let mut renderer_recovery = None;
    if headless {
        runtime.spawn({
            let data = shared.celeste_data.clone();
            async move {
                if let Some(celeste_discord::data::ProcessRequest::Restart) = process_requests.recv().await {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    data.save_all();
                    match relaunch_current_process() {
                        Ok(()) => std::process::exit(0),
                        Err(error) => {
                            tracing::error!(%error, "Failed to relaunch Celeste");
                            std::process::exit(69);
                        }
                    }
                }
            }
        });
    }

    // The VRChat log tailer feeds the ported event listener, which drives
    // the instance table. `spawn_tailer` reports and then stops on a machine
    // with no VRChat client, so this costs nothing there.
    startup_splash.status("Starting VRChat monitoring...");
    let tailer_events = celeste_vrchat::log_tail::spawn_tailer(&runtime.handle().clone());
    let audit_emitter = Arc::new(celeste_discord::events::audit_emitter::DiscordAuditEmitter::new(
        discord_http,
        discord_config,
        shared.celeste_data.clone(),
        settings.clone(),
        shared.staff_list.clone(),
        shared.watched_users.clone(),
    ));
    let tts_route = Arc::new(celeste_ui::tts::TtsSinkAdapter::new(
        tts.clone(),
        settings.clone(),
        Some(celeste_discord::voice_tts::DiscordVoiceTts::new(
            voice_manager,
            shared.discord_config.clone(),
            settings.clone(),
            runtime.handle().clone(),
        )),
        Some(ui_state.clone()),
    ));
    let listener = celeste_events::CelesteEventListener::new(celeste_events::listener::ListenerDeps {
        ui: Arc::new(ui_state.clone()),
        vrchat: shared.vrchat_client.clone(),
        settings: settings.clone(),
        data: shared.celeste_data.clone(),
        staff_list: shared.staff_list.clone(),
        watched_users: shared.watched_users.clone(),
        watched_avatars: shared.watched_avatars.clone(),
        watched_groups: shared.watched_groups.clone(),
        pronoun_lists: pronoun_lists.clone(),
        tts: tts_route.clone(),
        avatar_search: Arc::new(celeste_discord::avatar_search::ListenerAvatarSearch::new(
            avatar_search.clone(),
            settings.clone(),
        )),
        emitter: audit_emitter.clone(),
    });
    runtime.spawn(celeste_events::listener::run(listener, tailer_events));

    // Java independently reconciles tracked instance monitors every five
    // minutes and enforces the configured world/18+ policy every minute.
    runtime.spawn(celeste_discord::events::instance_lifecycle::run(
        shared.vrchat_client.clone(),
        settings.clone(),
        shared.celeste_data.clone(),
        audit_emitter,
        Arc::new(ui_state.clone()),
    ));
    runtime.spawn(celeste_discord::events::instance_lifecycle::run_fast_follow(
        shared.vrchat_client.clone(),
        settings.clone(),
        Arc::new(ui_state.clone()),
    ));

    // Java's main loop continuously polls the VRChat group audit endpoint.
    // This is separate from the local output-log listener above: it is what
    // creates durable moderation records and Discord moderation threads.
    runtime.spawn(celeste_discord::events::audit_poller::run(
        celeste_discord::events::audit_poller::AuditPoller::new(
            shared.vrchat_client.clone(),
            shared.http.clone(),
            shared.discord_config.clone(),
            shared.celeste_data.clone(),
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
                            .get_object(celeste_vrchat::client::GROUP_ID_KEY);
                        match configured.filter(|id| !id.trim().is_empty()) {
                            Some(group_id) => {
                                let name = client
                                    .get_group_cached(
                                        &group_id,
                                        false,
                                        celeste_vrchat::entity_cache::within_1_day(),
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
    let shutdown_data = shared.celeste_data.clone();
    if headless {
        tracing::info!("No display detected; running headless. Ctrl+C to exit.");
        // Headless can't run the interactive setup wizard (no window to show
        // it in), so — matching a scheduled/unattended Java run — connect
        // with whatever token is already on disk and attempt an unattended
        // VRChat login.
        let discord_token =
            celeste_core::settings::registry::registry_string_encrypted(settings.encrypted.clone(), "discord.token")
                .get_or_null();
        runtime.spawn({
            let slot = shared.vrchat_client.clone();
            let settings = settings.clone();
            async move {
                if let Some(client) = celeste_vrchat::client::login_with_stored_credentials(&settings).await {
                    *slot.write().await = Some(Arc::new(client));
                }
            }
        });
        runtime.spawn(celeste_discord::client::connect_and_serve(discord_token, shared));
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
        startup_splash.finish();
        if let Some(reason) = celeste_ui::platform::renderer_recovery_reason() {
            let message = format!(
                "Celeste could not initialize its automatic graphics renderer (normally DirectX 12 on Windows) on the previous launch.\n\n\
                 Windows reported:\n{reason}\n\n\
                 Celeste is trying again with the OpenGL compatibility renderer. No settings or data were removed. \
                 This is temporary; if Celeste opens normally, select OpenGL under Settings → General → Appearance \
                 to keep using it. You can return to Automatic after updating or changing the graphics driver."
            );
            let _ = celeste_ui::dialogs::info_message("Celeste graphics recovery", message);
        }
        let window_result = celeste_ui::run(
            ui_state,
            shared,
            data_dir,
            runtime.handle().clone(),
            tts,
            tts_route,
            pronoun_lists,
            two_factor_tx,
            two_factor_rx,
            managed_clients,
            &renderer_preference,
        );
        match window_result {
            Ok(()) => {}
            Err(error)
                if cfg!(windows)
                    && renderer_preference == "auto"
                    && !celeste_ui::platform::is_renderer_recovery() =>
            {
                let reason = error.to_string();
                tracing::error!(%reason, "Automatic graphics initialization failed; relaunching once with OpenGL");
                renderer_recovery = Some(reason);
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "the main Celeste window could not open using {}: {error}",
                    celeste_ui::platform::native_renderer_label(&renderer_preference)
                ));
            }
        }
    }

    // Headless mode has no eframe close callback, and this also provides a
    // final safety net if a platform closes the GUI without its explicit
    // confirmation path. The periodic saver limits hard-crash loss; this
    // makes every orderly exit durable.
    shutdown_data.save_all();

    let restart_requested = !headless && lifecycle_process_controller.restart_queued();

    // `Runtime::drop` waits indefinitely for blocking work. Discord voice,
    // networking and platform integrations can leave one of those workers
    // alive even after eframe has closed, which produced the confusing
    // "window is gone but Celeste still owns the terminal" state on Linux.
    // Stop asynchronous services in an orderly but bounded way; after the
    // deadline Tokio releases the main thread so the OS can end any
    // remaining process-local workers with Celeste itself.
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));

    if let Some(reason) = renderer_recovery {
        relaunch_with_renderer_recovery(&reason)?;
    } else if restart_requested {
        relaunch_current_process()?;
    }

    Ok(())
}

fn relaunch_current_process() -> anyhow::Result<()> {
    let executable = std::env::current_exe()?;
    let mut command = std::process::Command::new(executable);
    command.args(std::env::args_os().skip(1));
    command
        .env_remove(celeste_ui::platform::RENDERER_RECOVERY_ENV)
        .env_remove(celeste_ui::platform::RENDERER_RECOVERY_REASON_ENV);
    command.spawn()?;
    Ok(())
}

fn relaunch_with_renderer_recovery(reason: &str) -> anyhow::Result<()> {
    let executable = std::env::current_exe()?;
    let mut command = std::process::Command::new(executable);
    command
        .args(std::env::args_os().skip(1))
        .env(celeste_ui::platform::RENDERER_RECOVERY_ENV, "opengl")
        .env(
            celeste_ui::platform::RENDERER_RECOVERY_REASON_ENV,
            reason.chars().take(2_000).collect::<String>(),
        );
    command.spawn()?;
    Ok(())
}

/// Release builds are Windows GUI applications, so stderr is not visible.
/// Preserve a bootstrap failure outside the normal logger and show it using
/// the same native dialog backend as the data-folder question.
fn show_startup_failure(message: &str) {
    eprintln!("Celeste could not start: {message}");
    let report = format!(
        "Celeste could not start.\n\n{message}\n\n\
         If this is a graphics error, try launching Celeste with `--safe-mode`, or try \
         `--renderer vulkan`. Launchers can set CELESTE_RENDERER=opengl instead. A copy \
         of this error was written to startup-error.txt \
         in Celeste's default data folder."
    );
    let path = celeste_core::paths::default_data_dir().join("startup-error.txt");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let details = format!(
        "{}\nVersion: {}\nOS: {}\nError: {message}\n",
        chrono::Local::now().to_rfc3339(),
        celeste_core::identity::VERSION,
        std::env::consts::OS,
    );
    let _ = std::fs::write(&path, details);
    let _ = celeste_ui::dialogs::error_message("Celeste could not start", report);
}

/// Owns the short-lived companion splash process and its status file. The
/// drop marker also closes the splash if startup returns early with an error
/// or unwinds through this scope.
struct StartupSplashController {
    status_file: Option<std::path::PathBuf>,
    child: Option<std::process::Child>,
    finished: bool,
    progress: std::cell::Cell<f32>,
}

impl StartupSplashController {
    fn spawn(enabled: bool) -> Self {
        if !enabled {
            return Self { status_file: None, child: None, finished: true, progress: std::cell::Cell::new(-1.0) };
        }

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let status_file = std::env::temp_dir().join(format!(
            "celeste-splash-{}-{unique}.status",
            std::process::id()
        ));
        if std::fs::write(&status_file, "Starting Celeste...").is_err() {
            return Self { status_file: None, child: None, finished: true, progress: std::cell::Cell::new(-1.0) };
        }

        let child = std::env::current_exe().ok().and_then(|executable| {
            std::process::Command::new(executable)
                .args(std::env::args_os().skip(1))
                .env(celeste_ui::splash::STATUS_ENV, &status_file)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()
        });
        if child.is_none() {
            let _ = std::fs::remove_file(&status_file);
            return Self { status_file: None, child: None, finished: true, progress: std::cell::Cell::new(-1.0) };
        }

        Self { status_file: Some(status_file), child, finished: false, progress: std::cell::Cell::new(-1.0) }
    }

    fn status(&self, status: &str) {
        if let Some(path) = &self.status_file {
            let progress = self.progress.get();
            let value = if progress >= 0.0 {
                format!("__CELESTE_PROGRESS__\n{progress}\n{status}")
            } else {
                status.to_string()
            };
            let _ = std::fs::write(path, value);
        }
    }

    fn progress(&self, fraction: f32, status: &str) {
        let fraction = fraction.clamp(0.0, 1.0);
        self.progress.set(fraction);
        if let Some(path) = &self.status_file {
            let _ = std::fs::write(
                path,
                format!("__CELESTE_PROGRESS__\n{fraction}\n{status}"),
            );
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        if let Some(path) = &self.status_file {
            let _ = std::fs::write(
                path,
                format!("{}\nCeleste is ready.", celeste_ui::splash::DONE_STATUS),
            );
        }
        self.finished = true;

        // Let the splash complete its short visible handoff before opening
        // the main window. Bound the wait so a broken display backend can
        // never hold Celeste's real startup hostage.
        if let Some(mut child) = self.child.take() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        if let Some(path) = &self.status_file {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for StartupSplashController {
    fn drop(&mut self) {
        self.finish();
    }
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
    slot: celeste_discord::data::SharedVrChatClient,
    settings: Arc<Settings>,
    ui_state: celeste_ui::UiState,
    two_factor_tx: Option<
        std::sync::mpsc::Sender<(Arc<celeste_vrchat::client::VrChatClient>, Vec<String>)>,
    >,
    health_reporter: Arc<celeste_discord::health::OpsHealthReporter>,
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
                        celeste_vrchat::client::save_session(&settings, client);
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
                    match celeste_vrchat::client::login_with_stored_credentials_detailed(&settings).await {
                        celeste_vrchat::client::StoredLogin::LoggedIn(client) => {
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
                        celeste_vrchat::client::StoredLogin::RequiresTwoFactor { client, methods } => {
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
                        celeste_vrchat::client::StoredLogin::Unavailable => {
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
                    let vrchat_logs = celeste_vrchat::log_tail::resolve_vrc_app_data_dir();
                    let eligible = celeste_core::cache_cleanup::collect_eligible(
                        &data_dir,
                        &vrchat_logs,
                        cutoff,
                    );
                    let report = celeste_core::cache_cleanup::delete_files(&eligible);
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
