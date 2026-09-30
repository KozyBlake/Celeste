//! Port of `ScarletDiscordJDA`'s `condEmitEx` routing and the
//! `emitExtended*` embeds, as an implementation of
//! [`scarlet_events::AuditEmitter`].
//!
//! This is what turns the ported event listener's group-instance events into
//! actual Discord posts. Routing, gating and embed shape all follow Java:
//!
//! - The channel comes from `discord_bot.json`'s `auditExType2channelSf`
//!   (or `auditExType2secretChannelSf`), the same maps
//!   `/set-audit-ex-channel` and `/set-audit-secret-ex-channel` write. No
//!   channel configured for an event type means nothing is posted, and
//!   `is_emitting` reports false so the listener skips the work entirely.
//! - When the instance has a live instance-monitor embed in that same
//!   channel, the post goes into its thread instead — Java's
//!   `liveInstancesMetadata_getLocationInstanceEmbedMessage` lookup, ported
//!   here against the same data.
//! - Embeds carry Java's exact titles, descriptions, field names, colours
//!   (from `GroupAuditTypeEx`) and footer, and the same Ban/Unban buttons,
//!   which are handled in `crate::interactions`.
//!
//! **Not ported, tracked rather than faked:** the "Report <content>" links
//! on the spawn embeds (Java builds VRChat-Zendesk help-desk URLs through
//! `VRChatHelpDeskURLs`, a large per-category form-field builder that isn't
//! ported), and the training-mode
//! channel split (training mode is out of scope — see PORTING.md).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDateTime;
use scarlet_core::data::ScarletData;
use scarlet_core::settings::Settings;
use scarlet_core::staff_list::StaffList;
use scarlet_core::watched_entities::WatchedEntities;
use serenity::builder::{CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter, CreateMessage, EditMessage};
use serenity::model::id::{ChannelId, MessageId};
use serenity::model::Colour;

use crate::data::{DiscordConfigStore, SharedHttp};

/// `GroupAuditTypeEx`'s id/colour pairs, verbatim from the Java enum.
mod audit_type {
    pub const STAFF_JOIN: (&str, u32) = ("groupex.instance.staff.join", 0x00FF00);
    pub const STAFF_LEAVE: (&str, u32) = ("groupex.instance.staff.leave", 0xFFFF00);
    pub const USER_JOIN: (&str, u32) = ("groupex.instance.user.join", 0x007F00);
    pub const USER_LEAVE: (&str, u32) = ("groupex.instance.user.leave", 0x7F7F00);
    pub const USER_AVATAR: (&str, u32) = ("groupex.instance.user.avatar", 0xFF00FF);
    pub const USER_VIDEO: (&str, u32) = ("groupex.instance.user.video", 0x7F00FF);
    pub const VTK_START: (&str, u32) = ("groupex.instance.vtk", 0xFF7F00);
    pub const SPAWN_PEDESTAL: (&str, u32) = ("groupex.instance.user.pedestal", 0x7F007F);
    pub const SPAWN_STICKER: (&str, u32) = ("groupex.instance.user.sticker", 0x7F007F);
    pub const SPAWN_PRINT: (&str, u32) = ("groupex.instance.user.print", 0x7F007F);
    pub const SPAWN_EMOJI: (&str, u32) = ("groupex.instance.user.emoji", 0x7F007F);
    pub const SPAWN_PROP: (&str, u32) = ("groupex.instance.user.prop", 0x7F007F);
    pub const INSTANCE_INACTIVE: (&str, u32) = ("groupex.instance.inactive", 0x7F7FFF);
    pub const INSTANCE_ENFORCE: (&str, u32) = ("groupex.instance.enforce", 0xFF0000);
}

/// Java's `ScarletDiscord.FOOTER_PREFIX`.
fn footer_prefix() -> String {
    format!("{} {} \u{2727} ", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION)
}

#[derive(Clone)]
pub struct DiscordAuditEmitter {
    http: SharedHttp,
    config: Arc<DiscordConfigStore>,
    data: Arc<ScarletData>,
    settings: Arc<Settings>,
    staff_list: Arc<StaffList>,
    watched_users: Arc<WatchedEntities>,
    playerlists: Arc<tokio::sync::Mutex<HashMap<String, PlayerListState>>>,
}

#[derive(Clone)]
struct PlayerListEntry {
    user_id: String,
    display_name: String,
    joined_at: NaiveDateTime,
    avatar_name: Option<String>,
    left: bool,
}

#[derive(Clone, Default)]
struct PlayerListState {
    players: Vec<PlayerListEntry>,
    revision: u64,
}

impl DiscordAuditEmitter {
    pub fn new(
        http: SharedHttp,
        config: Arc<DiscordConfigStore>,
        data: Arc<ScarletData>,
        settings: Arc<Settings>,
        staff_list: Arc<StaffList>,
        watched_users: Arc<WatchedEntities>,
    ) -> Self {
        DiscordAuditEmitter {
            http,
            config,
            data,
            settings,
            staff_list,
            watched_users,
            playerlists: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Port of `isEmitting(GroupAuditTypeEx)`: connected, and a channel is
    /// configured for this event type.
    fn configured_channel(&self, audit_ex_id: &str, secret: bool) -> Option<ChannelId> {
        let map = if secret { "auditExType2secretChannelSf" } else { "auditExType2channelSf" };
        let raw = self.config.map_value(map, audit_ex_id)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        trimmed.parse::<u64>().ok().map(ChannelId::new)
    }

    /// Port of `condEmitEx`: resolve the destination (including the live
    /// instance embed's thread, when the instance has one in the same
    /// channel) and send.
    async fn cond_emit_ex(
        &self,
        audit_ex: (&str, u32),
        log: bool,
        secret: bool,
        location: Option<&str>,
        message: CreateMessage,
    ) {
        let Some(http) = self.http.read().await.clone() else {
            // Not connected to Discord; Java's `jda == null` guard.
            return;
        };
        let Some(mut channel_id) = self.configured_channel(audit_ex.0, secret) else { return };

        if let Some(location) = location {
            if let Some(embed_message) = self.data.live_instances_get_location_instance_embed_message(location, false) {
                let same_channel = embed_message
                    .channel_snowflake
                    .as_deref()
                    .and_then(|raw| raw.parse::<u64>().ok())
                    .is_some_and(|configured| configured == channel_id.get());
                if same_channel {
                    if let Some(thread) = embed_message.thread_snowflake.as_deref().and_then(|raw| raw.parse::<u64>().ok())
                    {
                        channel_id = ChannelId::new(thread);
                    }
                }
            }
        }

        match channel_id.send_message(&http, message).await {
            Ok(sent) => {
                if log {
                    tracing::info!("{} ({}/{})", audit_ex.0.replace('.', " "), channel_id, sent.id);
                }
            }
            Err(err) => tracing::warn!("Failed emitting {} to {}: {}", audit_ex.0, channel_id, err),
        }
    }

    fn base_embed(&self, audit_ex: (&str, u32)) -> CreateEmbed {
        CreateEmbed::new()
            .colour(Colour::new(audit_ex.1))
            .footer(CreateEmbedFooter::new(format!("{}Extended event", footer_prefix())))
            .timestamp(serenity::model::Timestamp::now())
    }

    /// The Ban/Unban row Java attaches to every user-targeted instance
    /// embed. Handled in `crate::interactions`.
    fn moderation_buttons(user_id: &str) -> CreateActionRow {
        CreateActionRow::Buttons(vec![
            CreateButton::new(format!("vrchat-user-ban:{user_id}"))
                .style(serenity::model::application::ButtonStyle::Danger)
                .label("Ban user"),
            CreateButton::new(format!("vrchat-user-unban:{user_id}"))
                .style(serenity::model::application::ButtonStyle::Success)
                .label("Unban user"),
        ])
    }

    async fn player_joined(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
    ) {
        if self.configured_channel("groupex.instance.playerlist", false).is_none() { return; }
        let mut states = self.playerlists.lock().await;
        let state = states.entry(location.to_string()).or_default();
        if let Some(player) = state.players.iter_mut().find(|player| player.user_id == user_id) {
            player.display_name = display_name.to_string();
            player.joined_at = timestamp;
            player.left = false;
        } else {
            state.players.push(PlayerListEntry {
                user_id: user_id.to_string(),
                display_name: display_name.to_string(),
                joined_at: timestamp,
                avatar_name: None,
                left: false,
            });
        }
        state.revision = state.revision.wrapping_add(1);
        let revision = state.revision;
        drop(states);
        self.schedule_playerlist_edit(location.to_string(), revision);
    }

    async fn player_left(&self, location: &str, user_id: &str) {
        if self.configured_channel("groupex.instance.playerlist", false).is_none() { return; }
        let mut states = self.playerlists.lock().await;
        let Some(state) = states.get_mut(location) else { return };
        if let Some(player) = state.players.iter_mut().find(|player| player.user_id == user_id) {
            player.left = true;
        }
        state.revision = state.revision.wrapping_add(1);
        let revision = state.revision;
        drop(states);
        self.schedule_playerlist_edit(location.to_string(), revision);
    }

    async fn player_avatar(&self, location: &str, user_id: &str, avatar_name: &str) {
        if self.configured_channel("groupex.instance.playerlist", false).is_none() { return; }
        let mut states = self.playerlists.lock().await;
        let Some(state) = states.get_mut(location) else { return };
        if let Some(player) = state.players.iter_mut().find(|player| player.user_id == user_id) {
            player.avatar_name = Some(avatar_name.to_string());
        }
        state.revision = state.revision.wrapping_add(1);
        let revision = state.revision;
        drop(states);
        self.schedule_playerlist_edit(location.to_string(), revision);
    }

    fn schedule_playerlist_edit(&self, location: String, revision: u64) {
        let emitter = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let players = {
                let states = emitter.playerlists.lock().await;
                let Some(state) = states.get(&location) else { return };
                if state.revision != revision { return; }
                state.players.clone()
            };
            emitter.flush_playerlist(&location, &players).await;
        });
    }

    async fn flush_playerlist(&self, location: &str, players: &[PlayerListEntry]) {
        let Some(instance) = self.data.live_instances_get_location_instance_embed_message(location, false) else { return };
        let (Some(thread_id), Some(message_id)) = (
            instance.thread_snowflake.as_deref().and_then(|id| id.parse::<u64>().ok()),
            instance.playerlist_message_snowflake.as_deref().and_then(|id| id.parse::<u64>().ok()),
        ) else { return };
        let Some(http) = self.http.read().await.clone() else { return };
        let embed = self.playerlist_embed(players);
        if let Err(error) = ChannelId::new(thread_id)
            .edit_message(&http, MessageId::new(message_id), EditMessage::new().embed(embed))
            .await
        {
            tracing::warn!(%error, location, "Could not update live player list");
        }
    }

    fn playerlist_embed(&self, players: &[PlayerListEntry]) -> CreateEmbed {
        let present = players.iter().filter(|player| !player.left).count();
        let left = players.len().saturating_sub(present);
        let mut description = if players.is_empty() {
            "*No players yet*".to_string()
        } else {
            let mut text = format!("**{present} present{}**\n\n", if left == 0 { String::new() } else { format!(" · {left} left") });
            for player in players.iter().filter(|player| !player.left) {
                self.push_player_line(&mut text, player, false);
            }
            if left > 0 {
                text.push_str("\n*Left:*\n");
                for player in players.iter().filter(|player| player.left) {
                    self.push_player_line(&mut text, player, true);
                }
            }
            text
        };
        if description.chars().count() > 4_000 {
            description = description.chars().take(3_999).collect::<String>() + "…";
        }
        CreateEmbed::new()
            .title("Live Player List")
            .description(description)
            .colour(Colour::new(0x007f7f))
            .timestamp(serenity::model::Timestamp::now())
    }

    fn push_player_line(&self, text: &mut String, player: &PlayerListEntry, left: bool) {
        if left { text.push_str("~~"); }
        if self.staff_list.is_id(&player.user_id) { text.push_str("⭐ "); }
        let watched = self.watched_users.get_watched_entity(&player.user_id);
        if watched.is_some() { text.push_str("⚠️ "); }
        text.push_str(&format!(
            "[{}]({}) · <t:{}:R>",
            escape_markdown(&player.display_name),
            user_link(&player.user_id),
            player.joined_at.and_utc().timestamp(),
        ));
        if let Some(avatar) = player.avatar_name.as_deref().filter(|name| !name.is_empty()) {
            text.push_str(&format!(" · *{}*", escape_markdown(avatar)));
        }
        if let Some(message) = watched.and_then(|watched| watched.message).filter(|message| !message.is_empty()) {
            text.push_str(&format!(" — {}", escape_markdown(&message)));
        }
        if left { text.push_str("~~"); }
        text.push('\n');
    }

    fn requesting_email(&self) -> Option<String> {
        // Only used by the not-yet-ported help-desk report links; kept so
        // the setting isn't silently forgotten when they land.
        let _ = &self.settings;
        None
    }

    /// Java `emitExtendedInstanceInactive`, sent after a previously tracked
    /// location disappears from `getGroupInstances`.
    pub async fn emit_instance_inactive(&self, location: &str) {
        let embed = self
            .base_embed(audit_type::INSTANCE_INACTIVE)
            .title("Instance Inactive")
            .field("Location", format!("`{location}`"), false);
        self.cond_emit_ex(
            audit_type::INSTANCE_INACTIVE,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed),
        )
        .await;
    }

    /// Java `emitExtendedInstanceEnforcement`.
    pub async fn emit_instance_enforcement(&self, location: &str, world_name: &str, reason: &str) {
        let embed = self
            .base_embed(audit_type::INSTANCE_ENFORCE)
            .title("Instance Enforcement")
            .field("Location", format!("`{location}`"), false)
            .field("World", escape_markdown(world_name), false)
            .field("Reason", reason, false);
        self.cond_emit_ex(
            audit_type::INSTANCE_ENFORCE,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed),
        )
        .await;
    }
}

/// Java wraps user text in `MarkdownSanitizer.escape`; this escapes the same
/// active characters so a display name containing `*` or `_` can't reformat
/// the embed.
fn escape_markdown(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '*' | '_' | '~' | '`' | '>' | '|' | '\\' | '[' | ']' | '(' | ')' | '#' | '-' | '+' | '.' | '!') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Java's `location.replace(":", "&instanceId=")`, which turns a location
/// into the world-launch link's query.
fn launch_link(location: &str) -> String {
    format!("https://vrchat.com/home/launch?worldId={}", location.replacen(':', "&instanceId=", 1))
}

fn user_link(user_id: &str) -> String {
    format!("https://vrchat.com/home/user/{user_id}")
}

#[async_trait::async_trait]
impl scarlet_events::AuditEmitter for DiscordAuditEmitter {
    async fn is_emitting(&self, event_type: &str) -> bool {
        self.http.read().await.is_some() && self.configured_channel(event_type, false).is_some()
    }

    async fn emit_user_join(&self, timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str) {
        self.player_joined(timestamp, location, user_id, display_name).await;
        let embed = self
            .base_embed(audit_type::USER_JOIN)
            .description(format!(
                "### [{}]({}) joined [a group instance]({})",
                escape_markdown(display_name),
                user_link(user_id),
                launch_link(location)
            ))
            .field("User ID", format!("`{user_id}`"), false);
        self.cond_emit_ex(
            audit_type::USER_JOIN,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_user_leave(&self, _timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str) {
        self.player_left(location, user_id).await;
        let embed = self
            .base_embed(audit_type::USER_LEAVE)
            .description(format!(
                "### [{}]({}) left [a group instance]({})",
                escape_markdown(display_name),
                user_link(user_id),
                launch_link(location)
            ))
            .field("User ID", format!("`{user_id}`"), false);
        self.cond_emit_ex(
            audit_type::USER_LEAVE,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_staff_join(&self, _timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str) {
        // Java's staff embeds carry no User ID field and no buttons.
        let embed = self.base_embed(audit_type::STAFF_JOIN).description(format!(
            "### [{}]({}) joined [a group instance]({})",
            escape_markdown(display_name),
            user_link(user_id),
            launch_link(location)
        ));
        self.cond_emit_ex(audit_type::STAFF_JOIN, false, false, Some(location), CreateMessage::new().embed(embed))
            .await;
    }

    async fn emit_staff_leave(&self, _timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str) {
        let embed = self.base_embed(audit_type::STAFF_LEAVE).description(format!(
            "### [{}]({}) left [a group instance]({})",
            escape_markdown(display_name),
            user_link(user_id),
            launch_link(location)
        ));
        self.cond_emit_ex(audit_type::STAFF_LEAVE, false, false, Some(location), CreateMessage::new().embed(embed))
            .await;
    }

    async fn emit_user_avatar(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        avatar_display_name: &str,
        potential_avatar_ids: &[String],
    ) {
        self.player_avatar(location, user_id, avatar_display_name).await;
        let mut embed = self
            .base_embed(audit_type::USER_AVATAR)
            .title(format!("{} switched avatars", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Avatar", escape_markdown(avatar_display_name), false);
        if !potential_avatar_ids.is_empty() {
            // Java lists the candidate ids it resolved; with avatar search
            // unported this is normally empty, and the field is then omitted
            // rather than shown blank.
            embed = embed.field(
                "Potential avatar IDs",
                potential_avatar_ids.iter().map(|id| format!("`{id}`")).collect::<Vec<_>>().join("\n"),
                false,
            );
        }
        let mut components = Vec::new();
        if !potential_avatar_ids.is_empty() && scarlet_core::features::avatar_search_enabled() {
            components.push(CreateActionRow::Buttons(vec![
                CreateButton::new("view-potential-avatar-matches").label("View avatars"),
            ]));
        }
        components.push(Self::moderation_buttons(user_id));
        self.cond_emit_ex(
            audit_type::USER_AVATAR,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(components),
        )
        .await;
    }

    async fn emit_vtk_initiated(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        target_id: Option<&str>,
        target_display_name: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
    ) {
        let target_id = target_id.unwrap_or("");
        let mut embed = self
            .base_embed(audit_type::VTK_START)
            .description(format!(
                "### [{}]({}) was targeted by a vote-to-kick in [a group instance]({})",
                escape_markdown(target_display_name),
                user_link(target_id),
                launch_link(location)
            ))
            .field("Target User ID", format!("`{target_id}`"), false);
        if let (Some(actor_id), Some(actor_name)) = (actor_id, actor_display_name) {
            embed = embed
                .author(
                    serenity::builder::CreateEmbedAuthor::new(escape_markdown(actor_name)).url(user_link(actor_id)),
                )
                .field("Initiating User ID", format!("`{actor_id}`"), false);
        }
        let mut message = CreateMessage::new().embed(embed);
        if !target_id.is_empty() {
            message = message.components(vec![Self::moderation_buttons(target_id)]);
        }
        // Java logs this one (`condEmitEx(..., true, ...)`).
        self.cond_emit_ex(audit_type::VTK_START, true, false, Some(location), message).await;
    }

    async fn emit_user_spawn_pedestal(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        content_type: &str,
        content_id: &str,
    ) {
        let lower = content_type.to_lowercase();
        // Java picks the article from the content type's first letter.
        let article = match lower.chars().next() {
            Some('a') | Some('e') | Some('i') | Some('o') | Some('u') => " spawned an ",
            _ => " spawned a ",
        };
        let mut embed = self
            .base_embed(audit_type::SPAWN_PEDESTAL)
            .title(format!("{}{}{} pedestal", escape_markdown(display_name), article, lower))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field(format!("{content_type} ID"), format!("`{content_id}`"), false);
        if content_id.starts_with("file_") {
            embed = embed.image(format!("https://api.vrchat.cloud/api/1/file/{content_id}/1/file"));
        }
        let _ = self.requesting_email();
        self.cond_emit_ex(
            audit_type::SPAWN_PEDESTAL,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_user_spawn_sticker(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        sticker_id: &str,
        file_id: Option<&str>,
    ) {
        let mut embed = self
            .base_embed(audit_type::SPAWN_STICKER)
            .title(format!("{} spawned a sticker", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Sticker ID", format!("`{sticker_id}`"), false);
        if let Some(file_id) = file_id {
            embed = embed
                .field("File ID", format!("`{file_id}`"), false)
                .image(format!("https://api.vrchat.cloud/api/1/file/{file_id}/1/file"));
        }
        self.cond_emit_ex(
            audit_type::SPAWN_STICKER,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_user_spawn_print(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        print_id: &str,
        file_id: Option<&str>,
        image_url: Option<&str>,
    ) {
        let mut embed = self
            .base_embed(audit_type::SPAWN_PRINT)
            .title(format!("{} spawned a print", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Print ID", format!("`{print_id}`"), false);
        if let Some(file_id) = file_id {
            embed = embed.field("File ID", format!("`{file_id}`"), false);
            embed = embed.image(
                image_url
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("https://api.vrchat.cloud/api/1/file/{file_id}/1/file")),
            );
        } else if let Some(image_url) = image_url {
            embed = embed.image(image_url);
        }
        self.cond_emit_ex(
            audit_type::SPAWN_PRINT,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_user_spawn_emoji(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        emoji_id: &str,
        file_id: Option<&str>,
        image_url: Option<&str>,
    ) {
        let mut embed = self
            .base_embed(audit_type::SPAWN_EMOJI)
            .title(format!("{} spawned an emoji", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Emoji ID", format!("`{emoji_id}`"), false);
        if let Some(file_id) = file_id {
            embed = embed.field("File ID", format!("`{file_id}`"), false);
            embed = embed.image(
                image_url
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("https://api.vrchat.cloud/api/1/file/{file_id}/1/file")),
            );
        } else if let Some(image_url) = image_url {
            embed = embed.image(image_url);
        }
        self.cond_emit_ex(
            audit_type::SPAWN_EMOJI,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_user_spawn_prop(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        prop_id: &str,
        author_id: Option<&str>,
        author_name: Option<&str>,
        image_url: Option<&str>,
    ) {
        let mut embed = self
            .base_embed(audit_type::SPAWN_PROP)
            .title(format!("{} spawned a prop", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Prop ID", format!("`{prop_id}`"), false);
        if let Some(author_id) = author_id {
            let label = escape_markdown(author_name.unwrap_or(author_id));
            embed = embed.field("Prop Author", format!("[{label}]({})", user_link(author_id)), false);
        }
        if let Some(image_url) = image_url {
            embed = embed.image(image_url);
        }
        self.cond_emit_ex(
            audit_type::SPAWN_PROP,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
        )
        .await;
    }

    async fn emit_user_video(
        &self,
        _timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        url: &str,
        title: &str,
    ) {
        let embed = self
            .base_embed(audit_type::USER_VIDEO)
            .title(format!("{} loaded video", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Video URL", format!("`{url}`"), false)
            .field("Video Title", escape_markdown(title), false);
        self.cond_emit_ex(audit_type::USER_VIDEO, false, false, Some(location), CreateMessage::new().embed(embed))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_links_replace_only_the_first_colon() {
        // A location is `wrld_x:1234~group(grp_y)~region(us)`; only the
        // world/instance separator becomes the query parameter.
        assert_eq!(
            launch_link("wrld_x:1234~group(grp_y)~region(us)"),
            "https://vrchat.com/home/launch?worldId=wrld_x&instanceId=1234~group(grp_y)~region(us)"
        );
    }

    #[test]
    fn display_names_cannot_inject_markdown() {
        assert_eq!(escape_markdown("**not bold**"), "\\*\\*not bold\\*\\*");
        assert_eq!(escape_markdown("plain name"), "plain name");
    }

    #[test]
    fn audit_type_ids_and_colours_match_the_java_enum() {
        assert_eq!(audit_type::USER_JOIN, ("groupex.instance.user.join", 0x007F00));
        assert_eq!(audit_type::STAFF_JOIN, ("groupex.instance.staff.join", 0x00FF00));
        assert_eq!(audit_type::VTK_START, ("groupex.instance.vtk", 0xFF7F00));
    }
}
