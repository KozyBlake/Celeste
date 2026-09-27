//! Port of `ScarletDiscordCommands.WatchedAvatar_` (`@SlashCmd("watched-avatar")`).
//! See [`super::watched_entity`] for the shared implementation.
//!
//! **Not ported**: Java gates this whole command behind
//! `@FeatureGate("watched.avatars.enabled")` (registered only when that
//! feature flag is on). No command-registration-time feature gating exists
//! in this crate yet (every ported command registers unconditionally), so
//! this always registers -- a minor parity gap, not a functional one.

use serenity::builder::CreateCommand;
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

use super::watched_entity::EntityKind;

pub const NAME: &str = "watched-avatar";

pub fn register() -> CreateCommand {
    super::watched_entity::register_generic(NAME, "Configures watched avatars", EntityKind::Avatar)
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let Some(entities) = ({ ctx.data.read().await.get::<crate::data::WatchedAvatarsKey>().cloned() }) else {
        super::defer(ctx, command).await?;
        return super::reply_ephemeral(ctx, command, "Watched avatars are not available.").await;
    };
    super::watched_entity::run_generic(ctx, command, entities, EntityKind::Avatar).await
}
