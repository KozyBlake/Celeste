//! Server-manager configuration commands backed by Java-compatible config
//! and Celeste settings storage.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::model::channel::{ChannelType, PartialChannel};
use serenity::model::permissions::Permissions;

use crate::data::{DiscordConfigKey, SettingsKey};

pub const TRAINING_NAME: &str = "set-training-channel";
pub const OPS_ALERT_NAME: &str = "set-ops-alert-channel";
pub const ACTION_LOG_NAME: &str = "set-discord-action-log-channel";
pub const ACCOUNT_AGE_NAME: &str = "set-discord-account-age-alert";
pub const AUTO_INVITE_NAME: &str = "set-verification-auto-invite";

#[derive(Clone, Copy)]
pub enum ChannelSetting {
    Training,
    OpsAlert,
    ActionLog,
}

impl ChannelSetting {
    fn name(self) -> &'static str {
        match self {
            Self::Training => TRAINING_NAME,
            Self::OpsAlert => OPS_ALERT_NAME,
            Self::ActionLog => ACTION_LOG_NAME,
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Training => "Set the channel that receives simulated training events",
            Self::OpsAlert => "Set the channel that receives Celeste health alerts",
            Self::ActionLog => "Set the channel used for Celeste moderation action logs",
        }
    }
    fn field(self) -> &'static str {
        match self {
            Self::Training => "trainingChannelSf",
            Self::OpsAlert => "opsAlertChannelSf",
            Self::ActionLog => "discordActionLogChannelSf",
        }
    }
}

pub fn register_channel(setting: ChannelSetting) -> CreateCommand {
    CreateCommand::new(setting.name())
        .description(setting.description())
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .add_option(CreateCommandOption::new(
            CommandOptionType::Channel,
            "discord-text-channel",
            "The channel, or omit to remove it",
        ))
}

fn channel<'a>(options: &'a [ResolvedOption<'a>]) -> Option<&'a PartialChannel> {
    options
        .iter()
        .find(|option| option.name == "discord-text-channel")
        .and_then(|option| match &option.value {
            ResolvedValue::Channel(channel) => Some(*channel),
            _ => None,
        })
}
fn message_channel(kind: ChannelType) -> bool {
    matches!(
        kind,
        ChannelType::Text
            | ChannelType::News
            | ChannelType::NewsThread
            | ChannelType::PublicThread
            | ChannelType::PrivateThread
            | ChannelType::Forum
    )
}

pub async fn run_channel(
    ctx: &Context,
    command: &CommandInteraction,
    setting: ChannelSetting,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let options = command.data.options();
    let selected = channel(&options);
    if let Some(selected) = selected {
        if !message_channel(selected.kind) {
            return super::reply_ephemeral(
                ctx,
                command,
                "The selected channel doesn't support message sending.",
            )
            .await;
        }
    }
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is unavailable.").await;
    };
    if let Err(err) = config.set_string(
        setting.field(),
        selected.map(|channel| channel.id.to_string()).as_deref(),
    ) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Could not save Discord configuration: {err}"),
        )
        .await;
    }
    let message = match (setting, selected) {
        (ChannelSetting::Training, Some(channel)) => format!(
            "Simulated [TRAINING] events will be sent to <#{}>.",
            channel.id
        ),
        (ChannelSetting::Training, None) => {
            "Training channel removed. Simulated events will no longer be posted to Discord.".into()
        }
        (ChannelSetting::OpsAlert, Some(channel)) => {
            format!("Celeste health alerts will be sent to <#{}>.", channel.id)
        }
        (ChannelSetting::OpsAlert, None) => "Ops alert channel removed.".into(),
        (ChannelSetting::ActionLog, Some(channel)) => {
            format!("Discord action logs will be sent to <#{}>.", channel.id)
        }
        (ChannelSetting::ActionLog, None) => "Discord action logging channel removed.".into(),
    };
    super::reply_ephemeral(ctx, command, &message).await
}

const AGE_THRESHOLDS: &[(&str, i64)] = &[
    ("Disabled", 0),
    ("1 minute", 1),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("15 minutes", 15),
    ("30 minutes", 30),
    ("1 hour", 60),
    ("2 hours", 120),
    ("6 hours", 360),
    ("12 hours", 720),
    ("1 day", 1440),
    ("2 days", 2880),
    ("3 days", 4320),
    ("1 week", 10080),
    ("2 weeks", 20160),
    ("1 month", 43200),
    ("3 months", 129600),
    ("6 months", 259200),
    ("1 year", 525600),
];

pub fn register_account_age() -> CreateCommand {
    let mut option = CreateCommandOption::new(
        CommandOptionType::Integer,
        "threshold",
        "The account-age threshold",
    )
    .required(true);
    for (label, minutes) in AGE_THRESHOLDS {
        option = option.add_int_choice(*label, *minutes as i32);
    }
    CreateCommand::new(ACCOUNT_AGE_NAME)
        .description("Set the Discord account-age alert threshold")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .add_option(option)
}

pub async fn run_account_age(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let minutes = command
        .data
        .options()
        .iter()
        .find(|option| option.name == "threshold")
        .and_then(|option| match option.value {
            ResolvedValue::Integer(value) => Some(value),
            _ => None,
        });
    let Some(minutes) =
        minutes.filter(|value| AGE_THRESHOLDS.iter().any(|(_, allowed)| allowed == value))
    else {
        return super::reply_ephemeral(ctx, command, "A valid `threshold` is required.").await;
    };
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is unavailable.").await;
    };
    if let Err(err) = config.set_json("discordAccountAgeAlertMinutes", serde_json::json!(minutes)) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Could not save Discord configuration: {err}"),
        )
        .await;
    }
    let message = if minutes == 0 {
        "Discord account-age alerts disabled.".into()
    } else {
        format!(
            "Discord account-age alerts will flag joins under {}.",
            AGE_THRESHOLDS
                .iter()
                .find(|(_, value)| *value == minutes)
                .unwrap()
                .0
        )
    };
    super::reply_ephemeral(ctx, command, &message).await
}

pub fn register_auto_invite() -> CreateCommand {
    CreateCommand::new(AUTO_INVITE_NAME)
        .description("Configure auto-inviting newly verified members to a VRChat group")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .add_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "enabled",
            "Whether auto-invite is enabled",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::Role,
            "verified-role",
            "The Discord verified role",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::Role,
            "members-role",
            "Role required before showing the link prompt",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::String,
            "vrchat-group-id",
            "The VRChat group id",
        ))
}

fn valid_group_id(value: &str) -> bool {
    regex::Regex::new(r"^grp_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        .expect("static VRChat group id regex")
        .is_match(value)
}

pub async fn run_auto_invite(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return super::reply_ephemeral(ctx, command, "Settings are unavailable.").await;
    };
    let options = command.data.options();
    let mut errors = Vec::new();
    for option in &options {
        match (&option.name, &option.value) {
            (&"enabled", ResolvedValue::Boolean(value)) => {
                if !settings.auto_invite_on_verify.set(Some(*value), "discord") {
                    errors.push("Could not save the enabled state.");
                }
            }
            (&"verified-role", ResolvedValue::Role(role)) => {
                if !settings
                    .verified_role_sf
                    .set(Some(role.id.to_string()), "discord")
                {
                    errors.push("Could not save the verified role.");
                }
            }
            (&"members-role", ResolvedValue::Role(role)) => {
                if !settings
                    .members_role_sf
                    .set(Some(role.id.to_string()), "discord")
                {
                    errors.push("Could not save the members role.");
                }
            }
            (&"vrchat-group-id", ResolvedValue::String(group_id)) => {
                if !valid_group_id(group_id) {
                    errors.push("The supplied value doesn't look like a VRChat group id.");
                } else if !settings
                    .auto_invite_group_id
                    .set(Some((*group_id).to_string()), "discord")
                {
                    errors.push("Could not save the VRChat group id.");
                }
            }
            _ => {}
        }
    }
    let role = settings.verified_role_sf.get();
    let members = settings.members_role_sf.get();
    let configured_group = settings.auto_invite_group_id.get();
    let custom_group = configured_group != "grp_abc12345-6789-0abc-def0-123456789abc"
        && !configured_group.is_empty();
    let effective_group = if custom_group {
        Some(configured_group.clone())
    } else {
        settings
            .store
            .get_object::<String>(scarlet_vrchat::client::GROUP_ID_KEY)
    };
    let mut output = String::new();
    if !errors.is_empty() {
        output.push_str("**Some changes could not be applied:**\n");
        for error in errors {
            output.push_str(&format!("- {error}\n"));
        }
        output.push('\n');
    }
    output.push_str("**Verification auto-invite settings**\n");
    output.push_str(&format!(
        "Enabled: {}\n",
        if settings.auto_invite_on_verify.get() {
            "Yes"
        } else {
            "No"
        }
    ));
    output.push_str(&format!(
        "Verified role: {}\n",
        if role == "1234567890" || role.is_empty() {
            "Not set".into()
        } else {
            format!("<@&{role}>")
        }
    ));
    output.push_str(&format!(
        "Members role (required before link prompt): {}\n",
        if members == "1234567890" || members.is_empty() {
            "Not set (no restriction)".into()
        } else {
            format!("<@&{members}>")
        }
    ));
    output.push_str(&format!(
        "VRChat group: {}{}",
        effective_group
            .as_deref()
            .map(|id| format!("[{id}](<https://vrchat.com/home/group/{id}>)"))
            .unwrap_or_else(|| "Not set".into()),
        if custom_group { "" } else { " (default)" }
    ));
    super::reply_ephemeral(ctx, command, &output).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_age_inventory_matches_java_bounds() {
        assert_eq!(AGE_THRESHOLDS.len(), 19);
        assert_eq!(AGE_THRESHOLDS.first(), Some(&("Disabled", 0)));
        assert_eq!(AGE_THRESHOLDS.last(), Some(&("1 year", 525_600)));
    }

    #[test]
    fn auto_invite_group_id_is_strict() {
        assert!(valid_group_id("grp_12345678-1234-1234-1234-123456789abc"));
        assert!(!valid_group_id("usr_12345678-1234-1234-1234-123456789abc"));
        assert!(!valid_group_id("grp_not-a-uuid"));
    }
}
