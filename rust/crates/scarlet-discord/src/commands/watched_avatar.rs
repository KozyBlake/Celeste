//! Port of `ScarletDiscordCommands.WatchedAvatar_` (`@SlashCmd("watched-avatar")`).
//! See [`super::watched_entity`] for the shared implementation.
//! Registration is gated by Java's `watched.avatars.enabled` feature flag.

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
