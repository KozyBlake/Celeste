//! Port of the non-report-template `/config-set` command groups.

use chrono::{NaiveTime, TimeZone, Utc};
use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};

use crate::data::SettingsKey;

pub const NAME: &str = "config-set";

fn integer_leaf(
    name: &str,
    description: &str,
    option_name: &str,
    option_description: &str,
) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, name, description).add_sub_option(
        CreateCommandOption::new(CommandOptionType::Integer, option_name, option_description)
            .required(true)
            .min_int_value(1)
            .max_int_value(1000),
    )
}

fn time_leaf() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "time-of-day",
        "Set what time of day to generate the report",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "time-zone-id", "The time zone")
            .required(true),
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "time-of-day", "The time of day")
            .required(true),
    )
}

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Configures miscellaneous settings")
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommandGroup,
                "moderation-summary",
                "Moderation summary settings",
            )
            .add_sub_option(time_leaf()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommandGroup,
                "suggested-moderation",
                "Suggested moderation settings",
            )
            .add_sub_option(integer_leaf(
                "kick-count",
                "The kick count",
                "kick-count",
                "The number of instance kicks to trigger suggested moderation",
            ))
            .add_sub_option(integer_leaf(
                "period-days",
                "The period",
                "period-days",
                "The number of days of history for which to consider instance kicks",
            )),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommandGroup,
                "outstanding-moderation",
                "Outstanding moderation settings",
            )
            .add_sub_option(integer_leaf(
                "period-days",
                "The period",
                "period-days",
                "The number of days of history for which to consider moderation events",
            ))
            .add_sub_option(time_leaf()),
        )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return super::reply_ephemeral(ctx, command, "Settings are unavailable.").await;
    };
    let options = command.data.options();
    let Some(group) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A configuration group is required.").await;
    };
    let ResolvedValue::SubCommandGroup(group_options) = &group.value else {
        return super::reply_ephemeral(ctx, command, "Expected a configuration group.").await;
    };
    let Some(leaf) = group_options.first() else {
        return super::reply_ephemeral(ctx, command, "A configuration action is required.").await;
    };
    let ResolvedValue::SubCommand(leaf_options) = &leaf.value else {
        return super::reply_ephemeral(ctx, command, "Expected a configuration action.").await;
    };

    let message = match (group.name, leaf.name) {
        ("suggested-moderation", "kick-count") => {
            let Some(value) = integer_option(leaf_options, "kick-count") else {
                return super::reply_ephemeral(ctx, command, "A `kick-count` option is required.")
                    .await;
            };
            settings
                .heuristic_kick_count
                .set(Some(value as i32), "discord");
            format!("Set suggested moderation kick count: {value}")
        }
        ("suggested-moderation", "period-days") => {
            let Some(value) = integer_option(leaf_options, "period-days") else {
                return super::reply_ephemeral(ctx, command, "A `period-days` option is required.")
                    .await;
            };
            settings
                .heuristic_period_days
                .set(Some(value as i32), "discord");
            format!(
                "Set suggested moderation period: {value} day{}",
                if value == 1 { "" } else { "s" }
            )
        }
        ("outstanding-moderation", "period-days") => {
            let Some(value) = integer_option(leaf_options, "period-days") else {
                return super::reply_ephemeral(ctx, command, "A `period-days` option is required.")
                    .await;
            };
            settings
                .outstanding_period_days
                .set(Some(value as i32), "discord");
            format!(
                "Set outstanding moderation period: {value} day{}",
                if value == 1 { "" } else { "s" }
            )
        }
        ("moderation-summary", "time-of-day") => {
            let next = match next_time_of_day(leaf_options) {
                Ok(next) => next,
                Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
            };
            settings.next_mod_summary.set(Some(next));
            format!(
                "Set mod summary generation time: next summary at <t:{}:f>",
                next.timestamp()
            )
        }
        ("outstanding-moderation", "time-of-day") => {
            let next = match next_time_of_day(leaf_options) {
                Ok(next) => next,
                Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
            };
            settings.next_outstanding_mod.set(Some(next));
            format!(
                "Set outstanding mod summary generation time: next summary at <t:{}:f>",
                next.timestamp()
            )
        }
        _ => return super::reply_ephemeral(ctx, command, "Unknown configuration action.").await,
    };
    super::reply_ephemeral(ctx, command, &message).await
}

fn integer_option(options: &[ResolvedOption<'_>], name: &str) -> Option<i64> {
    options
        .iter()
        .find(|option| option.name == name)
        .and_then(|option| match option.value {
            ResolvedValue::Integer(value) => Some(value),
            _ => None,
        })
}

fn next_time_of_day(options: &[ResolvedOption<'_>]) -> Result<chrono::DateTime<Utc>, String> {
    let zone_name = string_option(options, "time-zone-id")
        .ok_or_else(|| "A `time-zone-id` option is required.".to_string())?;
    let time_text = string_option(options, "time-of-day")
        .ok_or_else(|| "A `time-of-day` option is required.".to_string())?;
    let time = NaiveTime::parse_from_str(time_text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(time_text, "%H:%M:%S"))
        .map_err(|_| format!("Invalid time of day `{time_text}`"))?;
    if let Ok(zone) = zone_name.parse::<chrono_tz::Tz>() {
        return next_in_zone(&zone, time, zone_name);
    }
    if let Some(offset) = parse_fixed_offset(zone_name) {
        return next_in_zone(&offset, time, zone_name);
    }
    Err(format!("Invalid time zone/offset `{zone_name}`"))
}

fn next_in_zone<T: TimeZone>(
    zone: &T,
    time: NaiveTime,
    zone_name: &str,
) -> Result<chrono::DateTime<Utc>, String> {
    let now = Utc::now();
    let local_now = now.with_timezone(zone);
    let local = local_now.date_naive().and_time(time);
    let mut next = zone
        .from_local_datetime(&local)
        .earliest()
        .ok_or_else(|| format!("The local time `{time}` does not exist in `{zone_name}`"))?
        .with_timezone(&Utc);
    if next < now {
        next += chrono::Duration::hours(24);
    }
    Ok(next)
}

fn parse_fixed_offset(value: &str) -> Option<chrono::FixedOffset> {
    let sign = match value.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes) = value[1..].split_once(':')?;
    let hours: i32 = hours.parse().ok()?;
    let minutes: i32 = minutes.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    chrono::FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

fn string_option<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|option| option.name == name)
        .and_then(|option| match option.value {
            ResolvedValue::String(value) => Some(value),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_named_time_zone() {
        let zone: chrono_tz::Tz = "America/New_York".parse().unwrap();
        assert_eq!(zone.name(), "America/New_York");
    }

    #[test]
    fn parses_java_style_fixed_offsets() {
        assert_eq!(
            super::parse_fixed_offset("+05:30")
                .unwrap()
                .local_minus_utc(),
            19_800
        );
        assert_eq!(
            super::parse_fixed_offset("-04:00")
                .unwrap()
                .local_minus_utc(),
            -14_400
        );
        assert!(super::parse_fixed_offset("UTC").is_none());
    }
}
