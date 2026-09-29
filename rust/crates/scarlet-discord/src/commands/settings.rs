//! Generic Discord editor for the typed settings registry.

use scarlet_core::settings::file_valued::SettingKind;
use serenity::builder::{
    CreateActionRow, CreateAutocompleteResponse, CreateButton, CreateCommand, CreateCommandOption,
    CreateInputText, CreateInteractionResponse, CreateInteractionResponseMessage, CreateModal,
    CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ComponentInteraction, ComponentInteractionDataKind,
    ResolvedValue,
};
use serenity::model::prelude::{ButtonStyle, InputTextStyle};

use crate::data::SettingsKey;

pub const NAME: &str = "settings";
// Java uses `settings:<id>`; DPerms keys modal-submit rules by the prefix
// before `:`, so this spelling is data compatibility, not presentation.
pub const MODAL_PREFIX: &str = "settings:";

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

    if let Some(values) = choice_values(setting.as_ref()) {
        if values.is_empty() {
            return command
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(format!(
                                "No choices are currently available for {}.",
                                setting.name()
                            ))
                            .ephemeral(true),
                    ),
                )
                .await;
        }
        let current = setting.get_as_string();
        let page = values
            .iter()
            .position(|value| value == &current)
            .map_or(0, |index| index / 25);
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    choice_message(setting_id, setting.name(), &current, &values, page)
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

fn choice_values(
    setting: &dyn scarlet_core::settings::file_valued::SettingDescriptor,
) -> Option<Vec<String>> {
    match setting.kind() {
        SettingKind::Boolean { .. } => Some(vec!["true".to_string(), "false".to_string()]),
        SettingKind::Enum { variants, .. } => Some(variants.clone()),
        SettingKind::StringChoice { valid_values } => Some(valid_values()),
        _ => None,
    }
}

fn choice_message(
    setting_id: &str,
    setting_name: &str,
    current: &str,
    values: &[String],
    requested_page: usize,
) -> CreateInteractionResponseMessage {
    let page_count = values.len().div_ceil(25);
    let current_text = if current.is_empty() { "*(not set)*" } else { current };
    let mut message = CreateInteractionResponseMessage::new().content(format!(
        "Choose a value for **{}**. Current value: `{}`",
        setting_name, current_text
    ));

    if values.len() <= 125 {
        let rows = values
            .chunks(25)
            .enumerate()
            .map(|(page, chunk)| choice_row(setting_id, current, chunk, page, values.len()))
            .collect();
        return message.components(rows);
    }

    // Discord messages allow five action rows. Java switches to a paginator
    // beyond 125 choices; use the same one-page-at-a-time shape here.
    let page = requested_page.min(page_count.saturating_sub(1));
    let start = page * 25;
    let chunk = &values[start..(start + 25).min(values.len())];
    let navigation = CreateActionRow::Buttons(vec![
        CreateButton::new(format!("{MODAL_PREFIX}{setting_id}:page:{}", page.saturating_sub(1)))
            .label("Previous")
            .style(ButtonStyle::Secondary)
            .disabled(page == 0),
        CreateButton::new(format!("{MODAL_PREFIX}{setting_id}:page:{}", page + 1))
            .label("Next")
            .style(ButtonStyle::Secondary)
            .disabled(page + 1 >= page_count),
    ]);
    message = message.content(format!(
        "Choose a value for **{}**. Current value: `{}` — page {} of {}",
        setting_name,
        current_text,
        page + 1,
        page_count
    ));
    message.components(vec![
        choice_row(setting_id, current, chunk, page, values.len()),
        navigation,
    ])
}

fn choice_row(
    setting_id: &str,
    current: &str,
    values: &[String],
    page: usize,
    total: usize,
) -> CreateActionRow {
    let start = page * 25;
    let options = values
        .iter()
        .map(|value| {
            CreateSelectMenuOption::new(
                value.chars().take(100).collect::<String>(),
                value.clone(),
            )
            .default_selection(value == current)
        })
        .collect();
    CreateActionRow::SelectMenu(
        CreateSelectMenu::new(
            format!("{MODAL_PREFIX}{setting_id}:select"),
            CreateSelectMenuKind::String { options },
        )
        .placeholder(format!(
            "Choices {}–{} of {}",
            start + 1,
            (start + values.len()).min(total),
            total
        ))
        .min_values(1)
        .max_values(1),
    )
}

pub async fn component(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some(remainder) = interaction.data.custom_id.strip_prefix(MODAL_PREFIX) else {
        return Ok(());
    };
    let mut parts = remainder.split(':');
    let setting_id = parts.next().unwrap_or_default();
    let action = parts.next().unwrap_or_default();
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return component_reply(ctx, interaction, "Settings are unavailable.").await;
    };
    let Some(setting) = settings.registry.get(setting_id) else {
        return component_reply(
            ctx,
            interaction,
            format!("Unknown setting id `{setting_id}`."),
        )
        .await;
    };
    let Some(values) = choice_values(setting.as_ref()) else {
        return component_reply(ctx, interaction, "That setting does not use a choice menu.").await;
    };

    if action == "page" {
        let page = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0);
        return interaction
            .create_response(
                &ctx.http,
                CreateInteractionResponse::UpdateMessage(choice_message(
                    setting_id,
                    setting.name(),
                    &setting.get_as_string(),
                    &values,
                    page,
                )),
            )
            .await;
    }

    let ComponentInteractionDataKind::StringSelect { values: selected } = &interaction.data.kind
    else {
        return component_reply(ctx, interaction, "Expected one selected value.").await;
    };
    let Some(value) = selected.first().filter(|_| selected.len() == 1) else {
        return component_reply(ctx, interaction, "Expected exactly one selected value.").await;
    };
    if action != "select" || !values.contains(value) {
        return component_reply(
            ctx,
            interaction,
            format!("Invalid value {value} for {}", setting.name()),
        )
        .await;
    }
    if !setting.set_from_string(value, "discord") {
        return component_reply(
            ctx,
            interaction,
            format!("Invalid value {value} for {}", setting.name()),
        )
        .await;
    }
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(format!("Set {} to {}", setting.name(), value))
                    .components(Vec::new()),
            ),
        )
        .await
}

async fn component_reply(
    ctx: &Context,
    interaction: &ComponentInteraction,
    message: impl Into<String>,
) -> serenity::Result<()> {
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(message)
                    .ephemeral(true),
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
            current,
        ),
        SettingKind::Basic | SettingKind::Void { .. } => unreachable!("filtered before editor"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_operation_matches_java_and_existing_permission_files() {
        assert_eq!(MODAL_PREFIX, "settings:");
        assert_eq!(crate::permissions::operation("settings:custom_avatar_search_providers"), "settings");
    }

    #[test]
    fn up_to_125_choices_are_exposed_in_twenty_five_item_rows() {
        let values = (0..60).map(|value| format!("choice-{value}")).collect::<Vec<_>>();
        let message = serde_json::to_value(choice_message(
            "example",
            "Example",
            "choice-27",
            &values,
            0,
        ))
        .unwrap();
        let rows = message["components"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["components"][0]["options"].as_array().unwrap().len(), 25);
        assert_eq!(rows[1]["components"][0]["options"][2]["default"], true);
    }

    #[test]
    fn more_than_125_choices_use_a_bounded_paginator() {
        let values = (0..126).map(|value| format!("choice-{value}")).collect::<Vec<_>>();
        let message = serde_json::to_value(choice_message(
            "example",
            "Example",
            "choice-125",
            &values,
            5,
        ))
        .unwrap();
        let rows = message["components"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["components"][0]["options"].as_array().unwrap().len(), 1);
        assert_eq!(rows[1]["components"].as_array().unwrap().len(), 2);
        assert_eq!(rows[1]["components"][1]["disabled"], true);
    }
}
