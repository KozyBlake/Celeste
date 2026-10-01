//! Port of `ScarletDiscordCommands.discordWarn` (`@SlashCmd("discord-warn")`)
//! — DMs a Discord server member a warning with an optional reason. Gated
//! at the Discord level via `default_member_permissions(MODERATE_MEMBERS)`
//! (`@DefaultPerms(Permission.MODERATE_MEMBERS)` in Java).
//!
//! Includes Java's explicit actor permission and role-hierarchy checks,
//! owner protection, and configured Discord action-log output.

use serenity::builder::{CreateCommand, CreateCommandOption, CreateMessage};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::Permissions;

use crate::data::SettingsKey;

pub const NAME: &str = "discord-warn";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Warn a Discord server member")
        .default_member_permissions(Permissions::MODERATE_MEMBERS)
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::User,
                "discord-user",
                "The member to warn",
            )
            .required(true),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::String,
            "reason",
            "Reason shown to the member",
        ))
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let Some(settings) = ({ ctx.data.read().await.get::<SettingsKey>().cloned() }) else {
        return super::reply_ephemeral(ctx, command, "Settings are not available.").await;
    };
    if !settings.discord_kick_ban_enabled.get() {
        return super::reply_ephemeral(
            ctx,
            command,
            "The built-in Discord moderation commands are not enabled.\nEnable them under **Settings -> Discord** in Celeste.",
        )
        .await;
    }

    let options = command.data.options();
    let target = options
        .iter()
        .find(|o| o.name == "discord-user")
        .and_then(|o| match &o.value {
            ResolvedValue::User(user, Some(member)) => Some((*user, *member)),
            _ => None,
        });
    let Some((target_user, target_member)) = target else {
        return super::reply_ephemeral(ctx, command, "Please select a valid server member.").await;
    };
    let reason = options
        .iter()
        .find(|o| o.name == "reason")
        .and_then(|o| match &o.value {
            ResolvedValue::String(s) => Some(*s),
            _ => None,
        });

    let Some(guild_id) = command.guild_id else {
        return super::reply_ephemeral(
            ctx,
            command,
            "Could not find the configured Discord server.",
        )
        .await;
    };
    let guild = match ctx.http.get_guild(guild_id).await {
        Ok(guild) => guild,
        Err(_) => {
            return super::reply_ephemeral(
                ctx,
                command,
                "Could not find the configured Discord server.",
            )
            .await
        }
    };

    if guild.owner_id == target_user.id {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "Cannot warn **{}** - they are the server owner.",
                target_user.name
            ),
        )
        .await;
    }

    let Some(actor_member) = command.member.as_ref() else {
        return super::reply_ephemeral(ctx, command, "Could not determine your server membership.")
            .await;
    };
    if !actor_member
        .permissions
        .is_some_and(|permissions| permissions.contains(Permissions::MODERATE_MEMBERS))
    {
        return super::reply_ephemeral(ctx, command, "You do not have permission to warn members.")
            .await;
    }
    let roles = match guild_id.roles(&ctx.http).await {
        Ok(roles) => roles,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not check Discord role hierarchy: {err}"),
            )
            .await
        }
    };
    if !super::discord_member_action::can_interact(
        &roles,
        &actor_member.roles,
        &target_member.roles,
        guild.owner_id == command.user.id,
    ) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "Cannot warn **{}** - their highest role is equal to or above yours.",
                target_user.name
            ),
        )
        .await;
    }

    let actor_name = actor_member
        .nick
        .clone()
        .unwrap_or_else(|| command.user.name.clone());
    let target_name = target_member
        .nick
        .clone()
        .unwrap_or_else(|| target_user.name.clone());
    let reason_text = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("No reason provided");
    let dm_message = format!(
        "You have received a warning in **{}** from {actor_name}.\n**Reason:** {reason_text}",
        guild.name
    );

    match target_user
        .direct_message(ctx, CreateMessage::new().content(dm_message))
        .await
    {
        Ok(_) => {
            super::discord_member_action::emit_action_log(
                ctx,
                "Discord Warning",
                Some(actor_member),
                &target_name,
                target_user.id,
                reason,
                true,
                "Warning DM delivered.",
                super::discord_member_action::WARNING_COLOUR,
            )
            .await;
            super::reply_ephemeral(ctx, command, &format!("Warned **{target_name}**.")).await
        }
        Err(err) => {
            super::discord_member_action::emit_action_log(
                ctx,
                "Discord Warning",
                Some(actor_member),
                &target_name,
                target_user.id,
                reason,
                false,
                &format!("Failed to deliver warning DM: {err}"),
                super::discord_member_action::FAILURE_COLOUR,
            )
            .await;
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not DM **{target_name}**; warning was not delivered: {err}"),
            )
            .await
        }
    }
}
