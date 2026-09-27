//! Port of (part of) `ScarletDiscordCommands.VrchatGroups`
//! (`@SlashCmd("vrchat-group")` — a subcommand-group command). Java's
//! `VrchatGroups` has ~15 subcommands (search-members, list-bans,
//! list-invites, announcements, posts, transfers, open/close-instance,
//! add-role, remove-role); only `add-role`/`remove-role` are ported here —
//! the two most directly useful once ban/unban existed, and the ones that
//! needed no more VRChat API surface than what's already ported. The rest
//! are a tracked gap (see PORTING.md), not silently dropped: registering
//! more subcommands under this same parent command is how they'll be
//! added, matching Java's actual `/vrchat-group <subcommand>` shape rather
//! than inventing separate top-level commands.
//!
//! **Simplified**: `vrchat-user-id`/`vrchat-role-id` are plain string
//! options rather than Java's autocomplete-resolved
//! `io.github.vrchatapi.model.User`/`GroupRole` (see the `commands` module
//! docs). Permission checking and the linked-account requirement are real.

use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

pub const NAME: &str = "vrchat-group";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Manages the configured VRChat group").add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "add-role", "Adds a VRChat Group Role")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...)")
                    .required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-role-id", "The VRChat group role id (grol_...)")
                    .required(true),
            ),
    ).add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "remove-role", "Removes a VRChat Group Role")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...)")
                    .required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-role-id", "The VRChat group role id (grol_...)")
                    .required(true),
            ),
    )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };

    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };

    let user_id = sub_options.iter().find(|o| o.name == "vrchat-user-id").and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(s.to_string()),
        _ => None,
    });
    let role_id = sub_options.iter().find(|o| o.name == "vrchat-role-id").and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(s.to_string()),
        _ => None,
    });
    let (Some(user_id), Some(role_id)) = (user_id, role_id) else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user-id` and `vrchat-role-id` are both required.").await;
    };

    match subcommand.name {
        "add-role" => add_role(ctx, command, &user_id, &role_id).await,
        "remove-role" => remove_role(ctx, command, &user_id, &role_id).await,
        other => super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await,
    }
}

async fn add_role(ctx: &Context, command: &CommandInteraction, user_id: &str, role_id: &str) -> serenity::Result<()> {
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {user_id} ({err})")).await;
        }
    };

    if let Ok(Some(member)) = context.client.get_group_member(&context.group_id, user_id).await {
        if member.role_ids.contains(&role_id.to_string()) {
            return super::reply_ephemeral(ctx, command, &format!("{display_name} already has that role.")).await;
        }
    }

    match context.client.add_group_member_role(&context.group_id, user_id, role_id).await {
        Ok(_) => {
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!("Added the role `{role_id}` to {display_name}"))
                        .ephemeral(true),
                )
                .await
                .map(|_| ())
        }
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to add role: {err}")).await,
    }
}

async fn remove_role(ctx: &Context, command: &CommandInteraction, user_id: &str, role_id: &str) -> serenity::Result<()> {
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {user_id} ({err})")).await;
        }
    };

    if let Ok(Some(member)) = context.client.get_group_member(&context.group_id, user_id).await {
        if !member.role_ids.contains(&role_id.to_string()) {
            return super::reply_ephemeral(ctx, command, &format!("{display_name} already lacks that role.")).await;
        }
    }

    match context.client.remove_group_member_role(&context.group_id, user_id, role_id).await {
        Ok(_) => {
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!("Removed the role `{role_id}` from {display_name}"))
                        .ephemeral(true),
                )
                .await
                .map(|_| ())
        }
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to remove role: {err}")).await,
    }
}
