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
//! ported), the sticker/print/emoji/prop *lookups* that decorate those
//! embeds with images and author names (their VRChat endpoints aren't
//! ported), the live-player-list side effects, and the training-mode
//! channel split (training mode is out of scope — see PORTING.md).

use std::sync::Arc;

use chrono::NaiveDateTime;
use scarlet_core::data::ScarletData;
use scarlet_core::settings::Settings;
use serenity::builder::{CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter, CreateMessage};
use serenity::model::id::ChannelId;
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
    pub const SPAWN_PROP: (&str, u32) = ("groupex.instance.user.prop", 0x7F007F);
}

/// Java's `ScarletDiscord.FOOTER_PREFIX`.
fn footer_prefix() -> String {
    format!("{} {} \u{2727} ", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION)
}

pub struct DiscordAuditEmitter {
    http: SharedHttp,
    config: Arc<DiscordConfigStore>,
    data: Arc<ScarletData>,
    settings: Arc<Settings>,
}

impl DiscordAuditEmitter {
    pub fn new(
        http: SharedHttp,
        config: Arc<DiscordConfigStore>,
        data: Arc<ScarletData>,
        settings: Arc<Settings>,
    ) -> Self {
        DiscordAuditEmitter { http, config, data, settings }
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

    fn requesting_email(&self) -> Option<String> {
        // Only used by the not-yet-ported help-desk report links; kept so
        // the setting isn't silently forgotten when they land.
        let _ = &self.settings;
        None
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

    async fn emit_user_join(&self, _timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str) {
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
        self.cond_emit_ex(
            audit_type::USER_AVATAR,
            false,
            false,
            Some(location),
            CreateMessage::new().embed(embed).components(vec![Self::moderation_buttons(user_id)]),
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
    ) {
        let embed = self
            .base_embed(audit_type::SPAWN_STICKER)
            .title(format!("{} spawned a sticker", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Sticker ID", format!("`{sticker_id}`"), false);
        self.cond_emit_ex(
            audit_type::SPAWN_STICKER,
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
    ) {
        let embed = self
            .base_embed(audit_type::SPAWN_PROP)
            .title(format!("{} spawned a prop", escape_markdown(display_name)))
            .url(user_link(user_id))
            .field("User ID", format!("`{user_id}`"), false)
            .field("Location", format!("`{location}`"), false)
            .field("Prop ID", format!("`{prop_id}`"), false);
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
