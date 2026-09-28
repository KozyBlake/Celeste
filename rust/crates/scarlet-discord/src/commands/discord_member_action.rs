//! Confirmation-gated `/discord-ban` and `/discord-kick` flows.

use std::collections::HashMap;

use serenity::builder::{
    CreateActionRow, CreateButton, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateInteractionResponseFollowup, CreateMessage,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::guild::{Member, Role};
use serenity::model::id::{GuildId, RoleId, UserId};
use serenity::model::permissions::Permissions;
use serenity::model::prelude::ButtonStyle;

use crate::data::{DiscordConfigKey, DiscordModerationDraftsKey, SettingsKey};

pub const BAN_NAME: &str = "discord-ban";
pub const KICK_NAME: &str = "discord-kick";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberAction {
    Ban,
    Kick,
}
impl MemberAction {
    fn name(self) -> &'static str {
        if self == Self::Ban {
            BAN_NAME
        } else {
            KICK_NAME
        }
    }
    fn verb(self) -> &'static str {
        if self == Self::Ban {
            "ban"
        } else {
            "kick"
        }
    }
    pub(crate) fn title(self) -> &'static str {
        if self == Self::Ban {
            "Ban"
        } else {
            "Kick"
        }
    }
    fn past(self) -> &'static str {
        if self == Self::Ban {
            "banned"
        } else {
            "kicked"
        }
    }
    pub(crate) fn permission(self) -> Permissions {
        if self == Self::Ban {
            Permissions::BAN_MEMBERS
        } else {
            Permissions::KICK_MEMBERS
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscordModerationDraft {
    pub action: MemberAction,
    pub owner_id: UserId,
    pub guild_id: GuildId,
    pub target_id: UserId,
    pub target_name: String,
    pub reason: Option<String>,
}

pub fn register(action: MemberAction) -> CreateCommand {
    CreateCommand::new(action.name())
        .description(format!("{} a Discord server member", action.title()))
        .default_member_permissions(action.permission())
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::User,
                "discord-user",
                format!("The member to {}", action.verb()),
            )
            .required(true),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::String,
            "reason",
            "Reason shown to the member",
        ))
}

fn highest_role<'a>(roles: &'a HashMap<RoleId, Role>, member_roles: &[RoleId]) -> Option<&'a Role> {
    member_roles.iter().filter_map(|id| roles.get(id)).max()
}

pub(crate) fn can_interact(
    roles: &HashMap<RoleId, Role>,
    actor_roles: &[RoleId],
    target_roles: &[RoleId],
    actor_is_owner: bool,
) -> bool {
    actor_is_owner
        || match (
            highest_role(roles, actor_roles),
            highest_role(roles, target_roles),
        ) {
            (Some(actor), Some(target)) => actor > target,
            (Some(_), None) => true,
            _ => false,
        }
}

pub(crate) async fn bot_can_interact(
    ctx: &Context,
    guild_id: GuildId,
    target_roles: &[RoleId],
) -> serenity::Result<bool> {
    let bot_id = ctx.cache.current_user().id;
    let bot = guild_id.member(ctx, bot_id).await?;
    let roles = guild_id.roles(&ctx.http).await?;
    Ok(can_interact(&roles, &bot.roles, target_roles, false))
}

pub(crate) async fn emit_action_log(
    ctx: &Context,
    title: &str,
    actor: &str,
    target_name: &str,
    target_id: UserId,
    reason: &str,
    success: bool,
    detail: &str,
) {
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(channel_id) = config
        .and_then(|config| config.json("discordActionLogChannelSf"))
        .and_then(|value| value.as_str().and_then(|id| id.parse::<u64>().ok()))
    else {
        return;
    };
    let success_colour = if title == "Discord Ban" {
        0xED4245
    } else {
        0xFEE75C
    };
    let embed = CreateEmbed::new()
        .title(title)
        .colour(if success { success_colour } else { 0xED4245 })
        .field("Actor", actor, true)
        .field("Target", format!("{target_name} (`{target_id}`)"), true)
        .field("Reason", reason, false)
        .field("Result", detail, false);
    if let Err(err) = serenity::model::id::ChannelId::new(channel_id)
        .send_message(&ctx.http, CreateMessage::new().embed(embed))
        .await
    {
        tracing::warn!("Could not emit Discord action log: {err}");
    }
}

pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    action: MemberAction,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    if !settings.is_some_and(|settings| settings.discord_kick_ban_enabled.get()) {
        return super::reply_ephemeral(ctx, command, "The built-in Discord moderation commands are not enabled.\nEnable them under **Settings -> Discord** in Scarlet.").await;
    }
    let options = command.data.options();
    let target = options
        .iter()
        .find(|option| option.name == "discord-user")
        .and_then(|option| match &option.value {
            ResolvedValue::User(user, Some(member)) => Some((*user, *member)),
            _ => None,
        });
    let Some((target_user, target_member)) = target else {
        return super::reply_ephemeral(ctx, command, "Please select a valid server member.").await;
    };
    let reason = options
        .iter()
        .find(|option| option.name == "reason")
        .and_then(|option| match &option.value {
            ResolvedValue::String(value) => Some(value.trim().to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty());
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
                "Cannot {} **{}** — they are the server owner.",
                action.verb(),
                target_user.name
            ),
        )
        .await;
    }
    if !command
        .app_permissions
        .is_some_and(|permissions| permissions.contains(action.permission()))
    {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "The bot does not have the **{} Members** permission in this server.",
                action.title()
            ),
        )
        .await;
    }
    match bot_can_interact(ctx, guild_id, &target_member.roles).await {
        Ok(true) => {}
        Ok(false) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!(
                    "Cannot {} **{}** — their highest role is equal to or above mine.",
                    action.verb(),
                    target_user.name
                ),
            )
            .await
        }
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not check Discord role hierarchy: {err}"),
            )
            .await
        }
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    let drafts = {
        ctx.data
            .read()
            .await
            .get::<DiscordModerationDraftsKey>()
            .cloned()
    };
    let Some(drafts) = drafts else {
        return super::reply_ephemeral(
            ctx,
            command,
            "Discord moderation confirmation state is unavailable.",
        )
        .await;
    };
    let target_name = target_member
        .nick
        .clone()
        .unwrap_or_else(|| target_user.name.clone());
    drafts.write().await.insert(
        token.clone(),
        DiscordModerationDraft {
            action,
            owner_id: command.user.id,
            guild_id,
            target_id: target_user.id,
            target_name: target_name.clone(),
            reason: reason.clone(),
        },
    );
    let reason_line = reason
        .as_deref()
        .map(|reason| format!("\n**Reason:** {reason}"))
        .unwrap_or_default();
    command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(format!(
                    "{} **{}** (`{}`) from the server?{reason_line}",
                    action.title(),
                    target_name,
                    target_user.name
                ))
                .components(vec![CreateActionRow::Buttons(vec![
                    CreateButton::new(format!("discord-member-confirm:{token}"))
                        .label(action.title())
                        .style(ButtonStyle::Danger),
                    CreateButton::new(format!("discord-member-cancel:{token}"))
                        .label("Cancel")
                        .style(ButtonStyle::Secondary),
                ])])
                .ephemeral(true),
        )
        .await
        .map(|_| ())
}

pub(crate) async fn perform(
    ctx: &Context,
    draft: DiscordModerationDraft,
    actor: &Member,
) -> Result<String, String> {
    if !actor
        .permissions
        .is_some_and(|permissions| permissions.contains(draft.action.permission()))
    {
        return Err(format!(
            "You do not have permission to {} members.",
            draft.action.verb()
        ));
    }
    let guild = ctx
        .http
        .get_guild(draft.guild_id)
        .await
        .map_err(|_| "Could not find the configured Discord server.".to_string())?;
    let target = draft.guild_id.member(ctx, draft.target_id).await.ok();
    if draft.action == MemberAction::Kick && target.is_none() {
        return Err("Could not retrieve that member — they may have already left.".into());
    }
    if let Some(target) = &target {
        if guild.owner_id == target.user.id {
            return Err(format!(
                "Cannot {} **{}** — they are the server owner.",
                draft.action.verb(),
                target.display_name()
            ));
        }
        if !bot_can_interact(ctx, draft.guild_id, &target.roles)
            .await
            .map_err(|err| format!("Could not check Discord role hierarchy: {err}"))?
        {
            return Err(format!(
                "Cannot {} **{}** — their highest role is equal to or above mine.",
                draft.action.verb(),
                target.display_name()
            ));
        }
    }
    let actor_name = actor.display_name().to_string();
    let reason = draft.reason.as_deref().unwrap_or("No reason provided");
    if let Some(target) = &target {
        let dm = if draft.action == MemberAction::Ban {
            format!(
                "You have been banned from **{}**.\n**Reason:** {reason}",
                guild.name
            )
        } else {
            format!(
                "You have been kicked from **{}** by {actor_name}.\n**Reason:** {reason}",
                guild.name
            )
        };
        if let Err(err) = target
            .user
            .direct_message(ctx, CreateMessage::new().content(dm))
            .await
        {
            tracing::warn!(
                "Could not send {} DM to {}: {err}",
                draft.action.verb(),
                draft.target_id
            );
        }
    }
    let audit_reason = format!(
        "{} via Celeste by {actor_name}. Reason: {reason}",
        draft.action.title()
    );
    let result = if draft.action == MemberAction::Ban {
        draft
            .guild_id
            .ban_with_reason(&ctx.http, draft.target_id, 0, &audit_reason)
            .await
    } else {
        draft
            .guild_id
            .kick_with_reason(&ctx.http, draft.target_id, &audit_reason)
            .await
    };
    match result {
        Ok(()) => {
            let detail = format!("Member was {} from {}.", draft.action.past(), guild.name);
            emit_action_log(
                ctx,
                &format!("Discord {}", draft.action.title()),
                &actor_name,
                &draft.target_name,
                draft.target_id,
                reason,
                true,
                &detail,
            )
            .await;
            Ok(format!(
                "**{}** has been {} from the server.",
                draft.target_name,
                draft.action.past()
            ))
        }
        Err(err) => {
            emit_action_log(
                ctx,
                &format!("Discord {}", draft.action.title()),
                &actor_name,
                &draft.target_name,
                draft.target_id,
                reason,
                false,
                &format!("{} failed: {err}", draft.action.title()),
            )
            .await;
            Err(format!(
                "Failed to {} **{}**: {err}",
                draft.action.verb(),
                draft.target_name
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_metadata_matches_permissions() {
        assert_eq!(MemberAction::Ban.permission(), Permissions::BAN_MEMBERS);
        assert_eq!(MemberAction::Kick.permission(), Permissions::KICK_MEMBERS);
    }
}
