//! Ticket Tool age-verification configuration commands and modal entry.

use serenity::builder::{
    CreateActionRow, CreateCommand, CreateCommandOption, CreateInputText,
    CreateInteractionResponse, CreateModal,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::channel::ChannelType;
use serenity::model::permissions::Permissions;
use serenity::model::prelude::InputTextStyle;

use crate::data::DiscordConfigKey;

pub const AUTO_RESPONSE_NAME: &str = "set-ticket-tool-auto-response";
pub const MESSAGE_NAME: &str = "set-ticket-age-verify-message";
pub const MESSAGE_MODAL_ID: &str = "settings:ticketToolAgeVerificationMessage";
pub const DEFAULT_REGEX: &str = "(?i).*ticket.*";
pub const DEFAULT_NOTIFY_ROLE: &str = "1479385524786696242";
pub const DEFAULT_MESSAGE: &str = "If your ticket is regarding age verification;\n\nYou have 2 options.\n\nTake a photo of your ID with everything but your birthday covered, with a piece of paper next to it with todays date, the server name, your Discord username, and your VRChat username.\n\nIf you're an NSFW content creator and have a verified Fansly account, you can write within your bio the server name. Once that is done, take a screenshot and also send the link of your profile here.\n\nPlease run `/link-vrchat-account` in this ticket to link your VRChat account to get a group invite after you've verified.\n\nNote: you won't receive a VRChat group invite unless you've been verified and have the verified member role.";

pub fn register_auto_response() -> CreateCommand {
    CreateCommand::new(AUTO_RESPONSE_NAME)
        .description("Configure the Ticket Tool age-verification auto-response")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .add_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "enabled",
            "Whether the auto-response is enabled",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::Channel,
            "discord-category",
            "The Ticket Tool category",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::Role,
            "notify-role",
            "The role to notify",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::String,
            "channel-name-regex",
            "Regex matching ticket channel names",
        ))
}

pub fn register_message() -> CreateCommand {
    CreateCommand::new(MESSAGE_NAME)
        .description("Set or reset the Ticket Tool age-verification message")
        .default_member_permissions(Permissions::MANAGE_GUILD)
}

fn config_string(config: &crate::data::DiscordConfigStore, field: &str, default: &str) -> String {
    config
        .json(field)
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub async fn run_auto_response(
    ctx: &Context,
    command: &CommandInteraction,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is unavailable.").await;
    };
    let mut enabled = config
        .json("ticketToolAutoResponseEnabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let mut category = config
        .json("ticketToolCategorySf")
        .and_then(|value| value.as_str().map(str::to_string));
    let mut notify_role = Some(config_string(
        &config,
        "ticketToolNotifyRoleSf",
        DEFAULT_NOTIFY_ROLE,
    ));
    let mut channel_regex = Some(config_string(
        &config,
        "ticketToolChannelNameRegex",
        DEFAULT_REGEX,
    ));
    for option in command.data.options() {
        match option.value {
            ResolvedValue::Boolean(value) if option.name == "enabled" => enabled = value,
            ResolvedValue::Channel(value) if option.name == "discord-category" => {
                if value.kind != ChannelType::Category {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        "The selected channel isn't a category.",
                    )
                    .await;
                }
                category = Some(value.id.to_string());
            }
            ResolvedValue::Role(value) if option.name == "notify-role" => {
                notify_role = Some(value.id.to_string())
            }
            ResolvedValue::String(value) if option.name == "channel-name-regex" => {
                let trimmed = value.trim();
                if !trimmed.is_empty() && regex::Regex::new(trimmed).is_err() {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        "Invalid channel-name regex. Nothing changed.",
                    )
                    .await;
                }
                channel_regex = (!trimmed.is_empty()).then(|| trimmed.to_string());
            }
            _ => {}
        }
    }
    let fields = [
        ("ticketToolAutoResponseEnabled", serde_json::json!(enabled)),
        ("ticketToolCategorySf", serde_json::json!(category)),
        ("ticketToolNotifyRoleSf", serde_json::json!(notify_role)),
        (
            "ticketToolChannelNameRegex",
            serde_json::json!(channel_regex),
        ),
    ];
    if let Err(err) = config.update_json(fields) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Could not save Ticket Tool configuration: {err}"),
        )
        .await;
    }
    let message = format!(
        "Ticket Tool auto-response is {}.\nCategory: {}\nNotify role: {}\nChannel-name regex: {}",
        if enabled { "enabled" } else { "disabled" },
        category
            .as_deref()
            .map(|id| format!("<#{id}>"))
            .unwrap_or_else(|| "not set".into()),
        notify_role
            .as_deref()
            .map(|id| format!("<@&{id}>"))
            .unwrap_or_else(|| "not set".into()),
        channel_regex
            .as_deref()
            .map(|value| format!("`{value}`"))
            .unwrap_or_else(|| "not set".into())
    );
    super::reply_ephemeral(ctx, command, &message).await
}

pub async fn run_message(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    serenity::builder::CreateInteractionResponseMessage::new()
                        .content("Discord configuration is unavailable.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let current = config_string(&config, "ticketToolAgeVerificationMessage", DEFAULT_MESSAGE);
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new(MESSAGE_MODAL_ID, "Ticket age-verification message").components(
                    vec![CreateActionRow::InputText(
                        CreateInputText::new(
                            InputTextStyle::Paragraph,
                            "Message (leave empty to reset to default)",
                            "message",
                        )
                        .value(current)
                        .required(false)
                        .max_length(4000),
                    )],
                ),
            ),
        )
        .await
}

pub(crate) fn normalized_message(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        DEFAULT_MESSAGE.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blank_message_resets_to_java_default() {
        assert_eq!(normalized_message("  \n"), DEFAULT_MESSAGE);
    }
    #[test]
    fn java_default_regex_compiles() {
        assert!(regex::Regex::new(DEFAULT_REGEX).is_ok());
    }
}
