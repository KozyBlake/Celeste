//! Command registration and dispatch. Port target: `DCommands`/`DInteractions`
//! (JDA-specific slash-command plumbing in `net.sybyline.scarlet.server.discord`)
//! and the command bodies in `ScarletDiscordCommands.java` (5,765 lines —
//! ported incrementally, one command at a time, not all at once; see
//! PORTING.md for the running inventory of what's real vs. not started).
//!
//! Simplifications shared by every command ported here so far, tracked
//! rather than hidden: no autocomplete-resolved option types (Java's
//! `@SlashOpt` on a `io.github.vrchatapi.model.User`/`GroupRole`/etc. runs a
//! live VRChat lookup as you type in Discord's UI; these commands take a
//! plain string id/snowflake option instead, since wiring autocomplete
//! interactions is a separate feature not built yet), and no button-based
//! paginated embeds (Java's `discord.interactions.Pagination`; long output
//! here is split into multiple plain messages via [`paginate_on_lines`]
//! instead, a direct port of `MiscUtils.paginateOnLines`).

pub mod config_info;
pub mod discord_user_info;
pub mod moderation_tags;
pub mod secret_staff_list;
pub mod staff_list;
pub mod version;
pub mod vrchat_group;
pub mod vrchat_search;
pub mod vrchat_user_ban;
pub mod vrchat_user_info;
pub mod vrchat_user_unban;
pub mod watched_group;

use std::sync::Arc;

use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;
use serenity::model::mention::Mentionable;

/// Acks the interaction immediately without a visible message yet ("Scarlet
/// is thinking..."), then lets the handler take as long as it needs (a
/// VRChat API call, a paginated multi-message reply) without racing
/// Discord's ~3 second ack window on the initial response. Every command
/// that does any I/O calls this first, then reports its result with
/// [`send_pages`]/[`reply_ephemeral`] (both post-defer followups, not a
/// second initial response).
pub(crate) async fn defer(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    command.create_response(&ctx.http, CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new())).await
}

/// Sends each already-[`paginate_on_lines`]-split page as its own
/// follow-up message, in order. Must be called after [`defer`].
pub(crate) async fn send_pages(ctx: &Context, command: &CommandInteraction, pages: Vec<String>) -> serenity::Result<()> {
    for page in pages {
        command.create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(page)).await?;
    }
    Ok(())
}

/// A short, ephemeral (only-you-can-see-it) error/status reply. Must be
/// called after [`defer`].
pub(crate) async fn reply_ephemeral(ctx: &Context, command: &CommandInteraction, message: &str) -> serenity::Result<()> {
    command
        .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(message).ephemeral(true))
        .await
        .map(|_| ())
}

/// Registers every ported slash command as a global command. Global
/// registration can take up to an hour to propagate on Discord's side;
/// swap to per-guild registration during development if faster iteration
/// is needed (not done here to keep this skeleton simple).
pub async fn register_global_commands(ctx: &Context) {
    let commands = vec![
        version::register(),
        config_info::register(),
        discord_user_info::register(),
        vrchat_user_info::register(),
        vrchat_user_ban::register(),
        vrchat_user_unban::register(),
        vrchat_group::register(),
        vrchat_search::register(),
        staff_list::register(),
        secret_staff_list::register(),
        moderation_tags::register(),
        watched_group::register(),
    ];
    if let Err(err) = serenity::model::application::Command::set_global_commands(&ctx.http, commands).await {
        tracing::error!("Failed to register global slash commands: {}", err);
    }
}

/// Dispatches an incoming command interaction to its handler by name.
pub async fn dispatch(ctx: &Context, command: &CommandInteraction) {
    let result = match command.data.name.as_str() {
        version::NAME => version::run(ctx, command).await,
        config_info::NAME => config_info::run(ctx, command).await,
        discord_user_info::NAME => discord_user_info::run(ctx, command).await,
        vrchat_user_info::NAME => vrchat_user_info::run(ctx, command).await,
        vrchat_user_ban::NAME => vrchat_user_ban::run(ctx, command).await,
        vrchat_user_unban::NAME => vrchat_user_unban::run(ctx, command).await,
        vrchat_group::NAME => vrchat_group::run(ctx, command).await,
        vrchat_search::NAME => vrchat_search::run(ctx, command).await,
        staff_list::NAME => staff_list::run(ctx, command).await,
        secret_staff_list::NAME => secret_staff_list::run(ctx, command).await,
        moderation_tags::NAME => moderation_tags::run(ctx, command).await,
        watched_group::NAME => watched_group::run(ctx, command).await,
        other => {
            tracing::warn!("Received unknown slash command: {}", other);
            Ok(())
        }
    };
    if let Err(err) = result {
        tracing::error!("Error handling command `{}`: {}", command.data.name, err);
    }
}

/// Everything a VRChat moderation-action command needs, gathered and
/// permission-checked in one place since every such command in
/// `ScarletDiscordCommands.java` repeats the same preamble: confirm the
/// invoking Discord member has a linked VRChat account
/// (`globalMetadata_getSnowflakeId`), then confirm that account holds the
/// given VRChat group permission (`checkMemberHasVRChatPermission`).
///
/// **Not ported**: Java's `checkMemberHasScarletPermission` fallback (a
/// per-server override that can grant a Scarlet-specific permission
/// without needing the matching VRChat group role -- the role-to-Discord
/// mapping config it depends on isn't ported, see PORTING.md) and
/// `pendingModActions` (a dedup lock preventing two concurrent actions on
/// the same target). Returns `Err(message)` with a ready-to-send ephemeral
/// reply on the first failed precondition.
pub(crate) struct ModerationContext {
    pub client: Arc<scarlet_vrchat::client::VrChatClient>,
    pub group_id: String,
}

/// Resolves the shared VRChat client and the configured group id, with no
/// actor/permission check — for commands like `add-role`/`remove-role`
/// whose Java originals (`VrchatGroups.addRole`/`removeRole`) don't call
/// `checkMemberHasVRChatPermission` either, relying solely on Discord's own
/// per-command permission settings (`@DefaultPerms`). Faithfully porting
/// "no extra in-code gate" means not inventing one here.
pub(crate) async fn vrchat_context(ctx: &Context) -> Result<ModerationContext, String> {
    let settings = { ctx.data.read().await.get::<crate::data::SettingsKey>().cloned() }
        .ok_or_else(|| "Settings are not available.".to_string())?;
    let vrchat_slot = { ctx.data.read().await.get::<crate::data::VrChatClientKey>().cloned() }
        .ok_or_else(|| "Scarlet isn't logged in to VRChat right now.".to_string())?;
    let client = { vrchat_slot.read().await.clone() }.ok_or_else(|| "Scarlet isn't logged in to VRChat right now.".to_string())?;

    let group_id: Option<String> = settings.store.get_object(scarlet_vrchat::client::GROUP_ID_KEY);
    let group_id = group_id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "No VRChat group is configured (run Scarlet's setup wizard, or set `vrchat_group_id`).".to_string())?;

    Ok(ModerationContext { client, group_id })
}

/// [`vrchat_context`] plus the linked-account and VRChat-group-permission
/// checks that ban/unban's Java originals actually perform
/// (`globalMetadata_getSnowflakeId` then `checkMemberHasVRChatPermission`).
pub(crate) async fn moderation_context(
    ctx: &Context,
    command: &CommandInteraction,
    permission: &str,
) -> Result<ModerationContext, String> {
    let scarlet_data = { ctx.data.read().await.get::<crate::data::ScarletDataKey>().cloned() }
        .ok_or_else(|| "Scarlet data is not available.".to_string())?;
    let context = vrchat_context(ctx).await?;

    let actor_vrc_id = scarlet_data.global_get_snowflake_id(&command.user.id.to_string()).ok_or_else(|| {
        format!(
            "You must have linked ids to perform this action.\n(Use `/staff-list add` `vrchat-user`:*<your VRChat user ID>* `discord-user`:{} to accomplish this)",
            command.user.mention()
        )
    })?;

    let has_permission = context
        .client
        .user_has_group_permission(&context.group_id, &actor_vrc_id, permission)
        .await
        .map_err(|err| format!("Could not check your VRChat group permissions: {err}"))?;
    if !has_permission {
        return Err("You do not have permission to perform this action.\n||(Your admin can grant your linked VRChat account the relevant group permission.)||"
            .to_string());
    }

    Ok(context)
}

/// Port of `MiscUtils.paginateOnLines`: splits `input` into chunks of at
/// most `max_component_len` characters each, preferring to break on the
/// last newline within a chunk (so a page doesn't cut a line in half)
/// rather than always hard-splitting at the length limit. Used to fit long
/// command output under Discord's 2000-character message limit.
///
/// Java counts UTF-16 code units; this counts `char`s instead (there's no
/// meaningful UTF-16 concept in Rust) and always breaks on a `char`
/// boundary rather than a raw byte offset, since slicing a `&str` on a
/// non-boundary byte offset panics — a real correctness difference from a
/// naive byte-offset port, not just a style choice.
pub fn paginate_on_lines(input: &str, max_component_len: usize) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut parts = Vec::new();
    let mut start = 0usize;
    while start < len {
        let end = (start + max_component_len).min(len);
        if end >= len {
            parts.push(chars[start..].iter().collect::<String>().trim().to_string());
            start = len;
            continue;
        }
        // Search backwards from `end` (exclusive) for the last newline,
        // matching Java's `lastIndexOf("\n", end)` + `lastNewline < end`
        // (a newline exactly at `end` doesn't count -- that's the char
        // about to start the next page, not one that ends this page).
        match chars[start..end].iter().rposition(|&c| c == '\n') {
            Some(rel) if rel > 0 => {
                let last_newline = start + rel;
                parts.push(chars[start..last_newline].iter().collect::<String>().trim().to_string());
                start = last_newline + 1;
            }
            _ => {
                parts.push(chars[start..end].iter().collect::<String>().trim().to_string());
                start = end;
            }
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_input_is_a_single_page() {
        assert_eq!(paginate_on_lines("hello world", 2000), vec!["hello world".to_string()]);
    }

    #[test]
    fn splits_exactly_on_a_newline_at_the_limit() {
        let pages = paginate_on_lines("aaa\nbbb", 4);
        assert_eq!(pages, vec!["aaa".to_string(), "bbb".to_string()]);
    }

    #[test]
    fn prefers_the_latest_newline_within_the_window_not_the_first() {
        // Both newlines fall within the first 18-char window; the page
        // should fill up to the *later* one rather than stopping at the
        // first opportunity, maximizing how much fits per message.
        let input = "line one\nline two\nline three";
        let pages = paginate_on_lines(input, 18);
        assert_eq!(pages, vec!["line one\nline two".to_string(), "line three".to_string()]);
    }

    #[test]
    fn hard_splits_when_theres_no_newline_to_break_on() {
        let input = "a".repeat(10);
        let pages = paginate_on_lines(&input, 4);
        assert_eq!(pages, vec!["aaaa", "aaaa", "aa"]);
    }

    #[test]
    fn never_panics_on_a_multi_byte_character_near_the_boundary() {
        // Each "é" is 2 bytes in UTF-8; a naive byte-offset port would slice
        // mid-character here and panic. Also covers a boundary that lands
        // exactly on a multi-byte char rather than between two of them.
        let input = "é".repeat(20);
        let pages = paginate_on_lines(&input, 7);
        assert_eq!(pages.join(""), input);
    }

    #[test]
    fn empty_input_produces_no_pages() {
        assert!(paginate_on_lines("", 2000).is_empty());
    }
}
