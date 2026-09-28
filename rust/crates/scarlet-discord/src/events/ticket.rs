//! Port of `ScarletDiscordJDA`'s Ticket Tool auto-response:
//! `handleChannelCreate`, the ticket half of `handleMessageReceived`,
//! `isTicketToolAutoResponseCandidate`, `tryTicketToolAutoResponse`,
//! `sendTicketToolAutoResponse` and the three `inferTicketToolMember*`
//! helpers.
//!
//! When Ticket Tool opens a ticket channel, Celeste posts the configured
//! age-verification instructions into it, addressed to whoever opened the
//! ticket (and optionally pinging a staff role). Both triggers Java has are
//! ported: the channel being created, and the first message arriving in a
//! recently-created channel — the second exists because a channel can be
//! created before its permission overrides identify the opener, and because
//! Celeste may have been offline when the channel appeared.
//!
//! Configuration comes from the same `discord_bot.json` fields the
//! `/set-ticket-tool-auto-response` and `/set-ticket-age-verify-message`
//! commands already write, so a configuration made under the Java build is
//! honoured here unchanged.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use serenity::builder::{CreateMessage, GetMessages};
use serenity::client::Context;
use serenity::model::channel::{Channel, GuildChannel, Message};
use serenity::model::id::{ChannelId, GuildId, UserId};
use serenity::model::permissions::Permissions;

use crate::data::{DiscordConfigKey, SettingsKey};

/// Java's `TICKET_TOOL_AUTO_RESPONSE_MAX_CHANNEL_AGE`: a message only
/// triggers the auto-response while its channel is still this new, so an
/// ordinary message in an old ticket doesn't re-trigger it.
const MAX_CHANNEL_AGE: chrono::Duration = chrono::Duration::minutes(30);

/// Java delays the channel-create path, because Ticket Tool creates the
/// channel first and applies the opener's permission override (and posts
/// its own opening message) a moment later.
const CHANNEL_CREATE_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Port of `ticketToolAutoRespondedChannels`: channels already responded in,
/// so neither trigger can double-post. Java holds this on the JDA client
/// instance; this is the same per-process set.
fn responded_channels() -> &'static Mutex<HashSet<ChannelId>> {
    static RESPONDED: OnceLock<Mutex<HashSet<ChannelId>>> = OnceLock::new();
    RESPONDED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The ticket configuration, read fresh per event so a `/set-...` change
/// takes effect immediately — as it does in Java, where the handler reads
/// the live fields.
struct TicketConfig {
    enabled: bool,
    guild_id: Option<GuildId>,
    category_id: Option<ChannelId>,
    channel_name_pattern: Option<regex::Regex>,
    notify_role_id: Option<String>,
    message: String,
}

impl TicketConfig {
    async fn load(ctx: &Context) -> Option<TicketConfig> {
        let data = ctx.data.read().await;
        let config = data.get::<DiscordConfigKey>()?.clone();
        let settings = data.get::<SettingsKey>()?.clone();
        drop(data);

        let enabled = config
            .json("ticketToolAutoResponseEnabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let guild_id = scarlet_core::settings::registry::registry_string_encrypted(
            settings.encrypted.clone(),
            "discord.guild_id",
        )
        .get_or_null()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(GuildId::new);
        let category_id = config
            .json("ticketToolCategorySf")
            .and_then(|value| value.as_str().map(str::to_string))
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .map(ChannelId::new);
        // An invalid stored regex disables name matching rather than
        // matching everything, which is the safe direction for something
        // that posts a message.
        let channel_name_pattern = config
            .json("ticketToolChannelNameRegex")
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|raw| !raw.trim().is_empty())
            .and_then(|raw| match regex::Regex::new(&raw) {
                Ok(pattern) => Some(pattern),
                Err(err) => {
                    tracing::warn!("Ticket Tool channel-name regex is invalid, ignoring it: {err}");
                    None
                }
            });
        let notify_role_id = config
            .json("ticketToolNotifyRoleSf")
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|raw| !raw.trim().is_empty());
        let message = config
            .json("ticketToolAgeVerificationMessage")
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| crate::commands::ticket_config::DEFAULT_MESSAGE.to_string());

        Some(TicketConfig { enabled, guild_id, category_id, channel_name_pattern, notify_role_id, message })
    }

    /// Port of `isTicketToolAutoResponseCandidate`.
    fn is_candidate(&self, channel: &GuildChannel) -> bool {
        if !self.enabled {
            return false;
        }
        if self.guild_id != Some(channel.guild_id) {
            return false;
        }
        let category_match = self.category_id.is_some() && channel.parent_id == self.category_id;
        let name_match =
            self.channel_name_pattern.as_ref().is_some_and(|pattern| full_match(pattern, &channel.name));
        category_match || name_match
    }
}

/// Java's `Matcher.matches()` requires the *whole* string to match, while
/// Rust's `is_match` is a search. Anchoring keeps a pattern like `ticket-.*`
/// from also matching `old-ticket-99`.
fn full_match(pattern: &regex::Regex, value: &str) -> bool {
    pattern.find(value).is_some_and(|found| found.start() == 0 && found.end() == value.len())
}

/// Port of `handleChannelCreate`.
pub async fn on_channel_create(ctx: &Context, channel: &GuildChannel) {
    let Some(config) = TicketConfig::load(ctx).await else { return };
    if !config.is_candidate(channel) {
        return;
    }
    // Ticket Tool applies the opener's permission override just after
    // creating the channel, so give it a moment before trying to work out
    // who opened it.
    let ctx = ctx.clone();
    let channel = channel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(CHANNEL_CREATE_DELAY).await;
        try_auto_response(&ctx, &channel, None, "channel create").await;
    });
}

/// Port of the ticket half of `handleMessageReceived`.
///
/// The other half of that method (capturing a moderator's first message in
/// a moderation thread as the audit entry's description) depends on
/// `moderationThreadSf2auditId`, which belongs to the not-yet-ported
/// moderation-thread tracking — see PORTING.md.
pub async fn on_message(ctx: &Context, message: &Message) {
    let Some(guild_id) = message.guild_id else { return };
    if message.author.id == ctx.cache.current_user().id {
        return;
    }
    let Some(config) = TicketConfig::load(ctx).await else { return };
    if config.guild_id != Some(guild_id) {
        return;
    }

    let Some(channel) = guild_channel(ctx, message.channel_id).await else { return };
    // Serenity dates come from the `time` crate, chrono elsewhere in the
    // workspace; compare as unix seconds rather than converting types.
    let created_unix = message.channel_id.created_at().unix_timestamp();
    let age_seconds = chrono::Utc::now().timestamp() - created_unix;
    if age_seconds > MAX_CHANNEL_AGE.num_seconds() {
        return;
    }
    if !config.is_candidate(&channel) {
        return;
    }

    let opener = infer_opener_from_message(message);
    try_auto_response(ctx, &channel, opener, "message received").await;
}

/// Port of `tryTicketToolAutoResponse`.
async fn try_auto_response(ctx: &Context, channel: &GuildChannel, opener: Option<UserId>, source: &str) {
    let Some(config) = TicketConfig::load(ctx).await else { return };
    if !config.is_candidate(channel) {
        return;
    }
    if responded_channels().lock().unwrap().contains(&channel.id) {
        return;
    }

    let mut opener = match opener {
        Some(user) if !is_bot(ctx, channel.guild_id, user).await => Some(user),
        _ => None,
    };
    let mut source = source.to_string();
    if opener.is_none() {
        opener = infer_opener_from_overrides(ctx, channel).await;
    }
    if opener.is_none() {
        // Last resort: read the start of the channel's history, which is
        // where Ticket Tool's own opening message mentions the opener.
        match channel.id.messages(&ctx.http, GetMessages::new().limit(10)).await {
            Ok(mut history) => {
                // `messages` returns newest-first; Java asks for the
                // beginning of the channel, so walk oldest-first.
                history.reverse();
                opener = history.iter().find_map(infer_opener_from_message);
                source = format!("{source} history");
            }
            Err(err) => {
                tracing::debug!(
                    "Ticket Tool auto-response could not inspect history for {} ({}): {err}",
                    channel.name,
                    channel.id
                );
                return;
            }
        }
    }

    let Some(opener) = opener else {
        tracing::debug!(
            "Ticket Tool auto-response skipped in {} ({}): opener could not be inferred",
            channel.name,
            channel.id
        );
        return;
    };
    send_auto_response(ctx, channel, opener, &config, &source).await;
}

/// Port of `sendTicketToolAutoResponse`.
async fn send_auto_response(
    ctx: &Context,
    channel: &GuildChannel,
    opener: UserId,
    config: &TicketConfig,
    source: &str,
) {
    if is_bot(ctx, channel.guild_id, opener).await {
        return;
    }
    // Claim the channel before sending, so two triggers racing can't both
    // post; released again if the send fails, exactly as Java does.
    if !responded_channels().lock().unwrap().insert(channel.id) {
        return;
    }

    let notify = match &config.notify_role_id {
        Some(role) => format!(" <@&{}>", role.trim()),
        None => String::new(),
    };
    let content = format!("<@{opener}>{notify}\n\n{}", config.message);

    match channel.id.send_message(&ctx.http, CreateMessage::new().content(content)).await {
        Ok(_) => tracing::info!(
            "Ticket Tool auto-response sent in {} ({}) for opener {} via {}",
            channel.name,
            channel.id,
            opener,
            source
        ),
        Err(err) => {
            responded_channels().lock().unwrap().remove(&channel.id);
            tracing::warn!("Ticket Tool auto-response failed in {} ({}): {err}", channel.name, channel.id);
        }
    }
}

/// Port of `inferTicketToolMemberFromOverrides`: the member Ticket Tool
/// granted a per-user "view channel" override to is the ticket's opener.
async fn infer_opener_from_overrides(ctx: &Context, channel: &GuildChannel) -> Option<UserId> {
    for overwrite in &channel.permission_overwrites {
        let serenity::model::channel::PermissionOverwriteType::Member(user) = overwrite.kind else {
            continue;
        };
        if !overwrite.allow.contains(Permissions::VIEW_CHANNEL) {
            continue;
        }
        if is_bot(ctx, channel.guild_id, user).await {
            continue;
        }
        // Java takes the first candidate whether there is one or several.
        return Some(user);
    }
    None
}

/// Port of `inferTicketToolMemberFromMessage`: a non-bot mention first (the
/// opener Ticket Tool's own message pings), else the non-bot author.
fn infer_opener_from_message(message: &Message) -> Option<UserId> {
    if let Some(mentioned) = message.mentions.iter().find(|user| !user.bot) {
        return Some(mentioned.id);
    }
    if message.author.bot {
        return None;
    }
    Some(message.author.id)
}

async fn guild_channel(ctx: &Context, channel_id: ChannelId) -> Option<GuildChannel> {
    match channel_id.to_channel(&ctx.http).await {
        Ok(Channel::Guild(channel)) => Some(channel),
        _ => None,
    }
}

async fn is_bot(ctx: &Context, guild_id: GuildId, user_id: UserId) -> bool {
    if let Some(user) = ctx.cache.user(user_id) {
        return user.bot;
    }
    match guild_id.member(&ctx.http, user_id).await {
        Ok(member) => member.user.bot,
        // Unknown: treat as not-a-bot, matching Java's `getMember()`
        // returning a member whose user simply isn't flagged. Sending the
        // instructions to a human who didn't open the ticket is a smaller
        // failure than never sending them at all.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_name_matching_is_anchored_like_javas_matches() {
        let pattern = regex::Regex::new("ticket-.*").unwrap();
        assert!(full_match(&pattern, "ticket-0042"));
        // `Matcher.matches()` would reject this; a bare `is_match` would not.
        assert!(!full_match(&pattern, "old-ticket-0042"));
        assert!(!full_match(&pattern, "general"));
    }

    #[test]
    fn the_max_channel_age_matches_java() {
        assert_eq!(MAX_CHANNEL_AGE, chrono::Duration::minutes(30));
    }
}
