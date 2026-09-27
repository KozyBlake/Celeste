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

    match execute(ctx, &command.user.id.to_string(), target_id, true).await {
        Ok(message) => command
            .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(message))
            .await
            .map(|_| ()),
        Err(message) => super::reply_ephemeral(ctx, command, &message).await,
    }
}

/// The ban/unban action itself, shared by `/vrchat-user-ban`,
/// `/vrchat-user-unban` and the **Ban user**/**Unban user** buttons Scarlet
/// attaches to its instance-event embeds (port of
/// `ScarletDiscordUI._vrchatUserBan`/`_vrchatUserUnban`, which are likewise
/// one implementation behind both entry points).
///
/// Returns the message to show on success, or the reason it didn't happen.
/// Both are Java's own wording.
pub(crate) async fn execute(
    ctx: &Context,
    actor_snowflake: &str,
    target_id: &str,
    ban: bool,
) -> Result<String, String> {
    let context = super::moderation_context_for_snowflake(
        ctx,
        actor_snowflake,
        scarlet_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
    )
    .await?;

    let display_name = match context.client.get_user(target_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| target_id.to_string()),
        Err(err) => return Err(format!("No VRChat user found with id {target_id} ({err})")),
    };

    let currently_banned = match context.client.get_group_member(&context.group_id, target_id).await {
        Ok(Some(member)) => member.banned_at.is_some(),
        Ok(None) => false,
        Err(err) => return Err(format!("Could not check ban status: {err}")),
    };

    if ban {
        if currently_banned {
            return Err("This VRChat user is already banned".to_string());
        }
        match context.client.ban_from_group(&context.group_id, target_id).await {
            Ok(_) => Ok(format!("Banned {display_name}")),
            Err(err) => Err(format!("Failed to ban {display_name}: {err}")),
        }
    } else {
        if !currently_banned {
            return Err("This VRChat user is not banned".to_string());
        }
        match context.client.unban_from_group(&context.group_id, target_id).await {
            Ok(_) => Ok(format!("Unbanned {display_name}")),
            Err(err) => Err(format!("Failed to unban {display_name}: {err}")),
        }
    }
}
