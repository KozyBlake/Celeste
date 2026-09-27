//! Port of `/aux-webhooks` and `/set-audit-aux-webhooks`.

use serenity::builder::{
    CreateActionRow, CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};

use crate::data::DiscordConfigKey;

pub const NAME: &str = "aux-webhooks";
pub const ASSIGN_NAME: &str = "set-audit-aux-webhooks";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Configures auxiliary webhooks")
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list",
            "Lists all auxiliary webhooks",
        ))
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "add",
                "Adds an auxiliary webhook",
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "aux-webhook-id",
                    "The internal id to give the webhook",
                )
                .required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "aux-webhook-url",
                    "The URL of the webhook",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "remove",
                "Removes an auxiliary webhook",
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "aux-webhook-id",
                    "The internal id of the webhook",
                )
                .required(true),
            ),
        )
}

pub fn register_assignment() -> CreateCommand {
    CreateCommand::new(ASSIGN_NAME)
        .description("Sets auxiliary webhooks for an audit event type")
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "audit-event-type",
                "The VRChat group audit event type",
            )
            .required(true),
        )
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|option| option.name == name)
        .and_then(|option| match &option.value {
            ResolvedValue::String(value) => Some(*value),
            _ => None,
        })
}

fn valid_webhook_url(value: &str) -> bool {
    regex::Regex::new(r"^https://(?:canary\.|ptb\.)?(?:discord(?:app)?\.com)/api(?:/v\d+)?/webhooks/\d+/[-A-Za-z0-9_]+$")
        .expect("static Discord webhook URL regex").is_match(value)
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is unavailable.").await;
    };
    match subcommand.name {
        "list" => {
            let mut entries: Vec<_> = config
                .string_map("scarletAuxWh2webhookUrl")
                .into_iter()
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            if entries.is_empty() {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "No auxiliary webhooks are configured.",
                )
                .await;
            }
            let lines = entries
                .into_iter()
                .map(|(id, url)| format!("- **{id}** — `{url}`"))
                .collect::<Vec<_>>()
                .join("\n");
            super::send_pages(
                ctx,
                command,
                super::paginate_on_lines(&format!("### Auxiliary webhooks\n{lines}"), 2000),
            )
            .await
        }
        "add" => {
            let (Some(id), Some(url)) = (
                find_string(sub_options, "aux-webhook-id"),
                find_string(sub_options, "aux-webhook-url"),
            ) else {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "`aux-webhook-id` and `aux-webhook-url` are required.",
                )
                .await;
            };
            if config.map_value("scarletAuxWh2webhookUrl", id).is_some() {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "There is already an auxiliary webhook with that id",
                )
                .await;
            }
            if !valid_webhook_url(url) {
                return super::reply_ephemeral(ctx, command, "Invalid webhook url").await;
            }
            match config.set_map_value("scarletAuxWh2webhookUrl", id, Some(url)) {
                Ok(()) => super::reply_ephemeral(ctx, command, "Added auxiliary webhook").await,
                Err(err) => {
                    super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Could not save auxiliary webhook: {err}"),
                    )
                    .await
                }
            }
        }
        "remove" => {
            let Some(id) = find_string(sub_options, "aux-webhook-id") else {
                return super::reply_ephemeral(ctx, command, "`aux-webhook-id` is required.").await;
            };
            if config.map_value("scarletAuxWh2webhookUrl", id).is_none() {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "There is no auxiliary webhook with that id",
                )
                .await;
            }
            match config.set_map_value("scarletAuxWh2webhookUrl", id, None) {
                Ok(()) => super::reply_ephemeral(ctx, command, "Removed auxiliary webhook").await,
                Err(err) => {
                    super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Could not save auxiliary webhook removal: {err}"),
                    )
                    .await
                }
            }
        }
        other => {
            super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await
        }
    }
}

pub async fn run_assignment(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let options = command.data.options();
    let Some(event_type) = find_string(&options, "audit-event-type") else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("`audit-event-type` is required.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    if !super::set_event_channel::valid_audit_type(event_type) {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(format!("`{event_type}` isn't a valid audit log event type"))
                        .ephemeral(true),
                ),
            )
            .await;
    }
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("Discord configuration is unavailable.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let defaults = config.map_strings("auditType2scarletAuxWh", event_type);
    let mut webhooks: Vec<_> = config
        .string_map("scarletAuxWh2webhookUrl")
        .into_keys()
        .collect();
    webhooks.sort();
    webhooks.truncate(25);
    if webhooks.is_empty() {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("No auxiliary webhooks are configured.")
                        .ephemeral(true),
                ),
            )
            .await;
    }
    let max_values = webhooks.len() as u8;
    let options = webhooks
        .into_iter()
        .map(|id| CreateSelectMenuOption::new(&id, &id).default_selection(defaults.contains(&id)))
        .collect();
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(format!("Select auxiliary webhooks for `{event_type}`"))
                    .components(vec![CreateActionRow::SelectMenu(
                        CreateSelectMenu::new(
                            format!("set-audit-aux-webhooks:{event_type}"),
                            CreateSelectMenuKind::String { options },
                        )
                        .min_values(0)
                        .max_values(max_values),
                    )])
                    .ephemeral(true),
            ),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn webhook_url_validation_accepts_discord_and_rejects_other_hosts() {
        assert!(valid_webhook_url(
            "https://discord.com/api/webhooks/123/abc_DEF-456"
        ));
        assert!(valid_webhook_url(
            "https://canary.discordapp.com/api/v10/webhooks/123/token"
        ));
        assert!(!valid_webhook_url(
            "https://example.com/api/webhooks/123/token"
        ));
    }
}
