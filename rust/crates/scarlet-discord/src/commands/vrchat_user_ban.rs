//! Port of `ScarletDiscordCommands.vrchatUserBan`.
//!
//! **Simplified**: takes a plain `vrchat-user-id` string instead of an
//! autocomplete-resolved `io.github.vrchatapi.model.User` (see the
//! `commands` module docs). Java's "immediate ban with a tag" flow
//! (`tag-immediately`, a Discord select-menu of `ScarletModerationTags`) and
//! its training-simulation branch (`ScarletSimulation.isTrainingId`) aren't
//! ported — neither subsystem exists yet — so this only does the plain ban
//! path. Permission checking and the linked-account requirement ARE real
//! (see [`super::moderation_context`]).

use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

pub const NAME: &str = "vrchat-user-ban";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Bans a VRChat user from the configured group").add_option(
        CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...) to ban")
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
    let target_id = target_id.to_string();

    let context = match super::moderation_context(ctx, command, scarlet_vrchat::client::PERMISSION_GROUP_BANS_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(&target_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| target_id.clone()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {target_id} ({err})")).await;
        }
    };

    match context.client.get_group_member(&context.group_id, &target_id).await {
        Ok(Some(member)) if member.banned_at.is_some() => {
            return super::reply_ephemeral(ctx, command, "This VRChat user is already banned").await;
        }
        Ok(_) => {}
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("Could not check ban status: {err}")).await;
        }
    }

    match context.client.ban_from_group(&context.group_id, &target_id).await {
        Ok(_) => {
            command
                .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(format!("Banned {display_name}")))
                .await
                .map(|_| ())
        }
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to ban {display_name}: {err}")).await,
    }
}
