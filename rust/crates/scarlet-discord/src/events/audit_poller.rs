//! VRChat group-audit polling and Discord delivery.
//!
//! This ports the durable core of `Scarlet.queryEmit` and the moderation
//! branch of `ScarletDiscord.process`: a five-minute overlap, bounded
//! catch-up windows, audit-id deduplication, Java-compatible metadata files,
//! configured normal/secret routing, event-specific Java-style embeds,
//! instance-monitor threads, and moderation threads with tag and description
//! controls. Every known event is persisted even when no Discord route is
//! configured, matching Java's overlap/deduplication behavior.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use scarlet_core::data::{AuditEntryMetadata, ScarletData};
use scarlet_core::moderation_tags::ModerationTags;
use scarlet_core::settings::Settings;
use scarlet_core::staff_list::StaffList;
use scarlet_vrchat::models::GroupAuditLogEntry;
use serenity::builder::{
    CreateActionRow, CreateButton, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter,
    CreateMessage, CreateThread, EditMessage,
};
use serenity::model::channel::AutoArchiveDuration;
use serenity::model::id::{ChannelId, MessageId};
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
        if is_moderation(&entry.event_type) {
            if let (Some(client), Some(target_id)) = (client.as_ref(), entry.target_id.as_deref()) {
                let (user, groups, represented) = tokio::join!(
                    client.get_user_snapshot(target_id),
                    client.get_user_groups_snapshot(target_id),
                    client.get_user_represented_group_snapshot(target_id),
                );
                match user {
                    Ok(value) => metadata.snapshot_target_user = Some(value),
                    Err(error) => tracing::warn!(%error, target_id, "Could not snapshot moderation target user"),
                }
                match groups {
                    Ok(value) => metadata.snapshot_target_user_groups = Some(value),
                    Err(error) => tracing::warn!(%error, target_id, "Could not snapshot moderation target groups"),
                }
                match represented {
                    Ok(value) => metadata.snapshot_target_user_represented_group = Some(value),
                    Err(error) => tracing::warn!(%error, target_id, "Could not snapshot moderation target represented group"),
                }
            }
        }
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
        let mut pending_claimed = false;
        if entry.event_type == "group.user.ban" {
            if let Some(target_id) = entry.target_id.as_deref() {
                match crate::pending_moderation::PendingModeration::take_from_dir(
                    self.config.data_dir(),
                    target_id,
                ) {
                    Ok(Some(info)) => {
                        // Java clears `contentExtra` here, before looking at
                        // the record's contents, so a Celeste-issued ban is
                        // never labelled "Unclaimed".
                        pending_claimed = true;
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
        if entry.event_type == "group.instance.create" {
            self.create_instance_monitor(&entry, client.as_deref()).await;
        } else if entry.event_type == "group.instance.close" {
            if let Some(location) = entry.target_id.as_deref() {
                self.data.live_instances_set_location_closed_at(location, entry.created_at);
                self.finalise_playerlist(location).await;
            }
        }
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
            let mut message = CreateMessage::new().embed(embed);
            if entry.event_type == "group.request.create" {
                if let Some(target_id) = entry.target_id.as_deref() {
                    message = message.components(group_request_controls(target_id));
                }
            }
            match channel.send_message(&http, message).await {
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
                                    .content(moderation_aux_content(&metadata, &self.moderation_tags, pending_claimed))
                                    .components(moderation_controls(
                                        &entry,
                                        self.settings.timed_bans_enabled.get(),
                                    ));
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

    async fn create_instance_monitor(
        &self,
        entry: &GroupAuditLogEntry,
        client: Option<&scarlet_vrchat::client::VrChatClient>,
    ) {
        let Some(location) = entry.target_id.as_deref() else { return };
        self.data.live_instances_set_location_audit(location, &entry.id);

        let Some(http) = self.http.read().await.clone() else { return };
        let Some(channel) = self
            .config
            .map_value("auditExType2channelSf", "groupex.instance.monitor")
            .and_then(|id| id.parse::<u64>().ok())
            .map(ChannelId::new)
        else { return };

        let world_id = location.split(':').next().unwrap_or(location);
        let instance_name = location
            .split_once(':')
            .map(|(_, instance)| instance.split('~').next().unwrap_or(instance))
            .unwrap_or("instance");
        let world = match client {
            Some(client) => client
                .get_world_cached(world_id, Utc::now().timestamp_millis() - 86_400_000)
                .await
                .ok(),
            None => None,
        };
        let world_name = world.as_ref().map(|world| world.name.as_str()).unwrap_or(world_id);
        let title = format!("{world_name} ({instance_name})");
        let mut embed = CreateEmbed::new()
            .title(title.clone())
            .url(launch_link(location))
            .colour(Colour::new(0x7f7f7f))
            .footer(CreateEmbedFooter::new(format!(
                "{} {} \u{2727} Extended event",
                scarlet_core::identity::APP_NAME,
                scarlet_core::identity::VERSION,
            )))
            .timestamp(serenity::model::Timestamp::now());
        if let Some(image) = world.as_ref().and_then(|world| world.image_url.as_deref()) {
            embed = embed.image(image);
        }

        let message = match channel.send_message(&http, CreateMessage::new().embed(embed)).await {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(%error, location, "Could not create instance monitor message");
                return;
            }
        };
        let thread = match channel
            .create_thread_from_message(
                &http,
                message.id,
                CreateThread::new(truncate(&title, 100)).auto_archive_duration(AutoArchiveDuration::OneHour),
            )
            .await
        {
            Ok(thread) => thread,
            Err(error) => {
                tracing::warn!(%error, location, "Could not create instance monitor thread");
                return;
            }
        };
        let aux_message = thread.id.send_message(&http, CreateMessage::new().content("### Instance log")).await.ok();
        self.data.live_instances_set_location_instance_embed_message(
            location,
            self.config.string("guildSf"),
            Some(channel.get().to_string()),
            Some(message.id.get().to_string()),
            Some(thread.id.get().to_string()),
            aux_message.map(|message| message.id.get().to_string()),
            entry.created_at,
        );
        let playerlist_enabled = self
            .config
            .map_value("auditExType2channelSf", "groupex.instance.playerlist")
            .is_some_and(|value| !value.trim().is_empty());
        if playerlist_enabled {
            let playerlist = CreateEmbed::new()
                .title("Live Player List")
                .description("*No players yet*")
                .colour(Colour::new(0x007f7f))
                .timestamp(serenity::model::Timestamp::now());
            let join_button = CreateActionRow::Buttons(vec![
                CreateButton::new_link(launch_link(location)).label("Join Instance"),
            ]);
            match thread
                .id
                .send_message(
                    &http,
                    CreateMessage::new().embed(playerlist).components(vec![join_button]),
                )
                .await
            {
                Ok(message) => self
                    .data
                    .live_instances_set_playerlist_message(location, message.id.get().to_string()),
                Err(error) => tracing::warn!(%error, location, "Could not create live player-list message"),
            }
        }
    }

    async fn finalise_playerlist(&self, location: &str) {
        let Some(instance) = self.data.live_instances_get_location_instance_embed_message(location, false) else { return };
        let (Some(thread_id), Some(message_id)) = (
            instance.thread_snowflake.as_deref().and_then(|id| id.parse::<u64>().ok()),
            instance.playerlist_message_snowflake.as_deref().and_then(|id| id.parse::<u64>().ok()),
        ) else { return };
        let Some(http) = self.http.read().await.clone() else { return };
        let channel = ChannelId::new(thread_id);
        let message_id = MessageId::new(message_id);
        let message = match channel.message(&http, message_id).await {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(%error, location, "Could not retrieve live player list for finalisation");
                return;
            }
        };
        let Some(existing) = message.embeds.into_iter().next() else { return };
        let embed = CreateEmbed::from(existing)
            .footer(CreateEmbedFooter::new("Session ended"))
            .timestamp(serenity::model::Timestamp::now());
        if let Err(error) = channel
            .edit_message(
                &http,
                message_id,
                EditMessage::new().embed(embed).components(Vec::new()),
            )
            .await
        {
            tracing::warn!(%error, location, "Could not finalise live player list");
        }
    }

    async fn process_timed_bans(&self) {
        if !self.settings.timed_bans_enabled.get() { return; }
        let Some(client) = self.vrchat.read().await.clone() else { return };
        let Some(group_id) = self.group_id() else { return };
        let pending = crate::pending_moderation::PendingModeration::open(self.config.data_dir());
        let now = Utc::now().timestamp_millis();
        for (target_id, timed) in pending.timed_bans() {
            if timed.expires_at > now { continue; }
            let banned = match client.get_group_member(&group_id, &target_id).await {
                Ok(Some(member)) => crate::commands::vrchat_user_ban::is_banned_member(&member),
                Ok(None) => false,
                Err(error) => {
                    tracing::warn!(%error, target_id, "Timed-ban membership lookup failed; will retry");
                    continue;
                }
            };
            if !banned {
                let _ = pending.remove_timed_ban(&target_id);
                continue;
            }
            match pending.add_pending("group.user.unban", &target_id, &timed.actor_user_id) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    tracing::warn!(%error, target_id, "Could not queue expired timed ban");
                    continue;
                }
            }
            match client.unban_from_group(&group_id, &target_id).await {
                Ok(_) => { let _ = pending.remove_timed_ban(&target_id); }
                Err(error) => {
                    let _ = pending.take_pending("group.user.unban", &target_id);
                    tracing::warn!(%error, target_id, "Timed unban failed; will retry");
                }
            }
        }
    }

}

/// Port of the `contentExtra` half of
/// `ScarletDiscordJDA.emitUserModeration_auxMessage`.
///
/// Java starts from `"\nUnclaimed"` and clears it *only* when a
/// `group.user.ban` entry consumed a pending ban record — i.e. when the ban
/// was issued through Celeste, so a moderator is already attached to it, even
/// if they supplied neither tags nor a description. `pending_claimed` carries
/// that "we consumed a pending record" signal.
///
/// Deviation: Java renders the tags/description from the *pending record*
/// only, so re-creating an aux message later (`/fix-audit-thread`) always
/// prints a bare "Unclaimed" and drops any tags the entry has since gained.
/// This reads them back off the entry metadata instead, which for a freshly
/// polled entry is the same text Java would print and for a re-created
/// message is strictly more accurate.
pub(crate) fn moderation_aux_content(
    metadata: &AuditEntryMetadata,
    moderation_tags: &ModerationTags,
    pending_claimed: bool,
) -> String {
    let claimed = pending_claimed || metadata.has_tags() || metadata.has_description();
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
        poller.process_timed_bans().await;
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
    if let Some(title) = match event_type {
        "group.instance.close" => Some("Instance Close"),
        "group.instance.create" => Some("Instance Create"),
        "group.instance.kick" => Some("Instance Kick"),
        "group.instance.warn" => Some("Instance Warn"),
        "group.invite.create" => Some("Invite Create"),
        "group.invite.cancel" => Some("Invite Cancel"),
        "group.member.join" => Some("Member Join"),
        "group.member.leave" => Some("Member Leave"),
        "group.member.remove" => Some("Member Remove"),
        "group.member.role.assign" => Some("Member Role Assign"),
        "group.member.role.unassign" => Some("Member Role Unassign"),
        "group.member.user.update" => Some("Member User Update"),
        "group.instance.announcement" => Some("Announcement"),
        "group.post.create" => Some("Post Create"),
        "group.post.delete" => Some("Post Delete"),
        "group.post.update" => Some("Post Update"),
        "group.calendarEvent.create" => Some("Event Create"),
        "group.calendarEvent.delete" => Some("Event Delete"),
        "group.request.block" => Some("Request Block"),
        "group.request.create" => Some("Request Create"),
        "group.request.reject" => Some("Request Reject"),
        "group.role.create" => Some("Role Create"),
        "group.role.delete" => Some("Role Delete"),
        "group.role.update" => Some("Role Update"),
        "group.gallery.create" => Some("Gallery Create"),
        "group.gallery.delete" => Some("Gallery Delete"),
        "group.gallery.update" => Some("Gallery Update"),
        "group.transfer.accept" => Some("Transfer Accept"),
        "group.transfer.cancel" => Some("Transfer Cancel"),
        "group.transfer.start" => Some("Transfer Start"),
        "group.create" => Some("Create"),
        "group.update" => Some("Update"),
        "group.user.ban" => Some("User Ban"),
        "group.user.unban" => Some("User Unban"),
        _ => None,
    } { return title.to_string(); }
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
        "group.member.join"
        | "group.instance.create"
        | "group.invite.create"
        | "group.role.create"
        | "group.transfer.cancel"
        | "group.transfer.start" => 0x00ff00,
        "group.user.unban" | "group.request.create" => 0x007f00,
        "group.member.role.assign"
        | "group.instance.announcement"
        | "group.post.create"
        | "group.post.update"
        | "group.calendarEvent.create" => 0x00ff7f,
        "group.post.delete" => 0x7fff7f,
        "group.calendarEvent.delete" => 0xff0000,
        "group.member.user.update" | "group.role.update" | "group.update" => 0x007f7f,
        "group.transfer.accept" => 0x0000ff,
        _ => 0x000000,
    }
}

fn launch_link(location: &str) -> String {
    format!(
        "https://vrchat.com/home/launch?worldId={}",
        location.replacen(':', "&instanceId=", 1)
    )
}

fn entry_embed(entry: &GroupAuditLogEntry, target: Option<&scarlet_vrchat::models::User>) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .colour(Colour::new(event_colour(&entry.event_type)))
        .title(event_title(&entry.event_type))
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
        embed = embed.field("Target id", format!("`{target_id}`"), false);
    }
    if is_moderation(&entry.event_type) {
        if let Some(target) = target {
            embed = embed
                .title(target.display_name.as_deref().unwrap_or(&target.id))
                .url(format!("https://vrchat.com/home/user/{}", target.id));
            if let Some(image) = target
                .user_icon
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .or(target.current_avatar_image_url.as_deref())
            {
                embed = embed.image(image);
            }
            if let Some(date_joined) = target.date_joined {
                if let Some(joined) = date_joined.and_hms_opt(0, 0, 0) {
                    let epoch = joined.and_utc().timestamp();
                    embed = embed.field(
                        "Account age",
                        format!("<t:{epoch}:D> (<t:{epoch}:R>)"),
                        false,
                    );
                }
            }
            if let Some(status) = target.age_verification_status.as_deref() {
                embed = embed.field("Age verification", format!("`{status}`"), false);
            }
            if let Some(pronouns) = target.pronouns.as_deref().filter(|value| !value.trim().is_empty()) {
                embed = embed.field("Pronouns", format!("`{pronouns}`"), false);
            }
        } else {
            if let Some(target_id) = entry.target_id.as_deref() {
                embed = embed.title(target_id).url(format!("https://vrchat.com/home/user/{target_id}"));
            }
            embed = embed.field(
                "Note",
                "VRChat did not return full profile data for this user; some fields are omitted.",
                false,
            );
        }
    }

    let data = entry.data.as_ref().and_then(serde_json::Value::as_object);
    let string = |key: &str| data.and_then(|data| data.get(key)).and_then(serde_json::Value::as_str);
    let target_name = target.and_then(|user| user.display_name.as_deref())
        .or(entry.target_id.as_deref()).unwrap_or("Unknown user");
    let target_id = entry.target_id.as_deref().unwrap_or("");
    let group_id = entry.group_id.as_str();
    match entry.event_type.as_str() {
        "group.instance.create" => {
            embed = embed.title("Instance Open").url(launch_link(target_id));
        }
        "group.instance.close" => {
            embed = embed.title("Instance Close").url(launch_link(target_id));
        }
        "group.member.join" => embed = embed.title("Member Join").url(format!("https://vrchat.com/home/user/{target_id}")),
        "group.member.leave" => embed = embed.title("Member Leave").url(format!("https://vrchat.com/home/user/{target_id}")),
        "group.invite.create" => embed = embed.title("Invited User").url(format!("https://vrchat.com/home/user/{target_id}")),
        "group.request.block" => embed = embed.title("Blocked Request").url(format!("https://vrchat.com/home/user/{target_id}")),
        "group.request.create" => embed = embed.title("Created Request").url(format!("https://vrchat.com/home/user/{target_id}")),
        "group.request.reject" => embed = embed.title("Rejected Request").url(format!("https://vrchat.com/home/user/{target_id}")),
        "group.instance.announcement" => {
            embed = embed.title(string("title").unwrap_or("Announcement"));
            if let Some(message) = string("message") { embed = embed.description(message); }
        }
        "group.post.create" => {
            embed = embed.title(string("title").unwrap_or("Post Create"))
                .url(format!("https://vrchat.com/home/group/{group_id}/posts"));
            if let Some(text) = string("text") { embed = embed.description(text); }
            if let Some(image) = string("imageUrl") {
                embed = embed.image(image);
            } else if let Some(image_id) = string("imageId").filter(|id| !id.trim().is_empty()) {
                embed = embed.image(format!("https://vrchat.com/api/1/file/{image_id}/1/file"));
            }
        }
        "group.member.role.assign" | "group.member.role.unassign" => {
            let assigning = entry.event_type == "group.member.role.assign";
            let verb = if assigning { "Assigned" } else { "Unassigned" };
            let prep = if assigning { "to" } else { "from" };
            let role_name = string("roleName").or_else(|| string("roleId")).unwrap_or("Unknown role");
            let role_id = string("roleId").unwrap_or("");
            let actor_name = entry.actor_display_name.as_deref().or(entry.actor_id.as_deref()).unwrap_or("Unknown actor");
            let actor_id = entry.actor_id.as_deref().unwrap_or("");
            embed = embed.title(format!("{verb} Role {prep} {target_name}"))
                .url(format!("https://vrchat.com/home/user/{target_id}"))
                .description(format!(
                    "[{actor_name}](https://vrchat.com/home/user/{actor_id}) {} the role \"[{role_name}](https://vrchat.com/home/group/{group_id}/settings/roles/{role_id})\" {prep} [{target_name}](https://vrchat.com/home/user/{target_id})",
                    if assigning { "assigned" } else { "unassigned" },
                ));
        }
        "group.role.create" | "group.role.delete" => {
            let verb = if entry.event_type.ends_with("create") { "Created" } else { "Deleted" };
            let name = string("name").unwrap_or(target_id);
            embed = embed.title(format!("{verb} Role {name}"))
                .url(format!("https://vrchat.com/home/group/{group_id}/settings/roles/{target_id}"));
        }
        "group.member.user.update" | "group.role.update" | "group.update" | "group.post.update" | "group.gallery.update" => {
            let title = match entry.event_type.as_str() {
                "group.member.user.update" => "Updated Member User".to_string(),
                "group.role.update" => format!("Updated Role {target_id}"),
                "group.update" => "Updated Group".to_string(),
                "group.post.update" => "Updated Post".to_string(),
                _ => event_title(&entry.event_type),
            };
            embed = embed.title(title);
            if entry.event_type == "group.update" { embed = embed.url(format!("https://vrchat.com/home/group/{group_id}")); }
            if let Some(data) = data {
                for (key, change) in data {
                    if let Some(change) = change.as_object() {
                        let old = change.get("old").map(render_json_value).unwrap_or_else(|| "null".into());
                        let new = change.get("new").map(render_json_value).unwrap_or_else(|| "null".into());
                        embed = embed.field(format!("`{key}`"), format!("old: `{}`\nnew: `{}`", truncate(&old, 480), truncate(&new, 480)), false);
                    }
                }
            }
        }
        _ => {
            if let Some(data) = data {
                for (key, value) in data {
                    let rendered = if let Some(change) = value.as_object()
                        .filter(|change| change.len() == 2 && change.contains_key("old") && change.contains_key("new"))
                    {
                        let old = change.get("old").map(render_json_value).unwrap_or_else(|| "null".into());
                        let new = change.get("new").map(render_json_value).unwrap_or_else(|| "null".into());
                        format!("old: `{}`\nnew: `{}`", truncate(&old, 480), truncate(&new, 480))
                    } else {
                        format!("`{}`", truncate(&render_json_value(value), 960))
                    };
                    embed = embed.field(format!("`{key}`"), rendered, false);
                }
            }
        }
    }
    embed
}

fn render_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn moderation_controls(entry: &GroupAuditLogEntry, timed_bans: bool) -> Vec<CreateActionRow> {
    let mut rows = vec![CreateActionRow::Buttons(vec![
        CreateButton::new(format!("edit-tags:{}", entry.id)).label("Edit tags"),
        CreateButton::new(format!("edit-desc:{}", entry.id)).label("Edit description"),
        CreateButton::new(format!("event-redact:{}", entry.id)).label("Redact event"),
        CreateButton::new(format!("event-unredact:{}", entry.id)).label("Unredact event"),
    ]), CreateActionRow::Buttons(vec![
        CreateButton::new(format!("view-snapshot-user:{}", entry.id)).label("Snapshot: user"),
        CreateButton::new(format!("view-snapshot-user-groups:{}", entry.id)).label("Snapshot: user groups"),
        CreateButton::new(format!("view-snapshot-user-represented-group:{}", entry.id))
            .label("Snapshot: represented group"),
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
        if timed_bans {
            rows.push(CreateActionRow::Buttons(vec![
                CreateButton::new(format!("timed-ban:6:{target_id}")).label("Timed ban 6h"),
                CreateButton::new(format!("timed-ban:24:{target_id}")).label("Timed ban 24h"),
                CreateButton::new(format!("timed-ban:72:{target_id}")).label("Timed ban 3d"),
                CreateButton::new(format!("timed-ban:168:{target_id}")).label("Timed ban 7d"),
                CreateButton::new(format!("timed-ban-custom:{target_id}")).label("Timed ban…"),
            ]));
        }
    }
    rows
}

fn group_request_controls(target_id: &str) -> Vec<CreateActionRow> {
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(format!("group-request-accept:{target_id}"))
            .style(serenity::model::application::ButtonStyle::Success)
            .label("Accept"),
        CreateButton::new(format!("group-request-reject:{target_id}"))
            .style(serenity::model::application::ButtonStyle::Secondary)
            .label("Reject"),
        CreateButton::new(format!("group-request-block:{target_id}"))
            .style(serenity::model::application::ButtonStyle::Danger)
            .label("Block"),
    ])]
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
    fn a_consumed_pending_ban_is_never_labelled_unclaimed() {
        // Java sets `contentExtra = ""` the moment `pollBanInfo` returns a
        // record, before it looks at that record's tags or description.
        let dir = std::env::temp_dir().join("celeste-aux-content-test");
        std::fs::create_dir_all(&dir).unwrap();
        let tags = ModerationTags::open(&dir, "moderation_tags_absent.json");
        let metadata = AuditEntryMetadata::default();
        assert_eq!(moderation_aux_content(&metadata, &tags, true), "");
        assert_eq!(moderation_aux_content(&metadata, &tags, false), "Unclaimed");
    }

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

    #[test]
    fn join_request_controls_offer_all_java_actions() {
        let value = serde_json::to_value(group_request_controls("usr_1")).unwrap();
        let text = value.to_string();
        assert!(text.contains("group-request-accept:usr_1"));
        assert!(text.contains("group-request-reject:usr_1"));
        assert!(text.contains("group-request-block:usr_1"));
    }

    #[test]
    fn moderation_controls_offer_all_snapshot_views() {
        let entry = GroupAuditLogEntry {
            id: "gaud_1".into(),
            target_id: Some("usr_1".into()),
            ..GroupAuditLogEntry::default()
        };
        let text = serde_json::to_value(moderation_controls(&entry, false)).unwrap().to_string();
        assert!(text.contains("view-snapshot-user:gaud_1"));
        assert!(text.contains("view-snapshot-user-groups:gaud_1"));
        assert!(text.contains("view-snapshot-user-represented-group:gaud_1"));
    }

    fn audit_entry(event_type: &str, data: serde_json::Value) -> GroupAuditLogEntry {
        GroupAuditLogEntry {
            id: "gaud_1".into(),
            group_id: "grp_1".into(),
            created_at: Utc::now(),
            actor_id: Some("usr_actor".into()),
            actor_display_name: Some("Moderator".into()),
            target_id: Some("usr_target".into()),
            event_type: event_type.into(),
            description: Some("generic API description".into()),
            data: Some(data),
        }
    }

    #[test]
    fn announcement_uses_its_title_and_message_instead_of_raw_json() {
        let entry = audit_entry("group.instance.announcement", serde_json::json!({
            "title": "Movie night", "message": "Starting in ten minutes", "sendNotification": true
        }));
        let json = serde_json::to_value(entry_embed(&entry, None)).unwrap();
        assert_eq!(json["title"], "Movie night");
        assert_eq!(json["description"], "Starting in ten minutes");
        assert!(!json.to_string().contains("Event data"));
    }

    #[test]
    fn update_events_render_java_old_and_new_fields() {
        let entry = audit_entry("group.update", serde_json::json!({
            "name": {"old": "Old name", "new": "New name"},
            "joinState": {"old": "request", "new": "open"}
        }));
        let text = serde_json::to_value(entry_embed(&entry, None)).unwrap().to_string();
        assert!(text.contains("Updated Group"));
        assert!(text.contains("old: `Old name`"));
        assert!(text.contains("new: `New name`"));
        assert!(!text.contains("Event data"));
    }

    #[test]
    fn role_assignment_names_the_role_and_target() {
        let entry = audit_entry("group.member.role.assign", serde_json::json!({
            "roleId": "grol_1", "roleName": "Moderator"
        }));
        let target = scarlet_vrchat::models::User { id: "usr_target".into(), display_name: Some("Member".into()), ..Default::default() };
        let text = serde_json::to_value(entry_embed(&entry, Some(&target))).unwrap().to_string();
        assert!(text.contains("Assigned Role to Member"));
        assert!(text.contains("Moderator"));
        assert!(text.contains("grol_1"));
    }

    #[test]
    fn generic_event_data_is_rendered_as_named_fields() {
        let entry = audit_entry("group.calendarEvent.create", serde_json::json!({
            "title": "Game night", "accessType": "group", "featured": false
        }));
        let json = serde_json::to_value(entry_embed(&entry, None)).unwrap();
        let text = json.to_string();
        assert_eq!(json["title"], "Event Create");
        assert!(text.contains("Event Create"));
        assert!(text.contains("`title`"));
        assert!(text.contains("Game night"));
        assert!(!text.contains("Event data"));
    }

    #[test]
    fn moderation_embed_includes_java_profile_fields() {
        let entry = audit_entry("group.user.ban", serde_json::json!({}));
        let target = scarlet_vrchat::models::User {
            id: "usr_target".into(),
            display_name: Some("Member".into()),
            current_avatar_image_url: Some("https://example.com/avatar.png".into()),
            age_verification_status: Some("18+".into()),
            date_joined: chrono::NaiveDate::from_ymd_opt(2020, 1, 2),
            pronouns: Some("they/them".into()),
            ..Default::default()
        };
        let text = serde_json::to_value(entry_embed(&entry, Some(&target))).unwrap().to_string();
        for expected in ["Member", "avatar.png", "Account age", "Age verification", "18+", "Pronouns", "they/them"] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
    }

    #[test]
    fn every_java_audit_type_has_its_exact_title() {
        let expected = [
            ("group.instance.close", "Instance Close"),
            ("group.instance.create", "Instance Create"),
            ("group.instance.kick", "Instance Kick"),
            ("group.instance.warn", "Instance Warn"),
            ("group.invite.create", "Invite Create"),
            ("group.invite.cancel", "Invite Cancel"),
            ("group.member.join", "Member Join"),
            ("group.member.leave", "Member Leave"),
            ("group.member.remove", "Member Remove"),
            ("group.member.role.assign", "Member Role Assign"),
            ("group.member.role.unassign", "Member Role Unassign"),
            ("group.member.user.update", "Member User Update"),
            ("group.instance.announcement", "Announcement"),
            ("group.post.create", "Post Create"),
            ("group.post.delete", "Post Delete"),
            ("group.post.update", "Post Update"),
            ("group.calendarEvent.create", "Event Create"),
            ("group.calendarEvent.delete", "Event Delete"),
            ("group.request.block", "Request Block"),
            ("group.request.create", "Request Create"),
            ("group.request.reject", "Request Reject"),
            ("group.role.create", "Role Create"),
            ("group.role.delete", "Role Delete"),
            ("group.role.update", "Role Update"),
            ("group.gallery.create", "Gallery Create"),
            ("group.gallery.delete", "Gallery Delete"),
            ("group.gallery.update", "Gallery Update"),
            ("group.transfer.accept", "Transfer Accept"),
            ("group.transfer.cancel", "Transfer Cancel"),
            ("group.transfer.start", "Transfer Start"),
            ("group.create", "Create"),
            ("group.update", "Update"),
            ("group.user.ban", "User Ban"),
            ("group.user.unban", "User Unban"),
        ];
        for (event_type, title) in expected {
            assert_eq!(event_title(event_type), title, "wrong title for {event_type}");
        }
    }
}
