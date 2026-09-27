//! Rust port of `net.sybyline.scarlet.Main` / `Scarlet.main`.
//!
//! **Status: headless bootstrap only.** Ported so far: settings load,
//! Discord connect-if-a-token-is-configured (mirroring
//! `ScarletDiscordJDA`'s `if (token0 != null && !token0.isEmpty())` guard),
//! and — when a display is available — a minimal `scarlet-ui` window. Not
//! yet ported: the rest of `Scarlet.java`'s startup sequence (data-dir
//! resolution beyond `SCARLET_HOME`, VRChat client, executors, restart/
//! update exit-code handling). See ../../PORTING.md.
//!
//! ## Why this isn't `#[tokio::main] async fn main()`
//!
//! `eframe`'s window (via `winit`) needs to run its event loop on the
//! process's real main OS thread on some platforms (notably macOS) — it
//! can't be handed to a `tokio::task::spawn_blocking` worker thread. So
//! this builds the tokio runtime explicitly, spawns the async work
//! (currently just Discord) onto it, and reserves the actual main thread
//! for the GUI event loop when one is opened. Not visually verified in this
//! environment (no display server here — see `scarlet_ui::app`'s module
//! docs), so double-check this threading split on a machine with a display
//! before trusting it further.

use scarlet_core::settings::{registry, Settings};

fn main() -> anyhow::Result<()> {
    let data_dir = std::env::var_os("SCARLET_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&data_dir)?;

    let _log_guard = scarlet_core::log::init(&data_dir)?;

    tracing::info!("App: {} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION);

    let settings = Settings::open(&data_dir);
    if settings.check_has_version_changed_since_last_run(scarlet_core::identity::VERSION) {
        tracing::info!("Version changed since last run (or first run)");
    }
    settings.update_run_version_and_time(scarlet_core::identity::VERSION);

    let ui_state = scarlet_ui::UiState::new();
    ui_state.refresh_vrchat_api_status("not connected (VRChat client not wired up yet)");

    let runtime = tokio::runtime::Runtime::new()?;
    let discord_token = registry::registry_string_encrypted(settings.encrypted.clone(), "discord.token").get_or_null();
    runtime.spawn(run_discord(discord_token));

    if scarlet_ui::platform::is_headless() {
        tracing::info!("No display detected; running headless. Ctrl+C to exit.");
        runtime.block_on(std::future::pending::<()>());
    } else {
        // Blocks this (real OS main) thread until the window closes; Discord
        // keeps running on the tokio runtime's worker threads in the background.
        if let Err(err) = scarlet_ui::run(ui_state) {
            tracing::error!("UI error: {}", err);
        }
    }

    Ok(())
}

async fn run_discord(token: Option<String>) {
    match token {
        Some(token) if !token.trim().is_empty() => {
            tracing::info!(
                "Discord bot token configured (fingerprint {}); connecting...",
                scarlet_discord::token::fingerprint_discord_token(Some(&token))
            );
            match scarlet_discord::client::build_client(Some(&token)).await {
                Some(Ok(mut client)) => {
                    if let Err(err) = client.start().await {
                        tracing::error!("Discord client error: {}", err);
                    }
                }
                Some(Err(err)) => tracing::error!("Failed to build Discord client: {}", err),
                None => tracing::warn!("Discord token was configured but normalized to empty; not connecting"),
            }
        }
        _ => {
            tracing::warn!(
                "No Discord bot token configured (setting `discord.token`); running without Discord connectivity"
            );
        }
    }
}
