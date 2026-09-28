//! Port target: the `JDABuilder` setup in `ScarletDiscordJDA`'s constructor
//! (intents, event listener, cache flags, and audio module config). Discord
//! voice uses Songbird 0.6's native DAVE E2EE implementation, replacing the
//! Java fork's JNI-backed `LDaveSessionFactory` integration.

use serenity::client::Client;
use serenity::model::gateway::GatewayIntents;
use songbird::SerenityInit;

use crate::data::SharedState;
use crate::handler::Handler;
use crate::token::normalize_discord_bot_token;

/// Port of the `GatewayIntent.MESSAGE_CONTENT, GatewayIntent.GUILD_MEMBERS`
/// call in the Java constructor, plus the intents serenity itself needs to
/// receive guild/message gateway events at all (JDA enables a baseline set
/// by default that serenity requires you to ask for explicitly).
fn intents() -> GatewayIntents {
    GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::MESSAGE_CONTENT
}

/// Builds a serenity client for the given (already-normalized) bot token.
/// Returns `None` if `raw_token` is empty after normalization, matching the
/// Java version's `if (token0 != null && !token0.isEmpty())` guard around
/// ever constructing a `JDABuilder`.
pub async fn build_client(raw_token: Option<&str>) -> Option<serenity::Result<Client>> {
    let token = normalize_discord_bot_token(raw_token)?;
    Some(Client::builder(&token, intents()).event_handler(Handler).register_songbird().await)
}

async fn build_client_with_voice(
    raw_token: Option<&str>,
    voice: std::sync::Arc<songbird::Songbird>,
) -> Option<serenity::Result<Client>> {
    let token = normalize_discord_bot_token(raw_token)?;
    Some(
        Client::builder(&token, intents())
            .event_handler(Handler)
            .register_songbird_with(voice)
            .await,
    )
}

/// Builds and runs the Discord client to completion (i.e. until it errors
/// or the process shuts down), logging rather than propagating errors —
/// meant to be handed straight to `tokio::spawn`/`Handle::spawn` as a
/// fire-and-forget background task. Port of the "no token -> staff mode"
/// fallback in `ScarletDiscordJDA`'s constructor (`if (token0 != null &&
/// !token0.isEmpty())`); with no token, this just logs and returns.
///
/// Pulled out of `scarlet` (the bin crate) so both its headless path and
/// `scarlet-ui`'s setup wizard (which only knows the token once the user
/// finishes entering it, not at process startup) can share one entry point
/// instead of duplicating the connect-or-log-staff-mode logic.
///
/// `state` is inserted into the serenity client's `TypeMap` before it
/// starts, so every command handler can reach it via `ctx.data.read()`
/// (see `crate::data`'s module docs) — this is what lets a slash command
/// actually call the VRChat API or read/write `ScarletData` instead of only
/// ever answering from Discord-local state.
pub async fn connect_and_serve(token: Option<String>, state: SharedState) {
    match token {
        Some(token) if !token.trim().is_empty() => {
            tracing::info!(
                "Discord bot token configured (fingerprint {}); connecting...",
                crate::token::fingerprint_discord_token(Some(&token))
            );
            match build_client_with_voice(Some(&token), state.voice_manager.clone()).await {
                Some(Ok(mut client)) => {
                    // Publish the HTTP handle before starting, so the event
                    // listener's audit emitter can post as soon as the
                    // gateway is up (it lives outside the client and has no
                    // `Context` of its own).
                    *state.http.write().await = Some(client.http.clone());
                    {
                        let mut data = client.data.write().await;
                        state.insert_into(&mut data);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_token_builds_no_client() {
        assert!(build_client(None).await.is_none());
        assert!(build_client(Some("   ")).await.is_none());
    }
}
