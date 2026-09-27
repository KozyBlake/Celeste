//! Port of `ScarletDiscordCommands.WatchedUser_` (`@SlashCmd("watched-user")`).
//! See [`super::watched_entity`] for the shared implementation.

use serenity::builder::CreateCommand;
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

use super::watched_entity::EntityKind;

pub const NAME: &str = "watched-user";

pub fn register() -> CreateCommand {
    super::watched_entity::register_generic(NAME, "Configures watched users", EntityKind::User)
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let Some(entities) = ({ ctx.data.read().await.get::<crate::data::WatchedUsersKey>().cloned() }) else {
        super::defer(ctx, command).await?;
        return super::reply_ephemeral(ctx, command, "Watched users are not available.").await;
    };
    super::watched_entity::run_generic(ctx, command, entities, EntityKind::User).await
}
