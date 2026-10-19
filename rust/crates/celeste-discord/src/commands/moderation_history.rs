//! Ports the user-focused audit history commands from
//! `ScarletDiscordCommands`: `/moderation-log`, `/query-target-history`,
//! `/query-actor-history`, and `/actor-moderation-summary`.

use chrono::{Duration, Utc};
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateEmbed, CreateInteractionResponseFollowup,
    CreateMessage, CreateThread,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::channel::{AutoArchiveDuration, ChannelType};

use celeste_vrchat::client::GroupAuditLogFilter;

use crate::data::{CelesteDataKey, SecretStaffListKey, SettingsKey};

pub const MODERATION_LOG_NAME: &str = "moderation-log";
pub const QUERY_TARGET_NAME: &str = "query-target-history";
pub const QUERY_ACTOR_NAME: &str = "query-actor-history";
pub const ACTOR_SUMMARY_NAME: &str = "actor-moderation-summary";

fn user_option() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::String,
        "vrchat-user",
        "The VRChat user id (usr_...)",
    )
    .required(true)
    .set_autocomplete(true)
}

fn days_option() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::Integer,
        "days-back",
        "The number of days into the past to search for events",
    )
    .min_int_value(1)
    .max_int_value(1000)
}

pub fn register_moderation_log() -> CreateCommand {
    CreateCommand::new(MODERATION_LOG_NAME)
        .description(
            "Consolidate a VRChat user's moderation history (counts + links) into one post",
        )
        .add_option(user_option())
}

pub fn register_query_target() -> CreateCommand {
    CreateCommand::new(QUERY_TARGET_NAME)
        .description("Queries audit events targeting a specific VRChat user")
        .add_option(user_option())
        .add_option(days_option())
}

pub fn register_query_actor() -> CreateCommand {
    CreateCommand::new(QUERY_ACTOR_NAME)
        .description("Queries audit events actored by a specific VRChat user")
        .add_option(user_option())
        .add_option(days_option())
}

pub fn register_actor_summary() -> CreateCommand {
    CreateCommand::new(ACTOR_SUMMARY_NAME)
        .description("Generates a summary of certain moderation actions by a specific user")
        .add_option(user_option())
}

fn options(command: &CommandInteraction) -> Result<(String, i64), &'static str> {
    let options = command.data.options();
    let user_id = options
        .iter()
        .find(|option| option.name == "vrchat-user")
        .and_then(|option| match option.value {
            ResolvedValue::String(value) => Some(value.to_string()),
            _ => None,
        })
        .ok_or("A `vrchat-user` option is required.")?;
    let days = options
        .iter()
        .find(|option| option.name == "days-back")
        .and_then(|option| match option.value {
            ResolvedValue::Integer(value) => Some(value),
            _ => None,
        })
        .unwrap_or(4);
    Ok((user_id, days))
}

async fn should_redact(ctx: &Context, requester_snowflake: &str, vrc_id: &str) -> bool {
    let data = ctx.data.read().await;
    let Some(secret_staff) = data.get::<SecretStaffListKey>() else {
        return false;
    };
    if !secret_staff.is_id(vrc_id) {
        return false;
    }
    let requester_vrc_id = data
        .get::<CelesteDataKey>()
        .and_then(|celeste_data| celeste_data.global_get_snowflake_id(requester_snowflake));
    !requester_vrc_id
        .as_deref()
        .is_some_and(|id| secret_staff.is_id(id))
}

pub async fn run_query_target(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let (vrc_id, days_back) = match options(command) {
        Ok(value) => value,
        Err(message) => return super::reply_ephemeral(ctx, command, message).await,
    };
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let user = match context.client.get_user(&vrc_id).await {
        Ok(user) => user,
        Err(_) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {vrc_id}"),
            )
            .await;
        }
    };
    let display_name = user.display_name.unwrap_or_else(|| vrc_id.clone());
    let to = Utc::now();
    let from = to - Duration::days(days_back);
    let entries = match context
        .client
        .query_group_audit_logs(
            &context.group_id,
            &GroupAuditLogFilter::between(from, to).about_targets(&vrc_id),
        )
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Error querying audit target history for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>) ({vrc_id}): {err}"),
            )
            .await;
        }
    };
    if entries.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("No audit target history for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>) ({vrc_id})"),
        )
        .await;
    }

    let celeste_data = { ctx.data.read().await.get::<CelesteDataKey>().cloned() };
    let requester = command.user.id.to_string();
    let redact_target = should_redact(ctx, &requester, &vrc_id).await;
    let mut lines = vec![format!(
        "VRChat audit target history for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>):"
    )];
    if let Some(snowflake) = celeste_data
        .as_ref()
        .and_then(|data| data.user_metadata_get_snowflake(&vrc_id))
        .filter(|_| !redact_target)
    {
        lines.push(format!("### Linked Discord id:\n`{snowflake}`"));
    }
    // Java substitutes the *bot's own* VRChat display name
    // (`vrc.currentUser.getDisplayName()`), falling back to "a moderator"
    // only when there is no session — so a reader can still tell the action
    // was attributed to the Celeste account. Resolved once for the whole
    // listing rather than per entry.
    let redaction_name = context
        .client
        .get_current_user()
        .await
        .ok()
        .and_then(|user| user.display_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "a moderator".to_string());

    lines.push("### Audit target events:".to_string());
    for entry in entries {
        let actor_redacted = match entry.actor_id.as_deref() {
            Some(actor_id) => should_redact(ctx, &requester, actor_id).await,
            None => false,
        };
        let mut description = entry
            .description
            .unwrap_or_else(|| entry.event_type.clone());
        if actor_redacted {
            // Java substitutes only when the description actually contains
            // the actor's name, and otherwise **discards the description
            // entirely** for a canned sentence
            // (`ScarletDiscordCommands.java:3205-3212`). An unconditional
            // `replace` is a no-op when the name isn't literally present,
            // which printed the original unredacted text — the exact thing
            // this branch exists to prevent.
            description = redacted_description(
                &description,
                entry.actor_display_name.as_deref(),
                &redaction_name,
                &display_name,
            );
        }
        let linked = if actor_redacted {
            None
        } else {
            celeste_data
                .as_ref()
                .and_then(|data| data.audit_entry_metadata(&entry.id).message_url())
        };
        lines.push(format_entry_line(
            &entry.id,
            entry.created_at.timestamp(),
            &description,
            linked.as_deref(),
        ));
    }
    super::send_pages(
        ctx,
        command,
        super::paginate_on_lines(&lines.join("\n"), 2000),
    )
    .await
}

pub async fn run_query_actor(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let (vrc_id, days_back) = match options(command) {
        Ok(value) => value,
        Err(message) => return super::reply_ephemeral(ctx, command, message).await,
    };
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let user = match context.client.get_user(&vrc_id).await {
        Ok(user) => user,
        Err(_) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {vrc_id}"),
            )
            .await;
        }
    };
    let display_name = user.display_name.unwrap_or_else(|| vrc_id.clone());
    if should_redact(ctx, &command.user.id.to_string(), &vrc_id).await {
        return no_actor_history(ctx, command, &display_name, &vrc_id).await;
    }
    let to = Utc::now();
    let from = to - Duration::days(days_back);
    let entries = match context
        .client
        .query_group_audit_logs(
            &context.group_id,
            &GroupAuditLogFilter::between(from, to).by_actors(&vrc_id),
        )
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Error querying audit actor history for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>) ({vrc_id}): {err}"),
            )
            .await;
        }
    };
    if entries.is_empty() {
        return no_actor_history(ctx, command, &display_name, &vrc_id).await;
    }
    let celeste_data = { ctx.data.read().await.get::<CelesteDataKey>().cloned() };
    let mut lines = vec![format!(
        "VRChat audit actor history for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>):"
    )];
    if let Some(snowflake) = celeste_data
        .as_ref()
        .and_then(|data| data.user_metadata_get_snowflake(&vrc_id))
    {
        lines.push(format!("### Linked Discord id:\n`{snowflake}`"));
    }
    lines.push("### Audit actor events:".to_string());
    for entry in entries {
        let description = entry
            .description
            .unwrap_or_else(|| entry.event_type.clone());
        let linked = celeste_data
            .as_ref()
            .and_then(|data| data.audit_entry_metadata(&entry.id).message_url());
        lines.push(format_entry_line(
            &entry.id,
            entry.created_at.timestamp(),
            &description,
            linked.as_deref(),
        ));
    }
    super::send_pages(
        ctx,
        command,
        super::paginate_on_lines(&lines.join("\n"), 2000),
    )
    .await
}

async fn no_actor_history(
    ctx: &Context,
    command: &CommandInteraction,
    display_name: &str,
    vrc_id: &str,
) -> serenity::Result<()> {
    super::reply_ephemeral(
        ctx,
        command,
        &format!("No audit actor history for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>) ({vrc_id})"),
    )
    .await
}

pub async fn run_actor_summary(
    ctx: &Context,
    command: &CommandInteraction,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let (vrc_id, _) = match options(command) {
        Ok(value) => value,
        Err(message) => return super::reply_ephemeral(ctx, command, message).await,
    };
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let user = match context.client.get_user(&vrc_id).await {
        Ok(user) => user,
        Err(_) => {
            return super::reply_ephemeral(ctx, command, "No VRChat user found for that input.")
                .await
        }
    };
    let display_name = user.display_name.unwrap_or_else(|| vrc_id.clone());
    if should_redact(ctx, &command.user.id.to_string(), &vrc_id).await {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("No audit actor summary for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>) ({vrc_id})"),
        )
        .await;
    }
    let to = Utc::now();
    let from = to - Duration::days(3652);
    let mut counts = Vec::new();
    for (label, event_type) in [
        ("Kicks", "group.instance.kick"),
        ("Warns", "group.instance.warn"),
        ("Bans", "group.user.ban"),
    ] {
        let count = context
            .client
            .count_group_audit_logs(
                &context.group_id,
                &GroupAuditLogFilter::between(from, to)
                    .by_actors(&vrc_id)
                    .of_types(event_type),
            )
            .await;
        counts.push(format!(
            "{label}: {}",
            count
                .map(|value| value.to_string())
                .unwrap_or_else(|_| "??? (query failed)".into())
        ));
    }
    let celeste_data = { ctx.data.read().await.get::<CelesteDataKey>().cloned() };
    let mut lines = vec![format!(
        "VRChat audit actor summary for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>):"
    )];
    if let Some(snowflake) = celeste_data.and_then(|data| data.user_metadata_get_snowflake(&vrc_id))
    {
        lines.push(format!("### Linked Discord id:\n`{snowflake}`"));
    }
    lines.extend(counts);
    super::reply_ephemeral(ctx, command, &lines.join("\n")).await
}

pub async fn run_moderation_log(
    ctx: &Context,
    command: &CommandInteraction,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let (vrc_id, _) = match options(command) {
        Ok(value) => value,
        Err(message) => return super::reply_ephemeral(ctx, command, message).await,
    };
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let user = match context.client.get_user(&vrc_id).await {
        Ok(user) => user,
        Err(_) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {vrc_id}"),
            )
            .await
        }
    };
    let display_name = user.display_name.unwrap_or_else(|| vrc_id.clone());
    let data = { ctx.data.read().await.get::<CelesteDataKey>().cloned() };
    let Some(data) = data else {
        return super::reply_ephemeral(ctx, command, "Celeste data is not available.").await;
    };
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let lookback_days = settings
        .map(|settings| settings.moderation_log_lookback_days.get())
        .unwrap_or(0);
    let cutoff = (lookback_days > 0).then(|| Utc::now() - Duration::days(i64::from(lookback_days)));
    let user_meta = data.user_metadata(&vrc_id);
    if user_meta.audit_entry_ids.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("No moderation history recorded for [{display_name}](https://vrchat.com/home/user/{vrc_id})"),
        )
        .await;
    }

    let mut entries = Vec::new();
    for id in &user_meta.audit_entry_ids {
        let meta = data.audit_entry_metadata(id);
        if meta.entry_redacted || meta.has_parent_event() {
            continue;
        }
        let Some(entry) = meta.entry.as_ref() else {
            continue;
        };
        let Some(created_at) = entry
            .get("createdAt")
            .and_then(|value| value.as_str())
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
        else {
            continue;
        };
        if cutoff.is_some_and(|cutoff| created_at < cutoff) {
            continue;
        }
        entries.push((created_at, id.clone(), meta));
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    if entries.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("No moderation history for [{display_name}](https://vrchat.com/home/user/{vrc_id}) within the configured window ({lookback_days} day(s))."),
        )
        .await;
    }

    let mut counts = [0usize; 7];
    for (_, _, meta) in &entries {
        let event_type = meta
            .entry
            .as_ref()
            .and_then(|entry| entry.get("eventType"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let index = match event_type {
            "group.instance.warn" => 0,
            "group.instance.mute" => 1,
            "group.instance.kick" => 2,
            "group.member.remove" => 3,
            "group.user.ban" => 4,
            "group.user.unban" => 5,
            _ => 6,
        };
        counts[index] += 1;
    }
    let labels = [
        "warn",
        "mute",
        "instance kick",
        "group removal",
        "ban",
        "unban",
        "other",
    ];
    let mut parts = Vec::new();
    for (index, count) in counts.into_iter().enumerate() {
        if count > 0 {
            let plural = if count == 1 || index == 6 { "" } else { "s" };
            parts.push(format!("{count} {}{plural}", labels[index]));
        }
    }
    let window = if lookback_days > 0 {
        format!("past {lookback_days} day(s)")
    } else {
        "all recorded history".to_string()
    };
    let summary = format!(
        "**{}** action(s): {} _({window})_",
        entries.len(),
        parts.join(", ")
    );
    let mut action_lines = Vec::new();
    for (created_at, _, meta) in entries.iter().take(150) {
        let entry = meta.entry.as_ref().expect("filtered above");
        let event_type = entry
            .get("eventType")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let title = audit_type_title(event_type);
        let actor = meta.aux_actor_display_name.as_deref().or_else(|| {
            entry
                .get("actorDisplayName")
                .and_then(|value| value.as_str())
        });
        let description = meta
            .entry_description
            .as_deref()
            .or_else(|| entry.get("description").and_then(|value| value.as_str()));
        let mut line = format!("<t:{}:f> **{title}**", created_at.timestamp());
        if let Some(actor) = actor.filter(|value| !value.is_empty()) {
            line.push_str(&format!(" by {actor}"));
        }
        match (
            description.map(str::trim).filter(|value| !value.is_empty()),
            meta.some_url(),
        ) {
            (Some(description), Some(url)) => line.push_str(&format!(": [{description}]({url})")),
            (Some(description), None) => line.push_str(&format!(": {description}")),
            (None, Some(url)) => line.push_str(&format!(" ([post]({url}))")),
            (None, None) => {}
        }
        action_lines.push(line);
    }
    if entries.len() > 150 {
        action_lines.push(format!(
            "_… and {} older action(s) not listed._",
            entries.len() - 150
        ));
    }
    send_moderation_log(
        ctx,
        command,
        &display_name,
        &vrc_id,
        &summary,
        &action_lines,
    )
    .await
}

async fn send_moderation_log(
    ctx: &Context,
    command: &CommandInteraction,
    display_name: &str,
    vrc_id: &str,
    summary: &str,
    action_lines: &[String],
) -> serenity::Result<()> {
    let thread_name = truncate(&format!("Moderation log — {display_name}"), 100);
    let thread = command
        .channel_id
        .create_thread(
            &ctx.http,
            CreateThread::new(thread_name)
                .kind(ChannelType::PublicThread)
                .auto_archive_duration(AutoArchiveDuration::OneWeek),
        )
        .await;
    if let Ok(thread) = thread {
        let title = truncate(&format!("Moderation log — {display_name}"), 256);
        thread
            .id
            .send_message(
                &ctx.http,
                CreateMessage::new().embed(
                    CreateEmbed::new()
                        .title(title)
                        .url(format!("https://vrchat.com/home/user/{vrc_id}"))
                        .description(summary),
                ),
            )
            .await?;
        for page in super::paginate_on_lines(&action_lines.join("\n"), 4000) {
            thread
                .id
                .send_message(
                    &ctx.http,
                    CreateMessage::new().embed(CreateEmbed::new().description(page)),
                )
                .await?;
        }
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Created moderation log thread: <#{}>", thread.id),
        )
        .await;
    }

    let mut lines = vec![
        format!("## Moderation log — {display_name}"),
        summary.to_string(),
    ];
    lines.extend(action_lines.iter().cloned());
    for page in super::paginate_on_lines(&lines.join("\n"), 2000) {
        command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new().content(page),
            )
            .await?;
    }
    Ok(())
}

fn format_entry_line(id: &str, timestamp: i64, description: &str, url: Option<&str>) -> String {
    match url {
        Some(url) => format!("`{id}` <t:{timestamp}:f>: [{description}]({url})"),
        None => format!("`{id}` <t:{timestamp}:f>: {description}"),
    }
}

fn audit_type_title(event_type: &str) -> String {
    // Keep every command on the canonical Java `GroupAuditType.title` table.
    // Re-deriving this from the id changes names such as calendarEvent and
    // instance.announcement into "CalendarEvent" and "Instance Announcement".
    super::vrchat_group::audit_type_title(event_type)
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

/// Port of the redaction branch in `ScarletDiscordCommands.queryTargetHistory`
/// (`:3205-3212`).
///
/// Java substitutes the moderator's name only when the description actually
/// contains it, and otherwise **throws the description away** in favour of a
/// canned sentence. Doing an unconditional `String::replace` instead looks
/// equivalent but isn't: when the name doesn't appear, the replace is a
/// no-op and the original, unredacted description is published — which is
/// precisely what this command's redaction gate exists to prevent.
fn redacted_description(
    description: &str,
    actor_display_name: Option<&str>,
    redaction_name: &str,
    target_display_name: &str,
) -> String {
    match actor_display_name {
        Some(actor_name) if !actor_name.is_empty() && description.contains(actor_name) => {
            description.replace(actor_name, redaction_name)
        }
        _ => format!("User {target_display_name} was banned by {redaction_name}."),
    }
}

#[cfg(test)]
mod tests {
    /// The leak this guards: an unconditional `replace` silently publishes
    /// the original description whenever the moderator's name isn't
    /// literally in it.
    #[test]
    fn a_description_without_the_actors_name_is_discarded_not_published() {
        let leaked = redacted_description(
            "Banned for ban evasion after a report from staff",
            Some("SomeModerator"),
            "Celeste",
            "Target",
        );
        assert_eq!(leaked, "User Target was banned by Celeste.");
        assert!(!leaked.contains("ban evasion"));
    }

    #[test]
    fn a_description_containing_the_actors_name_is_substituted() {
        assert_eq!(
            redacted_description(
                "SomeModerator banned this user",
                Some("SomeModerator"),
                "Celeste",
                "Target",
            ),
            "Celeste banned this user"
        );
    }

    #[test]
    fn a_missing_or_empty_actor_name_falls_back_to_javas_sentence() {
        assert_eq!(
            redacted_description("anything at all", None, "Celeste", "Target"),
            "User Target was banned by Celeste."
        );
        assert_eq!(
            redacted_description("anything at all", Some(""), "Celeste", "Target"),
            "User Target was banned by Celeste."
        );
    }

    use super::*;

    #[test]
    fn known_and_unknown_audit_titles_are_readable() {
        assert_eq!(audit_type_title("group.user.ban"), "User Ban");
        assert_eq!(
            audit_type_title("group.calendarEvent.create"),
            "Event Create"
        );
        assert_eq!(audit_type_title("group.instance.announcement"), "Announcement");
    }

    #[test]
    fn linked_entry_line_masks_the_url() {
        assert_eq!(
            format_entry_line("gaud_1", 123, "A ban", Some("https://discord.test/1")),
            "`gaud_1` <t:123:f>: [A ban](https://discord.test/1)"
        );
    }
}
