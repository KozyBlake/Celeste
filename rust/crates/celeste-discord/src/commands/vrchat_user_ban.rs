//! Port of `ScarletDiscordCommands.vrchatUserBan`.
//!
//! Takes Java's `vrchat-user` option, with live user autocomplete, instead of
//! autocomplete-resolved `io.github.vrchatapi.model.User` (see the
//! `commands` module docs). The training-simulation branch is deliberately
//! out of scope; the normal and immediate-tag ban paths are both real.

use serenity::builder::{
    CreateActionRow, CreateButton, CreateCommand, CreateCommandOption,
    CreateInteractionResponseFollowup, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

pub const NAME: &str = "vrchat-user-ban";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Bans a VRChat user from the configured group").add_option(
        CreateCommandOption::new(CommandOptionType::String, "vrchat-user", "The VRChat user id (usr_...) to ban")
            .required(true)
            .set_autocomplete(true),
    ).add_option(CreateCommandOption::new(
        CommandOptionType::Boolean,
        "tag-immediately",
        "Whether to submit tags and a description now",
    ))
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let resolved = options.iter().find(|o| o.name == "vrchat-user").map(|o| o.value.clone());
    let Some(ResolvedValue::String(target_id)) = resolved else {
        return super::reply_ephemeral(ctx, command, "A `vrchat-user` option is required.").await;
    };
    let tag_immediately = options.iter().find(|o| o.name == "tag-immediately")
        .and_then(|o| match o.value { ResolvedValue::Boolean(value) => Some(value), _ => None })
        .unwrap_or(false);

    if tag_immediately {
        return begin_immediate(ctx, command, target_id).await;
    }

    match execute(ctx, &command.user.id.to_string(), command.member.as_deref(), target_id, true).await {
        Ok(message) => command
            .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(message))
            .await
            .map(|_| ()),
        Err(message) => super::reply_ephemeral(ctx, command, &message).await,
    }
}

async fn begin_immediate(
    ctx: &Context,
    command: &CommandInteraction,
    target_id: &str,
) -> serenity::Result<()> {
    let context = match super::moderation_context_for_snowflake_with_override(
        ctx,
        &command.user.id.to_string(),
        command.member.as_deref(),
        celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
        Some(celeste_core::permission::CelestePermission::GroupexBansManage.id()),
        Some("ban users"),
    ).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if context.client.get_user(target_id).await.is_err() {
        return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {target_id}")).await;
    }
    match context.client.get_group_member(&context.group_id, target_id).await {
        Ok(Some(member)) if is_banned_member(&member) => {
            return super::reply_ephemeral(ctx, command, "This VRChat user is already banned").await;
        }
        Ok(_) => {}
        Err(error) => {
            return super::reply_ephemeral(ctx, command, &format!("Could not check ban status: {error}")).await;
        }
    }
    let (tags, pending) = {
        let data = ctx.data.read().await;
        (
            data.get::<crate::data::ModerationTagsKey>().cloned(),
            data.get::<crate::data::PendingModerationKey>().cloned(),
        )
    };
    let Some(tags) = tags else {
        return super::reply_ephemeral(ctx, command, "Moderation tags are unavailable.").await;
    };
    let available = tags.get_tags();
    if available.is_empty() {
        return super::reply_ephemeral(ctx, command, "No moderation tags!").await;
    }
    let Some(pending) = pending else {
        return super::reply_ephemeral(ctx, command, "Pending moderation state is unavailable.").await;
    };
    match pending.add_ban_info(target_id, &command.user.id.to_string()) {
        Ok(true) => {}
        Ok(false) => return super::reply_ephemeral(ctx, command, "This VRChat user currently has automated/assisted moderation pending, please retry later").await,
        Err(error) => return super::reply_ephemeral(ctx, command, &format!("Could not save the pending ban: {error}")).await,
    }

    let available_count = available.len();
    let max_values = available_count.min(25) as u8;
    let options = available.into_iter().take(25).map(|tag| {
        let label = if tag.label.is_empty() { &tag.value } else { &tag.label };
        let mut option = CreateSelectMenuOption::new(
            label.chars().take(100).collect::<String>(),
            tag.value.chars().take(100).collect::<String>(),
        );
        if !tag.description.is_empty() {
            option = option.description(tag.description.chars().take(50).collect::<String>());
        }
        option
    }).collect();
    let select = CreateSelectMenu::new(
        format!("immediate-ban-select-tags:{target_id}"),
        CreateSelectMenuKind::String { options },
    ).min_values(0).max_values(max_values).placeholder(if available_count > 25 {
        "Select tags (first 25 shown)"
    } else {
        "Select tags"
    });
    let response = command.create_followup(&ctx.http, CreateInteractionResponseFollowup::new()
        .ephemeral(true)
        .components(vec![
            CreateActionRow::SelectMenu(select),
            CreateActionRow::Buttons(vec![
                CreateButton::new(format!("immediate-ban-edit-desc:{target_id}")).label("Set description"),
                CreateButton::new(format!("immediate-ban-cancel:{target_id}")).label("Cancel").style(serenity::model::application::ButtonStyle::Secondary),
                CreateButton::new(format!("immediate-ban-confirm:{target_id}")).label("Ban").style(serenity::model::application::ButtonStyle::Danger),
            ]),
        ])
    ).await;
    if response.is_err() {
        let _ = pending.take_ban_info(target_id);
    }
    response.map(|_| ())
}

/// The ban/unban action itself, shared by `/vrchat-user-ban`,
/// `/vrchat-user-unban` and the **Ban user**/**Unban user** buttons Celeste
/// attaches to its instance-event embeds (port of
/// `ScarletDiscordUI._vrchatUserBan`/`_vrchatUserUnban`, which are likewise
/// one implementation behind both entry points).
///
/// Returns the message to show on success, or the reason it didn't happen.
/// Both are Java's own wording.
pub(crate) async fn execute(
    ctx: &Context,
    actor_snowflake: &str,
    member: Option<&serenity::model::guild::Member>,
    target_id: &str,
    ban: bool,
) -> Result<String, String> {
    let context = super::moderation_context_for_snowflake_with_override(
        ctx,
        actor_snowflake,
        member,
        celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
        Some(celeste_core::permission::CelestePermission::GroupexBansManage.id()),
        Some(if ban { "ban users" } else { "unban users" }),
    )
    .await?;

    execute_with_context(ctx, &context, target_id, ban).await
}

pub(crate) fn is_banned_member(member: &celeste_vrchat::models::GroupMember) -> bool {
    member.banned_at.is_some()
        || member.membership_status.as_deref().is_some_and(|status| status.eq_ignore_ascii_case("banned"))
}

pub(crate) async fn execute_with_context(
    ctx: &Context,
    context: &super::ModerationContext,
    target_id: &str,
    ban: bool,
) -> Result<String, String> {

    let display_name = match context.client.get_user(target_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| target_id.to_string()),
        Err(err) => return Err(format!("No VRChat user found with id {target_id} ({err})")),
    };

    let currently_banned = match context.client.get_group_member(&context.group_id, target_id).await {
        Ok(Some(member)) => is_banned_member(&member),
        Ok(None) => false,
        Err(err) => return Err(format!("Could not check ban status: {err}")),
    };

    if ban {
        if currently_banned {
            return Err("This VRChat user is already banned".to_string());
        }
        let pending = { ctx.data.read().await.get::<crate::data::PendingModerationKey>().cloned() }
            .ok_or_else(|| "Pending moderation state is unavailable.".to_string())?;
        let actor_id = context.actor_id.as_deref().ok_or_else(|| "Linked VRChat actor is unavailable.".to_string())?;
        match pending.add_pending("group.user.ban", target_id, actor_id) {
            Ok(true) => {}
            Ok(false) => return Err("This VRChat user currently has automated/assisted moderation pending, please retry later".to_string()),
            Err(error) => return Err(format!("Could not save the pending moderation action: {error}")),
        }
        match context.client.ban_from_group(&context.group_id, target_id).await {
            Ok(_) => Ok(format!("Banned {display_name}")),
            Err(err) => {
                let _ = pending.take_pending("group.user.ban", target_id);
                Err(format!("Failed to ban {display_name}: {err}"))
            }
        }
    } else {
        if !currently_banned {
            return Err("This VRChat user is not banned".to_string());
        }
        let pending = { ctx.data.read().await.get::<crate::data::PendingModerationKey>().cloned() }
            .ok_or_else(|| "Pending moderation state is unavailable.".to_string())?;
        let actor_id = context.actor_id.as_deref().ok_or_else(|| "Linked VRChat actor is unavailable.".to_string())?;
        match pending.add_pending("group.user.unban", target_id, actor_id) {
            Ok(true) => {}
            Ok(false) => return Err("This VRChat user currently has automated/assisted moderation pending, please retry later".to_string()),
            Err(error) => return Err(format!("Could not save the pending moderation action: {error}")),
        }
        match context.client.unban_from_group(&context.group_id, target_id).await {
            Ok(_) => Ok(format!("Unbanned {display_name}")),
            Err(err) => {
                let _ = pending.take_pending("group.user.unban", target_id);
                Err(format!("Failed to unban {display_name}: {err}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_java_immediate_tag_option() {
        let command = serde_json::to_value(register()).unwrap();
        let options = command["options"].as_array().unwrap();
        let option = options.iter().find(|option| option["name"] == "tag-immediately").unwrap();
        assert_eq!(option["type"], 5);
        assert_ne!(option["required"], true);
    }

    #[test]
    fn recognizes_banned_members_from_either_api_field() {
        let by_date = celeste_vrchat::models::GroupMember {
            banned_at: Some("2026-01-01T00:00:00Z".into()),
            ..Default::default()
        };
        let by_status = celeste_vrchat::models::GroupMember {
            membership_status: Some("BANNED".into()),
            ..Default::default()
        };
        assert!(is_banned_member(&by_date));
        assert!(is_banned_member(&by_status));
        assert!(!is_banned_member(&Default::default()));
    }
}
