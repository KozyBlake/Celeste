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
pub mod version;
pub mod vrchat_user_info;

use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

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
    let commands =
        vec![version::register(), config_info::register(), discord_user_info::register(), vrchat_user_info::register()];
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
        other => {
            tracing::warn!("Received unknown slash command: {}", other);
            Ok(())
        }
    };
    if let Err(err) = result {
        tracing::error!("Error handling command `{}`: {}", command.data.name, err);
    }
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
