//! Rust port of `net.sybyline.scarlet.Main` / `Scarlet.main`.
//!
//! **Status: headless bootstrap only.** Ported so far: settings load,
//! Discord connect-if-a-token-is-configured (mirroring
//! `ScarletDiscordJDA`'s `if (token0 != null && !token0.isEmpty())` guard).
//! Not yet ported: the rest of `Scarlet.java`'s startup sequence (data-dir
//! resolution beyond `SCARLET_HOME`, VRChat client, executors, the
//! Swing/scarlet-ui splash and main window, restart/update exit-code
//! handling). See ../../PORTING.md.

use scarlet_core::settings::{registry, Settings};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let discord_token = registry::registry_string_encrypted(settings.encrypted.clone(), "discord.token");
    match discord_token.get_or_null() {
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

    Ok(())
}
