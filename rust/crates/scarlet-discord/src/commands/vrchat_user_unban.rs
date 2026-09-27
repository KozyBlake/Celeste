//! Port of `ScarletDiscordCommands.vrchatUserUnban`. Same simplifications
//! as [`super::vrchat_user_ban`] (plain string id option, no training-
//! simulation branch); permission checking and the linked-account
//! requirement are real.

use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

pub const NAME: &str = "vrchat-user-unban";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Unbans a VRChat user from the configured group").add_option(
        CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...) to unban")
            .required(true),
    )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let resolved = options.iter().find(|o| o.name == "vrchat-user-id").map(|o| o.value.clone());
    let Some(ResolvedValue::String(target_id)) = resolved else {
        return super::reply_ephemeral(ctx, command, "A `vrchat-user-id` option is required.").await;
    };

    // Same implementation as the ban path, with `ban: false` — see
    // `vrchat_user_ban::execute`.
    match super::vrchat_user_ban::execute(ctx, &command.user.id.to_string(), target_id, false).await {
        Ok(message) => command
            .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(message))
            .await
            .map(|_| ()),
        Err(message) => super::reply_ephemeral(ctx, command, &message).await,
    }
}
