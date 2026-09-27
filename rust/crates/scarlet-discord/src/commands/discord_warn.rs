//! Port of `ScarletDiscordCommands.discordWarn` (`@SlashCmd("discord-warn")`)
//! — DMs a Discord server member a warning with an optional reason. Gated
//! at the Discord level via `default_member_permissions(MODERATE_MEMBERS)`
//! (`@DefaultPerms(Permission.MODERATE_MEMBERS)` in Java).
//!
//! **Simplified**: Java re-checks `actorMember.hasPermission(MODERATE_MEMBERS)`
//! in-code in addition to the Discord-level default -- redundant under normal
//! server configuration, so this relies on `default_member_permissions`
//! alone (same as `unlink-vrchat-account-for`). The "cannot warn someone
//! whose highest role is equal to or above yours" hierarchy check
//! (`actorMember.canInteract(discordUser)`) is **not ported**: comparing
//! role hierarchy needs both members' full role lists plus the guild's
//! role-position ordering, and this crate has no shared helper for that yet
//! (it'll be needed again for `discord-kick`/`discord-ban`'s confirmation
//! flow once component-interaction dispatch exists, so it's worth building
//! once properly rather than half here). The "cannot warn the server
//! owner" check *is* ported (a single guild-owner-id comparison, cheap and
//! self-contained). `emitDiscordActionLog` (writing to a configured
//! Discord action-log channel) is **not ported** either -- that channel
//! config isn't wired up in this crate (see `set-discord-action-log-channel`
//! in PORTING.md's gap list), so this command has no side channel to log
//! to; the DM itself is still the real action.

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
        .add_option(CreateCommandOption::new(CommandOptionType::User, "discord-user", "The member to warn").required(true))
        .add_option(CreateCommandOption::new(CommandOptionType::String, "reason", "Reason shown to the member"))
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
            "The built-in Discord moderation commands are not enabled.\nEnable them under **Settings -> Discord** in Scarlet.",
        )
        .await;
    }

    let options = command.data.options();
    let target_user = options.iter().find(|o| o.name == "discord-user").and_then(|o| match &o.value {
        ResolvedValue::User(user, _) => Some(*user),
        _ => None,
    });
    let Some(target_user) = target_user else {
        return super::reply_ephemeral(ctx, command, "Please select a valid server member.").await;
    };
    let reason = options.iter().find(|o| o.name == "reason").and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(*s),
        _ => None,
    });

    let Some(guild_id) = command.guild_id else {
        return super::reply_ephemeral(ctx, command, "Could not find the configured Discord server.").await;
    };
    let guild = match ctx.http.get_guild(guild_id).await {
        Ok(guild) => guild,
        Err(_) => return super::reply_ephemeral(ctx, command, "Could not find the configured Discord server.").await,
    };

    if guild.owner_id == target_user.id {
        return super::reply_ephemeral(ctx, command, &format!("Cannot warn **{}** - they are the server owner.", target_user.name))
            .await;
    }

    let actor_name = command.member.as_ref().and_then(|m| m.nick.clone()).unwrap_or_else(|| command.user.name.clone());
    let reason_text = reason.map(str::trim).filter(|r| !r.is_empty()).unwrap_or("No reason provided");
    let dm_message =
        format!("You have received a warning in **{}** from {actor_name}.\n**Reason:** {reason_text}", guild.name);

    match target_user.direct_message(ctx, CreateMessage::new().content(dm_message)).await {
        Ok(_) => super::reply_ephemeral(ctx, command, &format!("Warned **{}**.", target_user.name)).await,
        Err(err) => {
            super::reply_ephemeral(ctx, command, &format!("Could not DM **{}**; warning was not delivered: {err}", target_user.name))
                .await
        }
    }
}
