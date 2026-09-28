//! Generic Discord editor for the typed settings registry.

use scarlet_core::settings::file_valued::SettingKind;
use serenity::builder::{
    CreateActionRow, CreateAutocompleteResponse, CreateCommand, CreateCommandOption,
    CreateInputText, CreateInteractionResponse, CreateInteractionResponseMessage, CreateModal,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::prelude::InputTextStyle;

use crate::data::SettingsKey;

pub const NAME: &str = "settings";
pub const MODAL_PREFIX: &str = "settings-edit:";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Configures settings")
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "edit", "Configure a setting")
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "setting-id",
                        "The setting id shown in Celeste's Settings tab",
                    )
                    .required(true)
                    .set_autocomplete(true),
                ),
        )
}

pub async fn autocomplete(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let focused = command
        .data
        .autocomplete()
        .map(|option| option.value.to_lowercase())
        .unwrap_or_default();
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let mut response = CreateAutocompleteResponse::new();
    if let Some(settings) = settings {
        let mut candidates = settings.registry.all();
        candidates.sort_by(|left, right| left.name().cmp(right.name()));
        for setting in candidates
            .into_iter()
            .filter(|setting| {
                focused.is_empty()
                    || setting.id().to_lowercase().contains(&focused)
                    || setting.name().to_lowercase().contains(&focused)
            })
            .take(25)
        {
            let label: String = setting.name().chars().take(100).collect();
            response = response.add_string_choice(label, setting.id());
        }
    }
    command
        .create_response(&ctx.http, CreateInteractionResponse::Autocomplete(response))
        .await
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("The `edit` subcommand is required.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("Expected the `edit` subcommand.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let setting_id = sub_options
        .iter()
        .find(|option| option.name == "setting-id")
        .and_then(|option| match option.value {
            ResolvedValue::String(value) => Some(value),
            _ => None,
        });
    let Some(setting_id) = setting_id else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("A `setting-id` option is required.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("Settings are unavailable.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let Some(setting) = settings.registry.get(setting_id) else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(format!("Unknown setting id `{setting_id}`."))
                        .ephemeral(true),
                ),
            )
            .await;
    };
    if matches!(
        setting.kind(),
        SettingKind::Basic | SettingKind::Void { .. }
    ) {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(format!(
                            "The setting {} currently can't be configured via Discord.",
                            setting.name()
                        ))
                        .ephemeral(true),
                ),
            )
            .await;
    }

    let (style, guidance, current) = editor_fields(setting.as_ref());
    let title: String = setting.name().chars().take(45).collect();
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new(format!("{MODAL_PREFIX}{setting_id}"), title).components(vec![
                    CreateActionRow::InputText(
                        CreateInputText::new(style, guidance, "value")
                            .value(current.chars().take(4000).collect::<String>())
                            .required(true)
                            .max_length(4000),
                    ),
                ]),
            ),
        )
        .await
}

fn editor_fields(
    setting: &dyn scarlet_core::settings::file_valued::SettingDescriptor,
) -> (InputTextStyle, String, String) {
    let current = setting.get_as_string();
    match setting.kind() {
        SettingKind::Boolean { .. } => (
            InputTextStyle::Short,
            "Enter true or false".to_string(),
            current,
        ),
        SettingKind::IntegerRange { min, max, .. } => (
            InputTextStyle::Short,
            format!("An integer between {min} and {max}, inclusive"),
            current,
        ),
        SettingKind::Enum { variants, .. } => (
            InputTextStyle::Short,
            format!("One of: {}", variants.join(", "))
                .chars()
                .take(45)
                .collect(),
            current,
        ),
        SettingKind::StringChoice { valid_values } => (
            InputTextStyle::Short,
            format!("One of: {}", valid_values().join(", "))
                .chars()
                .take(45)
                .collect(),
            current,
        ),
        SettingKind::StringPattern { pattern, .. } => (
            InputTextStyle::Short,
            pattern
                .as_ref()
                .map(|pattern| format!("Pattern: {pattern}"))
                .unwrap_or_else(|| setting.name().to_string())
                .chars()
                .take(45)
                .collect(),
            current,
        ),
        SettingKind::StringArrayPattern { pattern, .. } => (
            InputTextStyle::Paragraph,
            pattern
                .as_ref()
                .map(|pattern| format!("One per line; pattern: {pattern}"))
                .unwrap_or_else(|| "One value per line".to_string())
                .chars()
                .take(45)
                .collect(),
            current.split(',').collect::<Vec<_>>().join("\n"),
        ),
        SettingKind::Basic | SettingKind::Void { .. } => unreachable!("filtered before editor"),
    }
}
