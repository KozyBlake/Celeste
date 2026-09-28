//! VRChat group-audit polling and Discord delivery.
//!
//! This ports the durable core of `Scarlet.queryEmit` and the moderation
//! branch of `ScarletDiscord.process`: a five-minute overlap, bounded
//! catch-up windows, audit-id deduplication, Java-compatible metadata files,
//! configured normal/secret routing, and moderation threads with tag and
//! description controls. Event-specific decoration beyond moderation is
//! intentionally incremental; every known event is still persisted and
//! emitted with its actor/target/data rather than silently disappearing.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use scarlet_core::data::{AuditEntryMetadata, ScarletData};
use scarlet_core::moderation_tags::ModerationTags;
use scarlet_core::settings::Settings;
use scarlet_core::staff_list::StaffList;
use scarlet_vrchat::models::GroupAuditLogEntry;
use serenity::builder::{
    CreateActionRow, CreateButton, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter,
    CreateMessage, CreateThread,
};
use serenity::model::channel::AutoArchiveDuration;
use serenity::model::id::ChannelId;
use serenity::model::Colour;

use crate::data::{DiscordConfigStore, SharedHttp, SharedVrChatClient};

const OVERLAP_MINUTES: i64 = 5;
const MAX_QUERY_HOURS: i64 = 24;

pub struct AuditPoller {
    vrchat: SharedVrChatClient,
    http: SharedHttp,
    config: Arc<DiscordConfigStore>,
    data: Arc<ScarletData>,
    settings: Arc<Settings>,
    secret_staff: Arc<StaffList>,
    moderation_tags: Arc<ModerationTags>,
}

impl AuditPoller {
    pub fn new(
        vrchat: SharedVrChatClient,
        http: SharedHttp,
        config: Arc<DiscordConfigStore>,
        data: Arc<ScarletData>,
        settings: Arc<Settings>,
        secret_staff: Arc<StaffList>,
        moderation_tags: Arc<ModerationTags>,
    ) -> Self {
        Self { vrchat, http, config, data, settings, secret_staff, moderation_tags }
    }

    fn group_id(&self) -> Option<String> {
        self.settings
            .store
            .get_string(scarlet_vrchat::client::GROUP_ID_KEY)
            .filter(|id| !id.trim().is_empty())
    }

    async fn poll_once(&self) -> anyhow::Result<()> {
        let Some(client) = self.vrchat.read().await.clone() else { return Ok(()) };
        let Some(group_id) = self.group_id() else { return Ok(()) };
        let now = Utc::now();
        let stored = self.settings.last_audit_query.get_or_supply().unwrap_or(now);
        let max_days = i64::from(self.settings.audit_catchup_max_days.get().clamp(1, 90));
        let cursor = stored.max(now - Duration::days(max_days));
        if cursor != stored {
            tracing::warn!("Audit cursor exceeded the catch-up limit; advancing it to {cursor}");
            self.settings.last_audit_query.set(Some(cursor));
        }
        let (from, to, mut next_cursor) = query_window(cursor, now);
        let entries = client
            .query_group_audit_logs(&group_id, from, to, None, None, None)
            .await?;
        for entry in entries {
            next_cursor = next_cursor.max(entry.created_at);
            if self.data.audit_entry_metadata_exists(&entry.id) {
                continue;
            }
            if let Err(err) = self.process_entry(entry).await {
                tracing::error!("Failed to process VRChat audit entry: {err}");
            }
        }
        self.settings.last_audit_query.set(Some(next_cursor));
        Ok(())
    }

    async fn process_entry(&self, mut entry: GroupAuditLogEntry) -> anyhow::Result<()> {
        let client = self.vrchat.read().await.clone();
        if entry.actor_display_name.is_none() {
            if let (Some(client), Some(actor_id)) = (client.as_ref(), entry.actor_id.as_deref()) {
                if let Ok(actor) = client.get_user(actor_id).await {
                    entry.actor_display_name = actor.display_name;
                }
            }
        }

        let mut metadata = AuditEntryMetadata {
            entry: Some(serde_json::to_value(&entry)?),
            ..AuditEntryMetadata::default()
        };
        if let Some(target_id) = entry.target_id.as_deref() {
            match crate::pending_moderation::PendingModeration::take_pending_from_dir(
                self.config.data_dir(),
                &entry.event_type,
                target_id,
            ) {
                Ok(Some(actor_id)) => {
                    metadata.aux_actor_display_name = match client.as_ref() {
                        Some(client) => client.get_user(&actor_id).await.ok()
                            .and_then(|user| user.display_name)
                            .or_else(|| Some(actor_id.clone())),
                        None => Some(actor_id.clone()),
                    };
                    metadata.aux_actor_id = Some(actor_id);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, target_id, "Could not consume pending moderation action"),
            }
        }
        if entry.event_type == "group.user.ban" {
            if let Some(target_id) = entry.target_id.as_deref() {
                match crate::pending_moderation::PendingModeration::take_from_dir(
                    self.config.data_dir(),
                    target_id,
                ) {
                    Ok(Some(info)) => {
                        if let Some(tags) = info.tags {
                            metadata.entry_tags.add_all(tags);
                        }
                        metadata.entry_description = info.description
                            .map(|description| description.trim().to_string())
                            .filter(|description| !description.is_empty());
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(%error, target_id, "Could not consume pending ban metadata"),
                }
            }
        }
        let secret = entry.actor_id.as_deref().is_some_and(|id| self.secret_staff.is_id(id));
        let map = if secret { "auditType2secretChannelSf" } else { "auditType2channelSf" };
        let channel = self
            .config
            .map_value(map, &entry.event_type)
            .and_then(|id| id.parse::<u64>().ok())
            .map(ChannelId::new);

        if let (Some(http), Some(channel)) = (self.http.read().await.clone(), channel) {
            let target = if let (Some(client), Some(target_id)) = (client.as_ref(), entry.target_id.as_deref()) {
                client.get_user(target_id).await.ok()
            } else {
                None
            };
            let embed = entry_embed(&entry, target.as_ref());
            match channel.send_message(&http, CreateMessage::new().embed(embed)).await {
                Ok(message) => {
                    metadata.guild_snowflake = self.config.string("guildSf");
                    metadata.channel_snowflake = Some(channel.get().to_string());
                    metadata.message_snowflake = Some(message.id.get().to_string());
                    if is_moderation(&entry.event_type) {
                        let thread_name = thread_name(&entry);
                        match channel
                            .create_thread_from_message(
                                &http,
                                message.id,
                                CreateThread::new(thread_name)
                                    .auto_archive_duration(AutoArchiveDuration::OneHour),
                            )
                            .await
                        {
                            Ok(thread) => {
                                metadata.thread_snowflake = Some(thread.id.get().to_string());
                                let aux = CreateMessage::new()
                                    .content(moderation_aux_content(&metadata, &self.moderation_tags))
                                    .components(moderation_controls(&entry));
                                if let Ok(aux_message) = thread.id.send_message(&http, aux).await {
                                    metadata.aux_message_snowflake = Some(aux_message.id.get().to_string());
                                }
                            }
                            Err(err) => tracing::warn!("Could not create moderation thread for {}: {err}", entry.id),
                        }
                    }
                }
                Err(err) => tracing::warn!("Could not emit audit entry {}: {err}", entry.id),
            }
        }

        // Java records known entries even when no route is configured, so
        // overlap polling cannot repeatedly process the same event.
        self.data.set_audit_entry_metadata(&entry.id, metadata);
        Ok(())
    }

}

pub(crate) fn moderation_aux_content(
    metadata: &AuditEntryMetadata,
    moderation_tags: &ModerationTags,
) -> String {
    let claimed = metadata.has_tags() || metadata.has_description();
    let mut content = if claimed { String::new() } else { "Unclaimed".to_string() };
    if metadata.has_tags() {
        content.push_str("\n### Tags:\n");
        content.push_str(&metadata.entry_tags.iter()
            .map(|tag| moderation_tags.get_tag_label(tag))
            .collect::<Vec<_>>()
            .join(", "));
    }
    if let Some(description) = metadata.entry_description.as_deref() {
        content.push_str("\n### Description:\n");
        content.push_str(description);
    }
    content.trim_start().to_string()
}

pub async fn run(poller: AuditPoller) {
    loop {
        if let Err(err) = poller.poll_once().await {
            tracing::warn!("VRChat audit poll failed; the cursor was not advanced: {err}");
        }
        let seconds = poller.settings.audit_polling_interval.get().clamp(10, 300) as u64;
        tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
    }
}

fn query_window(cursor: DateTime<Utc>, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) {
    let from = cursor - Duration::minutes(OVERLAP_MINUTES);
    let maximum = from + Duration::hours(MAX_QUERY_HOURS);
    if maximum < now {
        (from, maximum, maximum)
    } else {
        (from, now, cursor)
    }
}

fn is_moderation(event_type: &str) -> bool {
    matches!(
        event_type,
        "group.instance.warn"
            | "group.instance.kick"
            | "group.member.remove"
            | "group.user.ban"
            | "group.user.unban"
    )
}

fn event_title(event_type: &str) -> String {
    event_type
        .strip_prefix("group.")
        .unwrap_or(event_type)
        .split('.')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn event_colour(event_type: &str) -> u32 {
    match event_type {
        "group.instance.close" | "group.member.remove" | "group.role.delete" | "group.user.ban" => 0xff0000,
        "group.instance.kick" => 0xff7f00,
        "group.instance.warn" => 0xffff00,
        "group.member.leave" | "group.member.role.unassign" | "group.request.reject" => 0xff007f,
        "group.member.join" | "group.instance.create" | "group.invite.create" | "group.role.create" => 0x00ff00,
        "group.user.unban" | "group.request.create" => 0x007f00,
        "group.member.role.assign" | "group.instance.announcement" | "group.post.create" => 0x00ff7f,
        "group.member.user.update" | "group.role.update" | "group.update" => 0x007f7f,
        "group.transfer.accept" => 0x0000ff,
        _ => 0x000000,
    }
}

fn entry_embed(entry: &GroupAuditLogEntry, target: Option<&scarlet_vrchat::models::User>) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .colour(Colour::new(event_colour(&entry.event_type)))
        .description(entry.description.clone().unwrap_or_else(|| event_title(&entry.event_type)))
        .timestamp(serenity::model::Timestamp::from_unix_timestamp(entry.created_at.timestamp()).expect("chrono timestamp is in range"))
        .footer(CreateEmbedFooter::new(format!(
            "{} {} \u{2727} {}",
            scarlet_core::identity::APP_NAME,
            scarlet_core::identity::VERSION,
            entry.id
        )));
    if let Some(actor) = entry.actor_display_name.as_deref().or(entry.actor_id.as_deref()) {
        let mut author = CreateEmbedAuthor::new(actor);
        if let Some(actor_id) = entry.actor_id.as_deref() {
            author = author.url(format!("https://vrchat.com/home/user/{actor_id}"));
        }
        embed = embed.author(author);
    }
    if let Some(target_id) = entry.target_id.as_deref() {
        let target_name = target
            .and_then(|user| user.display_name.as_deref())
            .unwrap_or(target_id);
        embed = embed
            .title(target_name)
            .url(format!("https://vrchat.com/home/user/{target_id}"))
            .field("Target id", format!("`{target_id}`"), false);
    }
    if let Some(data) = entry.data.as_ref().filter(|data| !data.is_null() && **data != serde_json::json!({})) {
        let rendered = serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string());
        embed = embed.field("Event data", format!("```json\n{}\n```", truncate(&rendered, 900)), false);
    }
    embed
}

pub(crate) fn moderation_controls(entry: &GroupAuditLogEntry) -> Vec<CreateActionRow> {
    let mut rows = vec![CreateActionRow::Buttons(vec![
        CreateButton::new(format!("edit-tags:{}", entry.id)).label("Edit tags"),
        CreateButton::new(format!("edit-desc:{}", entry.id)).label("Edit description"),
        CreateButton::new(format!("event-redact:{}", entry.id)).label("Redact event"),
        CreateButton::new(format!("event-unredact:{}", entry.id)).label("Unredact event"),
    ])];
    if let Some(target_id) = entry.target_id.as_deref() {
        rows.push(CreateActionRow::Buttons(vec![
            CreateButton::new(format!("vrchat-user-ban:{target_id}"))
                .style(serenity::model::application::ButtonStyle::Danger)
                .label("Ban user"),
            CreateButton::new(format!("vrchat-user-unban:{target_id}"))
                .style(serenity::model::application::ButtonStyle::Success)
                .label("Unban user"),
        ]));
    }
    rows
}

pub(crate) fn thread_name(entry: &GroupAuditLogEntry) -> String {
    let value = entry
        .description
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| entry.target_id.as_deref().unwrap_or(&entry.event_type));
    truncate(value, 100)
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_does_not_rewind_the_persisted_cursor() {
        let cursor = "2026-09-27T12:00:00Z".parse().unwrap();
        let now = cursor + Duration::minutes(30);
        let (from, to, next) = query_window(cursor, now);
        assert_eq!(from, cursor - Duration::minutes(5));
        assert_eq!(to, now);
        assert_eq!(next, cursor);
    }

    #[test]
    fn backlog_is_advanced_in_one_day_chunks() {
        let cursor = "2026-09-01T00:00:00Z".parse().unwrap();
        let now = cursor + Duration::days(3);
        let (from, to, next) = query_window(cursor, now);
        assert_eq!(from, cursor - Duration::minutes(5));
        assert_eq!(to, from + Duration::hours(24));
        assert_eq!(next, to);
    }

    #[test]
    fn moderation_inventory_matches_java() {
        for event in [
            "group.instance.warn",
            "group.instance.kick",
            "group.member.remove",
            "group.user.ban",
            "group.user.unban",
        ] {
            assert!(is_moderation(event));
        }
        assert!(!is_moderation("group.member.join"));
    }
}
