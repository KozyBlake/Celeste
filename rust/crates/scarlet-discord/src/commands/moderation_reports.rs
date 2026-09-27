//! Periodic/on-demand moderation summary reports.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateEmbed, CreateInteractionResponseFollowup,
    CreateMessage, CreateThread,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::channel::AutoArchiveDuration;

use crate::data::{ScarletDataKey, SecretStaffListKey, SettingsKey, StaffListKey};

pub const SUMMARY_NAME: &str = "moderation-summary";
pub const OUTSTANDING_NAME: &str = "outstanding-moderation";

fn hours_option() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::Integer,
        "hours-back",
        "The number of hours into the past to search for events",
    )
    .min_int_value(1)
    .max_int_value(24_000)
}

pub fn register_summary() -> CreateCommand {
    CreateCommand::new(SUMMARY_NAME)
        .description("Generates a summary of moderation actions")
        .add_option(hours_option())
}

pub fn register_outstanding() -> CreateCommand {
    CreateCommand::new(OUTSTANDING_NAME)
        .description("Generates a list of outstanding moderation actions")
        .add_option(hours_option())
}

fn hours_back(command: &CommandInteraction) -> i64 {
    command
        .data
        .options()
        .iter()
        .find(|option| option.name == "hours-back")
        .and_then(|option| match option.value {
            ResolvedValue::Integer(value) => Some(value),
            _ => None,
        })
        .unwrap_or(24)
}

pub async fn run_summary(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let shared = ctx.data.read().await;
    let scarlet_data = shared.get::<ScarletDataKey>().cloned();
    let staff_list = shared.get::<StaffListKey>().cloned();
    let settings = shared.get::<SettingsKey>().cloned();
    drop(shared);
    let (Some(scarlet_data), Some(staff_list)) = (scarlet_data, staff_list) else {
        return super::reply_ephemeral(ctx, command, "Scarlet moderation data is unavailable.")
            .await;
    };

    let end = Utc::now();
    let start = end - Duration::hours(hours_back(command));
    let entries = match context
        .client
        .query_group_audit_logs(
            &context.group_id,
            start,
            end,
            None,
            Some("group.instance.kick,group.instance.warn,group.user.ban,group.invite.create"),
            None,
        )
        .await
    {
        Ok(entries) => entries,
        Err(_) => {
            let embed = report_header(
                "Moderation Summary",
                start.timestamp(),
                end.timestamp(),
                0x7f7fff,
            )
            .description(format!(
                "Spanning <t:{}:f> through <t:{}:f>\nFailed to generate mod summary: could not query audit events",
                start.timestamp(), end.timestamp()
            ));
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new().embed(embed),
                )
                .await?;
            return Ok(());
        }
    };

    let mut actions: HashMap<String, [u64; 4]> = HashMap::new();
    let mut display_names = HashMap::new();
    for entry in entries {
        let meta = scarlet_data.audit_entry_metadata(&entry.id);
        let actor_id = meta
            .aux_actor_id
            .as_deref()
            .or(entry.actor_id.as_deref())
            .unwrap_or("unknown")
            .to_string();
        let display_name = meta
            .aux_actor_display_name
            .as_deref()
            .or(entry.actor_display_name.as_deref());
        if let Some(display_name) = display_name {
            display_names.insert(actor_id.clone(), display_name.to_string());
        }
        let counts = actions.entry(actor_id).or_default();
        match entry.event_type.as_str() {
            "group.instance.warn" => counts[0] += 1,
            "group.instance.kick" => counts[1] += 1,
            "group.user.ban" => counts[2] += 1,
            "group.invite.create" => counts[3] += 1,
            _ => {}
        }
    }

    let membership_entries = context
        .client
        .query_group_audit_logs(
            &context.group_id,
            start,
            end,
            None,
            Some("group.member.join,group.member.leave,group.member.remove,group.invite.create"),
            None,
        )
        .await
        .unwrap_or_default();
    let mut membership = [0u64; 4];
    for entry in membership_entries {
        match entry.event_type.as_str() {
            "group.member.join" => membership[0] += 1,
            "group.member.leave" => membership[1] += 1,
            "group.member.remove" => membership[2] += 1,
            "group.invite.create" => membership[3] += 1,
            _ => {}
        }
    }

    let mut staff_joins: HashMap<String, u64> = HashMap::new();
    for id in scarlet_data.custom_event_filter(
        Some("groupex.instance.staff.join"),
        Some(start),
        Some(end),
    ) {
        if let Some(actor_id) = scarlet_data.custom_event(&id).actor_id {
            *staff_joins.entry(actor_id).or_default() += 1;
        }
    }
    let instance_joins = scarlet_data
        .custom_event_filter(Some("groupex.instance.user.join"), Some(start), Some(end))
        .len();

    let mut lines = vec![format!(
        "### {}/{}/{}/{} group joins/leaves/kicks/invites",
        membership[0], membership[1], membership[2], membership[3]
    )];
    lines.push(format!("### {instance_joins} instance joins"));
    let votes_to_kick = actions
        .get("vrc_admin")
        .map(|counts| counts[1])
        .unwrap_or(0);
    if votes_to_kick > 0 {
        lines.push(format!("### Successful Votes-to-Kick: {votes_to_kick}"));
    }
    lines.push("### Staff moderation (warns/kicks/bans/invites/joins)".to_string());
    let only_activity = settings
        .as_ref()
        .is_some_and(|settings| settings.moderation_summary_only_activity.get());
    let mut skipped = 0usize;
    let mut totals = [0u64; 5];
    for staff_id in staff_list.get_ids() {
        let action = actions.get(&staff_id).copied().unwrap_or_default();
        let joins = staff_joins.get(&staff_id).copied().unwrap_or(0);
        let counts = [action[0], action[1], action[2], action[3], joins];
        for (total, value) in totals.iter_mut().zip(counts) {
            *total += value;
        }
        if only_activity && counts.iter().all(|count| *count == 0) {
            skipped += 1;
            continue;
        }
        let display_name = match display_names.get(&staff_id) {
            Some(name) => name.clone(),
            None => context
                .client
                .get_user(&staff_id)
                .await
                .ok()
                .and_then(|user| user.display_name)
                .unwrap_or_else(|| staff_id.clone()),
        };
        lines.push(format!(
            "[{display_name}](https://vrchat.com/home/user/{staff_id}): {} / {} / {} / {} / {}",
            counts[0], counts[1], counts[2], counts[3], counts[4]
        ));
    }
    if only_activity {
        lines.push(format!("(skipped {skipped} for inactivity)"));
    }
    lines.push(format!(
        "Staff totals: {} / {} / {} / {} / {}",
        totals[0], totals[1], totals[2], totals[3], totals[4]
    ));

    command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().embed(report_header(
                "Moderation Summary",
                start.timestamp(),
                end.timestamp(),
                0x7f7fff,
            )),
        )
        .await?;
    for page in super::paginate_on_lines(&lines.join("\n"), 4096) {
        command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new()
                    .embed(CreateEmbed::new().description(page)),
            )
            .await?;
    }
    Ok(())
}

pub async fn run_outstanding(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let shared = ctx.data.read().await;
    let scarlet_data = shared.get::<ScarletDataKey>().cloned();
    let secret_staff = shared.get::<SecretStaffListKey>().cloned();
    let settings = shared.get::<SettingsKey>().cloned();
    drop(shared);
    let (Some(scarlet_data), Some(secret_staff), Some(settings)) =
        (scarlet_data, secret_staff, settings)
    else {
        return super::reply_ephemeral(ctx, command, "Scarlet moderation data is unavailable.")
            .await;
    };

    let watched = outstanding_types(&settings);
    if watched.is_empty() {
        return command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new().content("No moderation events watched"),
            )
            .await
            .map(|_| ());
    }
    let end = Utc::now();
    let start = end - Duration::hours(hours_back(command));
    let event_types = watched.join(",");
    let entries = match context
        .client
        .query_group_audit_logs(
            &context.group_id,
            start,
            end,
            None,
            Some(&event_types),
            None,
        )
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not query outstanding moderation events: {err}"),
            )
            .await;
        }
    };

    let mut grouped: HashMap<String, (String, Vec<(String, String)>)> = HashMap::new();
    for entry in entries {
        if entry
            .actor_id
            .as_deref()
            .is_some_and(|id| secret_staff.is_id(id))
        {
            continue;
        }
        if !scarlet_data.audit_entry_metadata_exists(&entry.id) {
            continue;
        }
        let meta = scarlet_data.audit_entry_metadata(&entry.id);
        if meta
            .aux_actor_id
            .as_deref()
            .is_some_and(|id| secret_staff.is_id(id))
            || meta.has_tags()
            || meta.has_description()
            || meta.entry_redacted
            || meta.has_parent_event()
        {
            continue;
        }
        let actor_id = meta
            .aux_actor_id
            .as_deref()
            .or(entry.actor_id.as_deref())
            .unwrap_or("unknown")
            .to_string();
        let actor_name = meta
            .aux_actor_display_name
            .as_deref()
            .or(entry.actor_display_name.as_deref())
            .unwrap_or(&actor_id)
            .to_string();
        let description = entry
            .description
            .unwrap_or_else(|| entry.event_type.clone());
        let url = meta
            .some_url()
            .or_else(|| meta.channel_url())
            .unwrap_or_else(|| format!("https://vrchat.com/home/group/{}", context.group_id));
        grouped
            .entry(actor_id)
            .or_insert_with(|| (actor_name, Vec::new()))
            .1
            .push((description, url));
    }

    let header = command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().embed(report_header(
                "Outstanding Moderation",
                start.timestamp(),
                end.timestamp(),
                0xff7f00,
            )),
        )
        .await?;
    if grouped.is_empty() {
        return Ok(());
    }
    let thread = command
        .channel_id
        .create_thread_from_message(
            &ctx.http,
            header.id,
            CreateThread::new(format!(
                "Outstanding Moderation as of {}",
                end.format("%Y-%m-%d %H:%M UTC")
            ))
            .auto_archive_duration(AutoArchiveDuration::OneWeek),
        )
        .await
        .ok();
    let destination = thread.map(|thread| thread.id).unwrap_or(command.channel_id);
    let mut actors: Vec<_> = grouped.into_iter().collect();
    actors.sort_by(|left, right| left.1 .0.cmp(&right.1 .0));
    for (actor_id, (actor_name, entries)) in actors {
        let mut lines = Vec::new();
        match scarlet_data.user_metadata_get_snowflake(&actor_id) {
            Some(snowflake) => lines.push(format!("<@{snowflake}>:")),
            None => lines.push(format!(
                "Unknown Discord id for [{actor_name}](<https://vrchat.com/home/user/{actor_id}>)"
            )),
        }
        for (description, url) in entries {
            lines.push(format!("[Tags, Description](<{url}>): {description}"));
        }
        for page in super::paginate_on_lines(&lines.join("\n"), 2000) {
            destination
                .send_message(&ctx.http, CreateMessage::new().content(page))
                .await?;
        }
    }
    Ok(())
}

fn outstanding_types(settings: &scarlet_core::settings::Settings) -> Vec<&'static str> {
    let mut types = Vec::new();
    if settings.ping_on_outstanding_moderation_instance_warn.get() {
        types.push("group.instance.warn");
    }
    if settings.ping_on_outstanding_moderation_instance_kick.get() {
        types.push("group.instance.kick");
    }
    if settings.ping_on_outstanding_moderation_member_remove.get() {
        types.push("group.member.remove");
    }
    if settings.ping_on_outstanding_moderation_user_ban.get() {
        types.push("group.user.ban");
    }
    if settings.ping_on_outstanding_moderation_user_unban.get() {
        types.push("group.user.unban");
    }
    types
}

fn report_header(title: &str, start: i64, end: i64, color: u32) -> CreateEmbed {
    CreateEmbed::new()
        .title(title)
        .description(format!("Spanning <t:{start}:f> through <t:{end}:f>"))
        .color(color)
        .footer(serenity::builder::CreateEmbedFooter::new(
            "Scarlet • Extended event",
        ))
        .timestamp(serenity::model::Timestamp::from_unix_timestamp(end).unwrap_or_default())
}
