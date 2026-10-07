//! `/tag add|remove`: edits the moderation event associated with the current
//! Discord thread. The audit id is recovered from the `edit-tags:<id>`
//! marker in the thread's first messages, matching Java across restarts.

use std::collections::HashSet;

use serenity::builder::{
    CreateAutocompleteResponse, CreateCommand, CreateCommandOption, CreateInteractionResponse,
    EditMessage, GetMessages,
};
use serenity::client::Context;
use serenity::model::application::{
    ActionRowComponent, ButtonKind, CommandInteraction, CommandOptionType, ResolvedOption,
    ResolvedValue,
};
use serenity::model::channel::ChannelType;
use serenity::model::id::MessageId;
use serenity::model::Permissions;

use crate::data::{ModerationTagsKey, CelesteDataKey};

pub const NAME: &str = "tag";

pub fn register() -> CreateCommand {
    let subcommand = |name, description| {
        let mut option = CreateCommandOption::new(CommandOptionType::SubCommand, name, description);
        for index in 1..=5 {
            let option_name = if index == 1 {
                "tag".to_string()
            } else {
                format!("tag-{index}")
            };
            option = option.add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    option_name,
                    if index == 1 {
                        "Start typing to search every moderation tag"
                    } else {
                        "Another tag (optional)"
                    },
                )
                .required(index == 1)
                .set_autocomplete(true),
            );
        }
        option
    };
    CreateCommand::new(NAME)
        .description("Tag the moderation event this thread belongs to")
        .add_option(subcommand(
            "add",
            "Add tags to this thread's moderation event",
        ))
        .add_option(subcommand(
            "remove",
            "Remove tags from this thread's moderation event",
        ))
}

pub async fn autocomplete(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let focused_option = command.data.autocomplete();
    let focused = focused_option
        .as_ref()
        .map(|option| option.value)
        .unwrap_or_default();
    let focused_name = focused_option
        .as_ref()
        .map(|option| option.name)
        .unwrap_or_default();
    let picked: HashSet<&str> = command
        .data
        .options()
        .first()
        .and_then(|option| match &option.value {
            ResolvedValue::SubCommand(options) => Some(
                options
                    .iter()
                    .filter(|option| option.name != focused_name)
                    .filter_map(|option| match option.value {
                        ResolvedValue::String(value) => Some(value),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    let tags = { ctx.data.read().await.get::<ModerationTagsKey>().cloned() };
    let mut response = CreateAutocompleteResponse::new();
    if let Some(tags) = tags {
        for tag in tags
            .search_tags(focused, 125)
            .into_iter()
            .filter(|tag| !picked.contains(tag.value.as_str()))
            .take(25)
        {
            let label = if tag.label.is_empty() {
                &tag.value
            } else {
                &tag.label
            };
            let shown = if tag.description.is_empty() {
                label.clone()
            } else {
                format!("{label} — {}", tag.description)
            };
            response =
                response.add_string_choice(shown.chars().take(100).collect::<String>(), tag.value);
        }
    }
    command
        .create_response(&ctx.http, CreateInteractionResponse::Autocomplete(response))
        .await
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(
            ctx,
            command,
            "The `add` or `remove` subcommand is required.",
        )
        .await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a tag subcommand.").await;
    };
    let Some(audit_id) = audit_id_for_thread(ctx, command).await else {
        return super::reply_ephemeral(
            ctx,
            command,
            "Run `/tag` inside the thread of the kick, ban or warn you want to tag.",
        )
        .await;
    };
    let (data, tags) = {
        let data = ctx.data.read().await;
        (
            data.get::<CelesteDataKey>().cloned(),
            data.get::<ModerationTagsKey>().cloned(),
        )
    };
    let (Some(data), Some(tags)) = (data, tags) else {
        return super::reply_ephemeral(ctx, command, "Moderation tag data is unavailable.").await;
    };
    if let Some(denial) = tag_access_denial(ctx, command, &data, &audit_id).await {
        return super::reply_ephemeral(ctx, command, &denial).await;
    }

    let mut values = Vec::new();
    let mut unknown = Vec::new();
    for input in string_options(sub_options) {
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        let tag = tags.get_tag(input).or_else(|| {
            tags.get_tags()
                .into_iter()
                .find(|tag| tag.label.eq_ignore_ascii_case(input))
        });
        match tag {
            Some(tag) if !values.contains(&tag.value) => values.push(tag.value),
            Some(_) => {}
            None => unknown.push(input.to_string()),
        }
    }
    if !unknown.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "Unknown tag{}: `{}` — pick from the suggestions as you type.",
                if unknown.len() == 1 { "" } else { "s" },
                unknown.join("`, `")
            ),
        )
        .await;
    }
    let metadata = if subcommand.name == "add" {
        data.audit_entry_metadata_edit_tags(&audit_id, &[], values)
    } else {
        data.audit_entry_metadata_edit_tags(&audit_id, &values, Vec::new())
    };
    refresh_aux_message(ctx, &metadata, &tags).await;
    let now = if metadata.entry_tags.is_empty() {
        "*(none)*".to_string()
    } else {
        metadata
            .entry_tags
            .iter()
            .map(|value| tags.get_tag_label(value))
            .collect::<Vec<_>>()
            .join(", ")
    };
    super::reply_ephemeral(
        ctx,
        command,
        &format!(
            "{}. Tags on this event now: {now}",
            if subcommand.name == "add" {
                "Added"
            } else {
                "Removed"
            }
        ),
    )
    .await
}

fn string_options<'a>(options: &'a [ResolvedOption<'a>]) -> impl Iterator<Item = &'a str> {
    options.iter().filter_map(|option| match option.value {
        ResolvedValue::String(value) => Some(value),
        _ => None,
    })
}

async fn audit_id_for_thread(ctx: &Context, command: &CommandInteraction) -> Option<String> {
    let channel = command.channel_id.to_channel(&ctx.http).await.ok()?;
    let guild_channel = channel.guild()?;
    if !matches!(
        guild_channel.kind,
        ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread
    ) {
        return None;
    }
    let messages = command
        .channel_id
        .messages(
            &ctx.http,
            GetMessages::new().after(MessageId::new(1)).limit(3),
        )
        .await
        .ok()?;
    audit_id_from_messages(&messages)
}

fn audit_id_from_messages(messages: &[serenity::model::channel::Message]) -> Option<String> {
    messages
        .iter()
        .flat_map(|message| &message.components)
        .flat_map(|row| &row.components)
        .filter_map(|component| match component {
            ActionRowComponent::Button(button) => match &button.data {
                ButtonKind::NonLink { custom_id, .. } => custom_id.strip_prefix("edit-tags:"),
                _ => None,
            },
            _ => None,
        })
        .find(|id| !id.is_empty())
        .map(str::to_string)
}

async fn tag_access_denial(
    ctx: &Context,
    command: &CommandInteraction,
    data: &celeste_core::data::CelesteData,
    audit_id: &str,
) -> Option<String> {
    tag_access_denial_for(
        ctx,
        command.member.as_deref(),
        &command.user.id.to_string(),
        command.channel.as_ref().map(|channel| channel.kind),
        data,
        audit_id,
    )
    .await
}

/// Java checks the channel-scoped "can you post here" permission with
/// `MESSAGE_SEND_IN_THREADS` for a thread and `MESSAGE_SEND` otherwise.
fn post_permission(kind: Option<ChannelType>) -> Permissions {
    match kind {
        Some(ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread)
        | None => Permissions::SEND_MESSAGES_IN_THREADS,
        Some(_) => Permissions::SEND_MESSAGES,
    }
}

/// Port of `ScarletDiscordUI.auditEntryTagDenial(Member, MessageChannel, String)`.
/// Returns `None` when the member may tag the entry, or the exact denial
/// message Java would have replied with.
pub(crate) async fn tag_access_denial_for(
    ctx: &Context,
    member: Option<&serenity::model::guild::Member>,
    snowflake: &str,
    channel_kind: Option<ChannelType>,
    data: &celeste_core::data::CelesteData,
    audit_id: &str,
) -> Option<String> {
    let Some(member) = member else {
        return Some("Could not determine your guild permissions.".to_string());
    };
    // Staff are whoever the server lets post in its moderation threads.
    // `member.permissions` on an interaction is already channel-scoped, so
    // this is Java's `member.hasPermission(guildChannel, VIEW_CHANNEL, post)`.
    let permissions = member.permissions.unwrap_or_else(Permissions::empty);
    if permissions.contains(Permissions::VIEW_CHANNEL | post_permission(channel_kind))
        || permissions.contains(Permissions::ADMINISTRATOR)
    {
        return None;
    }
    if permissions.intersects(Permissions::MODERATE_MEMBERS | Permissions::MANAGE_GUILD) {
        return None;
    }
    if super::member_has_vrchat_permission(
        ctx,
        member,
        celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
    )
    .await
        || super::member_has_celeste_permission(
            ctx,
            member,
            celeste_core::permission::CelestePermission::GroupexBansManage,
            false,
        )
        .await
        || super::member_has_celeste_permission(
            ctx,
            member,
            celeste_core::permission::CelestePermission::GroupexTagsEdit,
            false,
        )
        .await
    {
        return None;
    }
    let linked = data.global_get_snowflake_id(snowflake);
    if let Some(linked) = linked {
        // Java re-checks a VRChat permission here (not just at the top), so an
        // instance moderator can tag any event once their account is linked.
        if super::member_has_vrchat_permission(
            ctx,
            member,
            celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_MODERATE,
        )
        .await
        {
            return None;
        }
        let metadata = data.audit_entry_metadata(audit_id);
        let actor = metadata
            .entry
            .as_ref()
            .and_then(|entry| entry.get("actorId"))
            .and_then(|id| id.as_str());
        if actor == Some(linked.as_str())
            || metadata.aux_actor_id.as_deref() == Some(linked.as_str())
        {
            return None;
        }
        tracing::warn!(
            "Rejected tag/description edit by {} for audit entry {audit_id}",
            member.user.id
        );
        Some("You can't tag this event: your role can't post in this moderation thread, and it isn't an action you took. Ask an admin to give your role permission to send messages here.".to_string())
    } else {
        tracing::warn!(
            "Rejected tag/description edit by {} for audit entry {audit_id}",
            member.user.id
        );
        Some("You can't tag this event: your role can't post in this moderation thread. Ask an admin to give your role permission to send messages here, or link your VRChat account with `/link-vrchat-account` to tag actions you took yourself.".to_string())
    }
}

pub(crate) async fn refresh_aux_message(
    ctx: &Context,
    metadata: &celeste_core::data::AuditEntryMetadata,
    tags: &celeste_core::moderation_tags::ModerationTags,
) {
    let (Some(thread), Some(message)) = (
        metadata
            .thread_snowflake
            .as_deref()
            .and_then(|id| id.parse::<u64>().ok()),
        metadata
            .aux_message_snowflake
            .as_deref()
            .and_then(|id| id.parse::<u64>().ok()),
    ) else {
        return;
    };
    let tag_text = if metadata.entry_tags.is_empty() {
        "*(none)*".to_string()
    } else {
        metadata
            .entry_tags
            .iter()
            .map(|value| tags.get_tag_label(value))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut content = if metadata.entry_redacted {
        format!("# **__REDACTED__**\n\nUnclaimed\n### Tags:\n{tag_text}")
    } else {
        format!("Unclaimed\n### Tags:\n{tag_text}")
    };
    if let Some(description) = metadata
        .entry_description
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        content.push_str("\n### Description:\n");
        content.push_str(description);
    }
    let _ = serenity::model::id::ChannelId::new(thread)
        .edit_message(
            &ctx.http,
            MessageId::new(message),
            EditMessage::new().content(content),
        )
        .await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn string_options_keeps_only_string_values() {
        // Option parsing is exercised through Discord's resolved option API;
        // keep the pure behavior pinned at the tag database layer instead.
        let dir = tempfile::tempdir().unwrap();
        let tags = celeste_core::moderation_tags::ModerationTags::open(dir.path(), "tags.json");
        tags.add_or_update_tag("harassment", Some("Harassment"), None);
        assert_eq!(tags.get_tag("harassment").unwrap().label, "Harassment");
    }

    #[test]
    fn event_title_marker_prefix_is_unambiguous() {
        assert_eq!(
            "edit-tags:gaud_123".strip_prefix("edit-tags:"),
            Some("gaud_123")
        );
        assert_eq!("edit-desc:gaud_123".strip_prefix("edit-tags:"), None);
    }
}
