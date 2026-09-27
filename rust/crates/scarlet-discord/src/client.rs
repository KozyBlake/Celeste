//! Port target: the `JDABuilder` setup in `ScarletDiscordJDA`'s constructor
//! (intents, event listener, cache flags, audio module config). DAVE voice
//! E2EE (`LDaveSessionFactory`/`AudioModuleConfig.withDaveSessionFactory`)
//! and voice support generally are not wired up yet — see
//! `scarlet-dave`'s docs and PORTING.md; this only stands up the gateway
//! connection and slash-command plumbing.

use serenity::client::Client;
use serenity::model::gateway::GatewayIntents;

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
        | GatewayIntents::MESSAGE_CONTENT
}

/// Builds a serenity client for the given (already-normalized) bot token.
/// Returns `None` if `raw_token` is empty after normalization, matching the
/// Java version's `if (token0 != null && !token0.isEmpty())` guard around
/// ever constructing a `JDABuilder`.
pub async fn build_client(raw_token: Option<&str>) -> Option<serenity::Result<Client>> {
    let token = normalize_discord_bot_token(raw_token)?;
    Some(Client::builder(&token, intents()).event_handler(Handler).await)
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
