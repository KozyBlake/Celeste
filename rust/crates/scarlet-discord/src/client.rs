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

const INITIAL_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const STABLE_CONNECTION: std::time::Duration = std::time::Duration::from_secs(60);

fn next_retry_delay(
    current: std::time::Duration,
    connected_for: Option<std::time::Duration>,
) -> std::time::Duration {
    if connected_for.is_some_and(|duration| duration >= STABLE_CONNECTION) {
        INITIAL_RETRY_DELAY
    } else {
        current.saturating_mul(2).min(MAX_RETRY_DELAY)
    }
}

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
    let Some(token) = normalize_discord_bot_token(token.as_deref()) else {
        tracing::warn!(
            "No Discord bot token configured (setting `discord.token`); running without Discord connectivity"
        );
        return;
    };

    let fingerprint = crate::token::fingerprint_discord_token(Some(&token));
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut attempt = 0_u64;

    loop {
        attempt += 1;
        tracing::info!(%fingerprint, attempt, "Discord bot token configured; connecting...");

        let connected_for = match build_client_with_voice(Some(&token), state.voice_manager.clone()).await {
            Some(Ok(mut client)) => {
                // Publish the HTTP handle before starting, so the event
                // listener's audit emitter can post as soon as the gateway
                // is up (it lives outside the client and has no `Context`).
                *state.http.write().await = Some(client.http.clone());
                {
                    let mut data = client.data.write().await;
                    state.insert_into(&mut data);
                }

                let started = std::time::Instant::now();
                match client.start().await {
                    Ok(()) => {
                        tracing::warn!("Discord client stopped; reconnecting");
                        state
                            .health_reporter
                            .incident(
                                "Discord gateway",
                                "The Discord client stopped unexpectedly. Celeste is reconnecting automatically.",
                            )
                            .await;
                    }
                    Err(error) => {
                        tracing::error!(%error, "Discord client stopped with an error; reconnecting");
                        state
                            .health_reporter
                            .incident(
                                "Discord gateway",
                                format!("The Discord client stopped: {error}. Celeste is reconnecting automatically."),
                            )
                            .await;
                    }
                }
                Some(started.elapsed())
            }
            Some(Err(error)) => {
                tracing::error!(%error, "Failed to build Discord client; retrying");
                state
                    .health_reporter
                    .incident(
                        "Discord gateway",
                        format!("Celeste could not establish a Discord client: {error}. It will keep retrying automatically."),
                    )
                    .await;
                None
            }
            // `token` was normalized above, so this is defensive rather than
            // an expected branch.
            None => {
                tracing::error!("Discord token unexpectedly normalized to empty; not connecting");
                return;
            }
        };

        *state.http.write().await = None;
        let scheduled_delay = if connected_for.is_some_and(|duration| duration >= STABLE_CONNECTION)
        {
            INITIAL_RETRY_DELAY
        } else {
            retry_delay
        };
        tracing::warn!(retry_seconds = scheduled_delay.as_secs(), "Discord reconnect scheduled");
        tokio::time::sleep(scheduled_delay).await;
        retry_delay = next_retry_delay(retry_delay, connected_for);
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

    #[test]
    fn retry_backoff_grows_caps_and_resets_after_a_stable_connection() {
        assert_eq!(next_retry_delay(INITIAL_RETRY_DELAY, None), std::time::Duration::from_secs(10));
        assert_eq!(
            next_retry_delay(MAX_RETRY_DELAY, None),
            MAX_RETRY_DELAY
        );
        assert_eq!(
            next_retry_delay(MAX_RETRY_DELAY, Some(STABLE_CONNECTION)),
            INITIAL_RETRY_DELAY
        );
    }
}
