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
pub mod config_set;
pub mod discord_user_info;
pub mod discord_warn;
pub mod discord_member_action;
pub mod export_log;
pub mod set_event_channel;
pub mod set_tts_voice;
pub mod set_voice_channel;
pub mod server_config;
pub mod server_restart;
pub mod settings;
pub mod ticket_config;
pub mod aux_webhooks;
pub mod animated_emoji;
pub mod moderation_tags;
pub mod tag;
pub mod moderation_history;
pub mod moderation_reports;
pub mod link_vrchat_account;
pub mod secret_staff_list;
pub mod staff_list;
pub mod unlink_vrchat_account;
pub mod unlink_vrchat_account_for;
pub mod version;
pub mod vrchat_group;
pub mod vrchat_search;
pub mod vrchat_user_ban;
pub mod vrchat_user_info;
pub mod vrchat_user_multi;
pub mod vrchat_user_unban;
pub mod watched_avatar;
pub mod watched_entity;
pub mod watched_group;
pub mod watched_user;

use std::sync::Arc;

use serenity::builder::{
    CreateActionRow, CreateInteractionResponse, CreateInteractionResponseFollowup,
    CreateInteractionResponseMessage, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption,
};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

/// Acks the interaction immediately without a visible message yet ("Celeste
/// is thinking..."), then lets the handler take as long as it needs (a
/// VRChat API call, a paginated multi-message reply) without racing
/// Discord's ~3 second ack window on the initial response. Every command
/// that does any I/O calls this first, then reports its result with
/// [`send_pages`]/[`reply_ephemeral`] (both post-defer followups, not a
/// second initial response).
pub(crate) async fn defer(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
        )
        .await
}

/// Sends each already-[`paginate_on_lines`]-split page as its own
/// follow-up message, in order. Must be called after [`defer`].
pub(crate) async fn send_pages(
    ctx: &Context,
    command: &CommandInteraction,
    pages: Vec<String>,
) -> serenity::Result<()> {
    for page in pages {
        command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new().content(page),
            )
            .await?;
    }
    Ok(())
}

/// A short, ephemeral (only-you-can-see-it) error/status reply. Must be
/// called after [`defer`].
pub(crate) async fn reply_ephemeral(
    ctx: &Context,
    command: &CommandInteraction,
    message: &str,
) -> serenity::Result<()> {
    command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(message)
                .ephemeral(true),
        )
        .await
        .map(|_| ())
}

/// Shared Java-style watched-entry removal picker. Discord permits five
/// action rows with 25 options each, so the first 125 entries are shown.
pub(crate) async fn watched_remove_menu(
    ctx: &Context,
    command: &CommandInteraction,
    kind: &str,
    plural: &str,
    entries: Vec<(String, String)>,
) -> serenity::Result<()> {
    let total = entries.len();
    if total == 0 {
        return reply_ephemeral(
            ctx,
            command,
            &format!("There are no watched {plural} to remove."),
        )
        .await;
    }
    let mut rows = Vec::new();
    for (index, chunk) in entries
        .into_iter()
        .take(125)
        .collect::<Vec<_>>()
        .chunks(25)
        .enumerate()
    {
        let options = chunk
            .iter()
            .map(|(id, name)| {
                let label: String = name.chars().take(100).collect();
                CreateSelectMenuOption::new(label, id)
            })
            .collect();
        rows.push(CreateActionRow::SelectMenu(
            CreateSelectMenu::new(
                format!("watched-remove:{kind}:{index}"),
                CreateSelectMenuKind::String { options },
            )
            .placeholder(format!("Select watched {plural} to remove"))
            .min_values(1)
            .max_values(chunk.len() as u8),
        ));
    }
    let note = if total > 125 {
        format!("\nShowing the first 125 of {total}; remove some and run this again for the rest.")
    } else {
        String::new()
    };
    command.create_followup(&ctx.http, CreateInteractionResponseFollowup::new()
        .content(format!("Select watched {plural} to remove — they'll be deleted from the list immediately.{note}"))
        .components(rows).ephemeral(true)).await.map(|_| ())
}

/// Registers every ported slash command as a global command. Global
/// registration can take up to an hour to propagate on Discord's side;
/// swap to per-guild registration during development if faster iteration
/// is needed (not done here to keep this skeleton simple).
/// Removes any guild-scoped copies of Celeste's commands from the
/// configured guild.
///
/// Both Java (`jda.upsertCommand`) and this port register commands
/// **globally**, so a guild-scoped copy can only be a leftover from an
/// older build that registered them per-guild. Discord lists guild and
/// global commands side by side, which is why the server showed two of
/// everything — two `/config-info`, two `/discord-ban`, and so on.
///
/// This runs on every connect and is a no-op once the guild is clean.
pub async fn clear_stale_guild_commands(ctx: &Context) {
    let guild_id = {
        let data = ctx.data.read().await;
        let Some(settings) = data.get::<crate::data::SettingsKey>().cloned() else { return };
        drop(data);
        scarlet_core::settings::registry::registry_string_encrypted(settings.encrypted.clone(), "discord.guild_id")
            .get_or_null()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .map(serenity::model::id::GuildId::new)
    };
    let Some(guild_id) = guild_id else { return };

    match guild_id.get_commands(&ctx.http).await {
        Ok(existing) if existing.is_empty() => {}
        Ok(existing) => {
            let names: Vec<&str> = existing.iter().map(|command| command.name.as_str()).collect();
            tracing::info!(
                "Removing {} stale guild-scoped command(s) duplicating the global set: {}",
                existing.len(),
                names.join(", ")
            );
            if let Err(err) = guild_id.set_commands(&ctx.http, Vec::new()).await {
                tracing::warn!("Could not clear guild-scoped commands: {err}");
            }
        }
        Err(err) => tracing::debug!("Could not list guild-scoped commands: {err}"),
    }
}

/// Every slash command this build registers.
///
/// Split out from [`register_global_commands`] so the command surface
/// can be inspected without a Discord connection — `command_surface_matches_java`
/// below checks it against the Java build's own inventory, so a missing
/// command is something the test names rather than something you notice
/// in the client.
pub fn all_commands() -> Vec<serenity::builder::CreateCommand> {
    let mut commands = vec![
        version::register(),
        config_info::register(),
        config_set::register(),
        discord_user_info::register(),
        vrchat_user_info::register(),
        vrchat_user_ban::register(),
        vrchat_user_multi::register(true),
        vrchat_user_unban::register(),
        vrchat_user_multi::register(false),
        vrchat_group::register(),
        vrchat_search::register(),
        staff_list::register(),
        secret_staff_list::register(),
        moderation_tags::register(),
        tag::register(),
        moderation_history::register_moderation_log(),
        moderation_history::register_query_target(),
        moderation_history::register_query_actor(),
        moderation_history::register_actor_summary(),
        moderation_reports::register_summary(),
        moderation_reports::register_outstanding(),
        link_vrchat_account::register(),
        watched_group::register(),
        watched_user::register(),
        watched_avatar::register(),
        unlink_vrchat_account::register(),
        unlink_vrchat_account_for::register(),
        discord_warn::register(),
        discord_member_action::register(discord_member_action::MemberAction::Ban),
        discord_member_action::register(discord_member_action::MemberAction::Kick),
        export_log::register(),
        set_event_channel::register(set_event_channel::RouteKind::Audit),
        set_event_channel::register(set_event_channel::RouteKind::Extended),
        set_event_channel::register(set_event_channel::RouteKind::SecretAudit),
        set_event_channel::register(set_event_channel::RouteKind::SecretExtended),
        set_tts_voice::register(),
        set_voice_channel::register(),
        aux_webhooks::register(),
        aux_webhooks::register_assignment(),
        server_config::register_channel(server_config::ChannelSetting::Training),
        server_config::register_channel(server_config::ChannelSetting::OpsAlert),
        server_config::register_channel(server_config::ChannelSetting::ActionLog),
        server_config::register_account_age(),
        server_config::register_auto_invite(),
        server_restart::register(),
        settings::register(),
        ticket_config::register_auto_response(),
        ticket_config::register_message(),
    ];
    if scarlet_core::features::animated_emoji_enabled() {
        commands.push(animated_emoji::register());
        commands.push(animated_emoji::register_message());
    }
    commands
}

pub async fn register_global_commands(ctx: &Context) {
    let commands = all_commands();

    if let Err(err) =
        serenity::model::application::Command::set_global_commands(&ctx.http, commands).await
    {
        tracing::error!("Failed to register global slash commands: {}", err);
    }
}

/// Dispatches an incoming command interaction to its handler by name.
pub async fn dispatch(ctx: &Context, command: &CommandInteraction) {
    let result = match command.data.name.as_str() {
        version::NAME => version::run(ctx, command).await,
        config_info::NAME => config_info::run(ctx, command).await,
        config_set::NAME => config_set::run(ctx, command).await,
        discord_user_info::NAME => discord_user_info::run(ctx, command).await,
        vrchat_user_info::NAME => vrchat_user_info::run(ctx, command).await,
        vrchat_user_ban::NAME => vrchat_user_ban::run(ctx, command).await,
        vrchat_user_multi::BAN_NAME => vrchat_user_multi::run(ctx, command, true).await,
        vrchat_user_unban::NAME => vrchat_user_unban::run(ctx, command).await,
        vrchat_user_multi::UNBAN_NAME => vrchat_user_multi::run(ctx, command, false).await,
        vrchat_group::NAME => vrchat_group::run(ctx, command).await,
        vrchat_search::NAME => vrchat_search::run(ctx, command).await,
        staff_list::NAME => staff_list::run(ctx, command).await,
        secret_staff_list::NAME => secret_staff_list::run(ctx, command).await,
        moderation_tags::NAME => moderation_tags::run(ctx, command).await,
        tag::NAME => tag::run(ctx, command).await,
        moderation_history::MODERATION_LOG_NAME => moderation_history::run_moderation_log(ctx, command).await,
        moderation_history::QUERY_TARGET_NAME => moderation_history::run_query_target(ctx, command).await,
        moderation_history::QUERY_ACTOR_NAME => moderation_history::run_query_actor(ctx, command).await,
        moderation_history::ACTOR_SUMMARY_NAME => moderation_history::run_actor_summary(ctx, command).await,
        moderation_reports::SUMMARY_NAME => moderation_reports::run_summary(ctx, command).await,
        moderation_reports::OUTSTANDING_NAME => moderation_reports::run_outstanding(ctx, command).await,
        link_vrchat_account::NAME => link_vrchat_account::run(ctx, command).await,
        watched_group::NAME => watched_group::run(ctx, command).await,
        watched_user::NAME => watched_user::run(ctx, command).await,
        watched_avatar::NAME => watched_avatar::run(ctx, command).await,
        unlink_vrchat_account::NAME => unlink_vrchat_account::run(ctx, command).await,
        unlink_vrchat_account_for::NAME => unlink_vrchat_account_for::run(ctx, command).await,
        discord_warn::NAME => discord_warn::run(ctx, command).await,
        discord_member_action::BAN_NAME => discord_member_action::run(ctx, command, discord_member_action::MemberAction::Ban).await,
        discord_member_action::KICK_NAME => discord_member_action::run(ctx, command, discord_member_action::MemberAction::Kick).await,
        export_log::NAME => export_log::run(ctx, command).await,
        set_event_channel::AUDIT_NAME => set_event_channel::run(ctx, command, set_event_channel::RouteKind::Audit).await,
        set_event_channel::EXTENDED_NAME => set_event_channel::run(ctx, command, set_event_channel::RouteKind::Extended).await,
        set_event_channel::SECRET_AUDIT_NAME => set_event_channel::run(ctx, command, set_event_channel::RouteKind::SecretAudit).await,
        set_event_channel::SECRET_EXTENDED_NAME => set_event_channel::run(ctx, command, set_event_channel::RouteKind::SecretExtended).await,
        set_tts_voice::NAME => set_tts_voice::run(ctx, command).await,
        set_voice_channel::NAME => set_voice_channel::run(ctx, command).await,
        aux_webhooks::NAME => aux_webhooks::run(ctx, command).await,
        aux_webhooks::ASSIGN_NAME => aux_webhooks::run_assignment(ctx, command).await,
        server_config::TRAINING_NAME => server_config::run_channel(ctx, command, server_config::ChannelSetting::Training).await,
        server_config::OPS_ALERT_NAME => server_config::run_channel(ctx, command, server_config::ChannelSetting::OpsAlert).await,
        server_config::ACTION_LOG_NAME => server_config::run_channel(ctx, command, server_config::ChannelSetting::ActionLog).await,
        server_config::ACCOUNT_AGE_NAME => server_config::run_account_age(ctx, command).await,
        server_config::AUTO_INVITE_NAME => server_config::run_auto_invite(ctx, command).await,
        server_restart::NAME => server_restart::run(ctx, command).await,
        settings::NAME => settings::run(ctx, command).await,
        ticket_config::AUTO_RESPONSE_NAME => ticket_config::run_auto_response(ctx, command).await,
        ticket_config::MESSAGE_NAME => ticket_config::run_message(ctx, command).await,
        animated_emoji::NAME => animated_emoji::run(ctx, command).await,
        other => {
            tracing::warn!("Received unknown slash command: {}", other);
            Ok(())
        }
    };
    if let Err(err) = result {
        tracing::error!("Error handling command `{}`: {}", command.data.name, err);
    }
}

pub async fn dispatch_autocomplete(ctx: &Context, command: &CommandInteraction) {
    let result = match command.data.name.as_str() {
        settings::NAME => settings::autocomplete(ctx, command).await,
        tag::NAME => tag::autocomplete(ctx, command).await,
        _ => command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Autocomplete(
                    serenity::builder::CreateAutocompleteResponse::new(),
                ),
            )
            .await,
    };
    if let Err(err) = result {
        tracing::error!(command = %command.data.name, "Error handling autocomplete: {err}");
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
/// per-server override that can grant a Celeste-specific permission
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
    let settings = {
        ctx.data
            .read()
            .await
            .get::<crate::data::SettingsKey>()
            .cloned()
    }
    .ok_or_else(|| "Settings are not available.".to_string())?;
    let vrchat_slot = {
        ctx.data
            .read()
            .await
            .get::<crate::data::VrChatClientKey>()
            .cloned()
    }
    .ok_or_else(|| "Celeste isn't logged in to VRChat right now.".to_string())?;
    let client = { vrchat_slot.read().await.clone() }
        .ok_or_else(|| "Celeste isn't logged in to VRChat right now.".to_string())?;

    let group_id: Option<String> = settings
        .store
        .get_object(scarlet_vrchat::client::GROUP_ID_KEY);
    let group_id = group_id.filter(|id| !id.trim().is_empty()).ok_or_else(|| {
        "No VRChat group is configured (run Celeste's setup wizard, or set `vrchat_group_id`)."
            .to_string()
    })?;

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
    moderation_context_for_snowflake(ctx, &command.user.id.to_string(), permission).await
}

/// Modal/component counterpart to [`moderation_context`].
pub(crate) async fn moderation_context_for_snowflake(
    ctx: &Context,
    snowflake: &str,
    permission: &str,
) -> Result<ModerationContext, String> {
    let scarlet_data = {
        ctx.data
            .read()
            .await
            .get::<crate::data::ScarletDataKey>()
            .cloned()
    }
    .ok_or_else(|| "Celeste data is not available.".to_string())?;
    let context = vrchat_context(ctx).await?;

    let actor_vrc_id = scarlet_data.global_get_snowflake_id(snowflake).ok_or_else(|| {
        format!(
            "You must have linked ids to perform this action.\n(Use `/staff-list add` `vrchat-user`:*<your VRChat user ID>* `discord-user`:<@{snowflake}> to accomplish this)"
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
                parts.push(
                    chars[start..last_newline]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
                start = last_newline + 1;
            }
            _ => {
                parts.push(
                    chars[start..end]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
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
        assert_eq!(
            paginate_on_lines("hello world", 2000),
            vec!["hello world".to_string()]
        );
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
        assert_eq!(
            pages,
            vec!["line one\nline two".to_string(), "line three".to_string()]
        );
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

#[cfg(test)]
mod command_surface_tests {
    use super::*;

    /// Every invocable command this build registers, as `"name sub group"`
    /// paths — the same shape as `java_commands.txt`.
    fn rust_command_leaves() -> std::collections::BTreeSet<String> {
        let mut leaves = std::collections::BTreeSet::new();
        for command in all_commands() {
            let value = serde_json::to_value(&command).expect("a command serializes");
            let Some(name) = value.get("name").and_then(|name| name.as_str()) else { continue };
            // Message/user context commands (`type` 2 or 3) aren't slash
            // commands and have no counterpart in the Java inventory.
            if value.get("type").and_then(|kind| kind.as_u64()).is_some_and(|kind| kind != 1) {
                continue;
            }
            collect(&mut leaves, name, value.get("options"));
        }
        leaves
    }

    /// Walks an options tree, recording a path for every leaf. Discord's
    /// option types: 1 = sub-command, 2 = sub-command group.
    fn collect(out: &mut std::collections::BTreeSet<String>, prefix: &str, options: Option<&serde_json::Value>) {
        let children: Vec<&serde_json::Value> = options
            .and_then(|options| options.as_array())
            .map(|options| {
                options
                    .iter()
                    .filter(|option| {
                        matches!(option.get("type").and_then(serde_json::Value::as_u64), Some(1) | Some(2))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if children.is_empty() {
            out.insert(prefix.to_string());
            return;
        }
        for child in children {
            let Some(name) = child.get("name").and_then(|name| name.as_str()) else { continue };
            collect(out, &format!("{prefix} {name}"), child.get("options"));
        }
    }

    fn java_command_leaves() -> std::collections::BTreeSet<String> {
        include_str!("../../java_commands.txt")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect()
    }

    /// Java commands this port deliberately does not have, each tied to a
    /// subsystem that is out of scope (see PORTING.md's scope section). Any
    /// *other* missing command is a gap, and the test below names it.
    const DELIBERATELY_ABSENT: &[&str] = &[
        // Report templates.
        "config-set report-template download-report-template",
        "config-set report-template edit-report-template",
        "config-set report-template upload-report-template",
        "config-set report-template view-report-template",
        "config-set report-template view-report-template-format",
        // Evidence submission.
        "submit-evidence",
        // Scheduling / calendar events, in full.
        "schedule add",
        "schedule cancel",
        "schedule list",
        "schedule new",
        "schedule remove",
        "schedule set-access",
        "schedule set-active",
        "schedule set-category",
        "schedule set-close-after",
        "schedule set-date",
        "schedule set-description",
        "schedule set-duration",
        "schedule set-featured",
        "schedule set-frequency",
        "schedule set-guest-join-early",
        "schedule set-host-join-early",
        "schedule set-image",
        "schedule set-languages",
        "schedule set-max-pending",
        "schedule set-mirror-on-discord",
        "schedule set-notify-create",
        "schedule set-overflow",
        "schedule set-platforms",
        "schedule set-roles",
        "schedule set-tags",
        "schedule set-time",
        "schedule set-title",
    ];

    /// Java commands that *should* exist here and don't yet — a gap, not a
    /// decision. Kept separate from the list above so "we chose not to" and
    /// "we haven't got there" never blur together.
    const NOT_YET_PORTED: &[(&str, &str)] = &[(
        "server-restart update-now",
        "Java writes the target version to scarlet.version and restarts into it, \
         which works because its launcher fetches that version's jar. A native \
         binary has no equivalent; an in-place self-updater would be a new \
         subsystem rather than a port.",
    )];

    /// The one test that answers "what's still missing?" with a list instead
    /// of a feeling. It fails when a Java command has no Rust counterpart
    /// and isn't on the deliberately-absent list, and prints exactly which.
    #[test]
    fn command_surface_matches_java() {
        let rust = rust_command_leaves();
        let java = java_command_leaves();
        let absent: std::collections::BTreeSet<&str> = DELIBERATELY_ABSENT.iter().copied().collect();

        let known_gap: std::collections::BTreeSet<&str> =
            NOT_YET_PORTED.iter().map(|(leaf, _)| *leaf).collect();

        let missing: Vec<&String> = java
            .iter()
            .filter(|leaf| !rust.contains(*leaf) && !absent.contains(leaf.as_str()) && !known_gap.contains(leaf.as_str()))
            .collect();

        assert!(
            missing.is_empty(),
            "{} Java command(s) have no Rust counterpart and aren't accounted for:\n  {}\n\n\
             (Ported {} of {} invocable Java commands; {} deliberately absent, {} known gaps.)",
            missing.len(),
            missing.iter().map(|leaf| leaf.as_str()).collect::<Vec<_>>().join("\n  "),
            java.len() - absent.len() - known_gap.len(),
            java.len(),
            absent.len(),
            known_gap.len(),
        );

        // A gap that has since been filled should be taken off the list,
        // otherwise it stays "known" forever and nobody notices it landed.
        let stale: Vec<&str> = known_gap.iter().copied().filter(|leaf| rust.contains(*leaf)).collect();
        assert!(stale.is_empty(), "these are listed as not-yet-ported but are registered: {stale:?}");
    }

    /// The reverse direction: something registered here that Java has no
    /// command for is either a deliberate addition or a typo in a command
    /// name, and a typo is worth catching.
    #[test]
    fn no_unexpected_commands_beyond_javas_surface() {
        let rust = rust_command_leaves();
        let java = java_command_leaves();
        // Java's `/scarlet-version` is this build's `/celeste-version`
        // after the rename; nothing else should be new.
        const EXPECTED_ADDITIONS: &[&str] = &["celeste-version", "scarlet-version"];
        let extra: Vec<&String> =
            rust.iter().filter(|leaf| !java.contains(*leaf) && !EXPECTED_ADDITIONS.contains(&leaf.as_str())).collect();
        assert!(extra.is_empty(), "commands registered that Java doesn't have: {extra:?}");
    }
}
