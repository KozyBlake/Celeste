//! Shared implementation for Java's four audit/extended/secret event
//! channel-routing commands.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::model::channel::{ChannelType, PartialChannel};

use crate::data::DiscordConfigKey;

pub const AUDIT_NAME: &str = "set-audit-channel";
pub const EXTENDED_NAME: &str = "set-audit-ex-channel";
pub const SECRET_AUDIT_NAME: &str = "set-audit-secret-channel";
pub const SECRET_EXTENDED_NAME: &str = "set-audit-secret-ex-channel";

const AUDIT_TYPES: &[&str] = &[
    "group.instance.close",
    "group.instance.create",
    "group.instance.kick",
    "group.instance.warn",
    "group.invite.create",
    "group.invite.cancel",
    "group.member.join",
    "group.member.leave",
    "group.member.remove",
    "group.member.role.assign",
    "group.member.role.unassign",
    "group.member.user.update",
    "group.instance.announcement",
    "group.post.create",
    "group.post.delete",
    "group.post.update",
    "group.calendarEvent.create",
    "group.calendarEvent.delete",
    "group.request.block",
    "group.request.create",
    "group.request.reject",
    "group.role.create",
    "group.role.delete",
    "group.role.update",
    "group.gallery.create",
    "group.gallery.delete",
    "group.gallery.update",
    "group.transfer.accept",
    "group.transfer.cancel",
    "group.transfer.start",
    "group.create",
    "group.update",
    "group.user.ban",
    "group.user.unban",
];

const EXTENDED_TYPES: &[&str] = &[
    "groupex.instance.inactive",
    "groupex.instance.staff.join",
    "groupex.instance.staff.leave",
    "groupex.instance.user.join",
    "groupex.instance.user.leave",
    "groupex.instance.user.avatar",
    "groupex.instance.user.video",
    "groupex.instance.vtk",
    "groupex.instance.user.pedestal",
    "groupex.instance.user.sticker",
    "groupex.instance.user.print",
    "groupex.instance.user.emoji",
    "groupex.instance.user.prop",
    "groupex.instance.monitor",
    "groupex.instance.playerlist",
    "groupex.instance.enforce",
    "groupex.periodic.mod_summary",
    "groupex.heuristic.mod",
    "groupex.outstanding.mod",
    "groupex.watched.mod",
    "groupex.action.fail",
];

#[derive(Clone, Copy)]
pub enum RouteKind {
    Audit,
    Extended,
    SecretAudit,
    SecretExtended,
}

impl RouteKind {
    fn name(self) -> &'static str {
        match self {
            Self::Audit => AUDIT_NAME,
            Self::Extended => EXTENDED_NAME,
            Self::SecretAudit => SECRET_AUDIT_NAME,
            Self::SecretExtended => SECRET_EXTENDED_NAME,
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Audit => "Sets the text channel certain audit event types use",
            Self::Extended => "Sets the text channel certain extended event types use",
            Self::SecretAudit => "Sets the secret channel certain audit event types use",
            Self::SecretExtended => "Sets the secret channel certain extended event types use",
        }
    }
    fn option(self) -> &'static str {
        if matches!(self, Self::Extended | Self::SecretExtended) {
            "audit-ex-event-type"
        } else {
            "audit-event-type"
        }
    }
    fn map(self) -> &'static str {
        match self {
            Self::Audit => "auditType2channelSf",
            Self::Extended => "auditExType2channelSf",
            Self::SecretAudit => "auditType2secretChannelSf",
            Self::SecretExtended => "auditExType2secretChannelSf",
        }
    }
    fn valid_types(self) -> &'static [&'static str] {
        if matches!(self, Self::Extended | Self::SecretExtended) {
            EXTENDED_TYPES
        } else {
            AUDIT_TYPES
        }
    }
    fn extended(self) -> bool {
        matches!(self, Self::Extended | Self::SecretExtended)
    }
    fn secret(self) -> bool {
        matches!(self, Self::SecretAudit | Self::SecretExtended)
    }
}

pub fn register(kind: RouteKind) -> CreateCommand {
    CreateCommand::new(kind.name())
        .description(kind.description())
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                kind.option(),
                "The event type id",
            )
            .required(true),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::Channel,
            "discord-text-channel",
            "The channel, or omit to unassociate it",
        ))
}

fn string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|option| option.name == name)
        .and_then(|option| match &option.value {
            ResolvedValue::String(value) => Some(*value),
            _ => None,
        })
}
fn channel<'a>(options: &'a [ResolvedOption<'a>]) -> Option<&'a PartialChannel> {
    options
        .iter()
        .find(|option| option.name == "discord-text-channel")
        .and_then(|option| match &option.value {
            ResolvedValue::Channel(value) => Some(*value),
            _ => None,
        })
}
fn is_message_channel(kind: ChannelType) -> bool {
    matches!(
        kind,
        ChannelType::Text
            | ChannelType::News
            | ChannelType::NewsThread
            | ChannelType::PublicThread
            | ChannelType::PrivateThread
            | ChannelType::Forum
    )
}
fn title(id: &str) -> String {
    let known = match id {
        "group.instance.announcement" => Some("Announcement"),
        "group.calendarEvent.create" => Some("Event Create"),
        "group.calendarEvent.delete" => Some("Event Delete"),
        "groupex.instance.inactive" => Some("Instance Inactive"),
        "groupex.instance.staff.join" => Some("Staff Join"),
        "groupex.instance.staff.leave" => Some("Staff Leave"),
        "groupex.instance.user.join" => Some("User Join"),
        "groupex.instance.user.leave" => Some("User Leave"),
        "groupex.instance.user.avatar" => Some("User Switch Avatar"),
        "groupex.instance.user.video" => Some("User Load Video"),
        "groupex.instance.vtk" => Some("Vote-to-Kick Initiated"),
        "groupex.instance.user.pedestal" => Some("User Spawn Pedestal"),
        "groupex.instance.user.sticker" => Some("User Spawn Sticker"),
        "groupex.instance.user.print" => Some("User Spawn Print"),
        "groupex.instance.user.emoji" => Some("User Spawn Emoji"),
        "groupex.instance.user.prop" => Some("User Spawn Prop"),
        "groupex.instance.monitor" => Some("Instance Monitor"),
        "groupex.instance.playerlist" => Some("Live Player List"),
        "groupex.instance.enforce" => Some("Instance Enforcement"),
        "groupex.periodic.mod_summary" => Some("Moderation Summary"),
        "groupex.heuristic.mod" => Some("Suggested Moderation"),
        "groupex.outstanding.mod" => Some("Outstanding Moderation"),
        "groupex.watched.mod" => Some("Watched Moderation"),
        "groupex.action.fail" => Some("Action Failure"),
        _ => None,
    };
    if let Some(known) = known {
        return known.to_string();
    }
    id.strip_prefix("groupex.")
        .or_else(|| id.strip_prefix("group."))
        .unwrap_or(id)
        .split('.')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    kind: RouteKind,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let options = command.data.options();
    let Some(event_type) = string(&options, kind.option()) else {
        return super::reply_ephemeral(ctx, command, "An event type is required.").await;
    };
    if !kind.valid_types().contains(&event_type) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{event_type}` is not a valid event type."),
        )
        .await;
    }
    let selected = channel(&options);
    if let Some(selected) = selected {
        if !is_message_channel(selected.kind) {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!(
                    "The channel {} doesn't support message sending",
                    selected.name.as_deref().unwrap_or("selected")
                ),
            )
            .await;
        }
    }
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is unavailable.").await;
    };
    if let Err(err) = config.set_map_value(
        kind.map(),
        event_type,
        selected.map(|channel| channel.id.to_string()).as_deref(),
    ) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Could not save Discord configuration: {err}"),
        )
        .await;
    }
    let category = if kind.extended() {
        "VRChat extended group audit event type"
    } else {
        "VRChat group audit log event type"
    };
    let secret = if kind.secret() { " secret" } else { "" };
    let message = match selected {
        Some(channel) => format!(
            "Associating {category} {} ({event_type}) with{secret} channel <#{}>",
            title(event_type),
            channel.id
        ),
        None => format!(
            "Unassociating {category} {} ({event_type}) from any{secret} channels",
            title(event_type)
        ),
    };
    super::reply_ephemeral(ctx, command, &message).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_known_event_ids() {
        assert!(AUDIT_TYPES.contains(&"group.user.ban"));
        assert!(EXTENDED_TYPES.contains(&"groupex.action.fail"));
    }
    #[test]
    fn title_fallback_is_readable() {
        assert_eq!(title("groupex.instance.user.join"), "User Join");
        assert_eq!(title("group.calendarEvent.create"), "Event Create");
    }
}
