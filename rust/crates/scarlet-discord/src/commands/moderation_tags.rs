//! Port of `ScarletDiscordCommands.ModerationTags` (`@SlashCmd("moderation-tags")`),
//! backed by `scarlet_core::moderation_tags::ModerationTags`. Ported:
//! `list`, `add`, `delete`.
//!
//! Thread-level `/tag add|remove` lives in [`super::tag`].
//!
//! **Simplified**: `list` renders as plain text via
//! [`super::paginate_on_lines`] instead of Java's paginated per-tag embeds.
//! Not gated (matches Java's `@DefaultPerms`-only `ModerationTags`).

use serenity::builder::{
    CreateAutocompleteResponse, CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseFollowup,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue};

use scarlet_core::moderation_tags::{TagRemoveResult, TagUpdateResult};

use crate::data::ModerationTagsKey;

pub const NAME: &str = "moderation-tags";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Configures custom moderation tags")
        .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "list", "Lists all custom moderation tags"))
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "add", "Adds a custom moderation tag")
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "value",
                        "The internal id of the tag (prefer only a-zA-Z, 0-9, -, _)",
                    )
                    .required(true)
                    .max_length(100),
                )
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "label", "The internal display name of the tag")
                        .required(true)
                        .max_length(100),
                )
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "description",
                    "The internal description of the tag",
                )),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "delete", "Removes a custom moderation tag")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "value", "The internal id of the tag")
                        .required(true)
                        .set_autocomplete(true),
                ),
        )
}

pub async fn autocomplete(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let query = command.data.autocomplete().map(|option| option.value).unwrap_or_default();
    let tags = { ctx.data.read().await.get::<ModerationTagsKey>().cloned() };
    let mut response = CreateAutocompleteResponse::new();
    if let Some(tags) = tags {
        for tag in tags.search_tags(query, 25) {
            let label = if tag.label.is_empty() { &tag.value } else { &tag.label };
            response = response.add_string_choice(
                format!("{label} ({})", tag.value).chars().take(100).collect::<String>(),
                tag.value,
            );
        }
    }
    command.create_response(&ctx.http, CreateInteractionResponse::Autocomplete(response)).await
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(*s),
        _ => None,
    })
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

    let Some(tags) = ({ ctx.data.read().await.get::<ModerationTagsKey>().cloned() }) else {
        return super::reply_ephemeral(ctx, command, "Moderation tags are not available.").await;
    };

    match subcommand.name {
        "list" => run_list(ctx, command, &tags).await,
        "add" => run_add(ctx, command, sub_options, &tags).await,
        "delete" => run_delete(ctx, command, sub_options, &tags).await,
        other => super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await,
    }
}

async fn run_list(
    ctx: &Context,
    command: &CommandInteraction,
    tags: &scarlet_core::moderation_tags::ModerationTags,
) -> serenity::Result<()> {
    let all = tags.get_tags();
    if all.is_empty() {
        return super::reply_ephemeral(ctx, command, "No custom moderation tags exist yet.").await;
    }
    let mut lines = vec!["### Moderation tags".to_string()];
    for tag in &all {
        let label = if tag.label.is_empty() { tag.value.as_str() } else { tag.label.as_str() };
        let mut line = format!("`{}` — {label}", tag.value);
        if !tag.description.is_empty() {
            line.push_str(&format!(": {}", tag.description));
        }
        lines.push(line);
    }
    super::send_pages(ctx, command, super::paginate_on_lines(&lines.join("\n"), 2000)).await
}

async fn run_add(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    tags: &scarlet_core::moderation_tags::ModerationTags,
) -> serenity::Result<()> {
    let Some(value) = find_string(sub_options, "value") else {
        return super::reply_ephemeral(ctx, command, "`value` is required.").await;
    };
    let Some(label) = find_string(sub_options, "label") else {
        return super::reply_ephemeral(ctx, command, "`label` is required.").await;
    };
    let description = find_string(sub_options, "description");

    let message = match tags.add_or_update_tag(value, Some(label), description) {
        TagUpdateResult::MaxTagsReached => {
            format!("Failed to add moderation tag: there are already the maximum of {} moderation tags", scarlet_core::moderation_tags::MAX_TAGS)
        }
        TagUpdateResult::Created => {
            let mut msg = format!("Created moderation tag `{value}`: label=`{label}`");
            if let Some(description) = description {
                msg.push_str(&format!(" description=`{description}`"));
            }
            msg
        }
        TagUpdateResult::Updated => {
            let mut msg = format!("Updated moderation tag `{value}`: label=`{label}`");
            if let Some(description) = description {
                msg.push_str(&format!(" description=`{description}`"));
            }
            msg
        }
    };

    command
        .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(message).ephemeral(true))
        .await
        .map(|_| ())
}

async fn run_delete(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    tags: &scarlet_core::moderation_tags::ModerationTags,
) -> serenity::Result<()> {
    let Some(value) = find_string(sub_options, "value") else {
        return super::reply_ephemeral(ctx, command, "`value` is required.").await;
    };

    let message = match tags.remove_tag(value) {
        TagRemoveResult::Empty => "Failed to delete moderation tag: there are no moderation tags".to_string(),
        TagRemoveResult::NotFound => format!("Failed to add the tag: the tag `{value}` does not exist"),
        TagRemoveResult::Removed => format!("Deleted moderation tag `{value}`"),
    };

    super::reply_ephemeral(ctx, command, &message).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_result_messages_match_java_wording() {
        let dir = tempfile::tempdir().unwrap();
        let tags = scarlet_core::moderation_tags::ModerationTags::open(dir.path(), "moderation_tags.json");
        assert_eq!(tags.add_or_update_tag("harassment", Some("Harassment"), None), TagUpdateResult::Created);
        assert_eq!(tags.add_or_update_tag("harassment", Some("Harassment 2"), None), TagUpdateResult::Updated);
    }
}
