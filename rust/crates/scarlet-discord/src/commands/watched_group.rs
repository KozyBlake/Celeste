//! Port of (part of) `ScarletDiscordCommands.WatchedGroup_`
//! (`@SlashCmd("watched-group")`), backed by
//! `scarlet_core::watched_groups::WatchedGroups`. Ported: `list`, `add`,
//! `delete-watched-group`, `view`, `set-critical`, `set-silent`, `set-type`,
//! `set-priority`, `set-message`, `set-notes`, `set-tags`, `add-tag`,
//! `remove-tag`, `export`, `import`, and the component-backed `remove-menu`.
//!
//! **Simplified**: `vrchat-group` is a plain string option rather than
//! Java's autocomplete-resolved id (see the `commands` module docs). Every
//! Java handler here also calls `this.vrchatGroup(event, groupId)`, which
//! only fetches the group to *cache* it for that autocomplete's "last
//! edited" suggestion -- with no autocomplete wired up, that call has no
//! Rust-observable effect, so it's dropped everywhere except where a
//! handler's reply actually needs the group's name (`add`/
//! `delete-watched-group`/`view`), where `vrchat_context`'s client is used
//! to fetch it directly. `group-priority`'s `-100..=100` range (Java's
//! `setRequiredRange(-100L, 100L)`) is enforced in the handler because
//! serenity 0.12 can't represent a negative option minimum. Not gated (matches Java's
//! `@DefaultPerms`-only `WatchedGroup_`).

use serenity::builder::{
    CreateActionRow, CreateAttachment, CreateCommand, CreateCommandOption, CreateInputText,
    CreateInteractionResponse, CreateInteractionResponseFollowup, CreateModal,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::model::channel::Attachment;
use serenity::model::prelude::InputTextStyle;

use scarlet_core::watched_groups::{WatchedGroup, WatchedGroupType};

use crate::data::WatchedGroupsKey;

pub const NAME: &str = "watched-group";

fn group_id_option() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::String,
        "vrchat-group",
        "The VRChat group id (grp_XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX)",
    )
    .required(true)
    .set_autocomplete(true)
}

fn group_type_option(name: &str) -> CreateCommandOption {
    let mut option =
        CreateCommandOption::new(CommandOptionType::String, name, "The type of watched group")
            .required(true);
    for t in WatchedGroupType::ALL {
        option = option.add_string_choice(t.as_str(), t.as_str());
    }
    option
}

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Configures watched groups")
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list",
            "Lists all watched groups",
        ).add_sub_option(super::pagination_option()))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "remove-menu",
            "Remove watched groups from a dropdown of their names",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "export",
            "Exports watched groups as a JSON file",
        ))
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "import",
                "Imports watched groups from an attached file",
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Attachment,
                    "import-file",
                    "A watched-groups JSON or legacy CSV file",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "add", "Adds a watched group")
                .add_sub_option(group_id_option())
                .add_sub_option(group_type_option("group-type"))
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "group-tags",
                    "A list of tags, separated by one of ',', ';', '/'",
                ))
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "group-priority",
                    "The priority of this group (-100 to 100)",
                ))
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "message",
                    "A message to announce with TTS",
                )),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "delete-watched-group",
                "Removes a watched group",
            )
            .add_sub_option(group_id_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "view",
                "Views a group's watch information",
            )
            .add_sub_option(group_id_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-critical",
                "Sets a group's critical status",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Boolean,
                    "critical",
                    "The critical status of the group",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-silent",
                "Sets a group's silent status",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Boolean,
                    "silent",
                    "The silent status of the group",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-type",
                "Sets a group's watch type",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(group_type_option("group-type")),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-priority",
                "Sets a group's priority",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "group-priority",
                    "The priority of this group (-100 to 100)",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-message",
                "Sets a group's TTS announcement message",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "message",
                    "A message to announce with TTS",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-notes",
                "Sets a group's notes",
            )
            .add_sub_option(group_id_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-tags",
                "Sets a group's tags",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "group-tags",
                    "A list of tags, separated by one of ',', ';', '/'",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "add-tag",
                "Adds a tag for a group",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "group-tag", "A tag")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "remove-tag",
                "Removes a tag from a group",
            )
            .add_sub_option(group_id_option())
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "group-tag", "A tag")
                    .required(true),
            ),
        )
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            ResolvedValue::String(s) => Some(*s),
            _ => None,
        })
}

fn find_int(options: &[ResolvedOption<'_>], name: &str) -> Option<i64> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            ResolvedValue::Integer(n) => Some(*n),
            _ => None,
        })
}

fn find_bool(options: &[ResolvedOption<'_>], name: &str) -> Option<bool> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            ResolvedValue::Boolean(b) => Some(*b),
            _ => None,
        })
}

fn find_attachment<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a Attachment> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            ResolvedValue::Attachment(attachment) => Some(*attachment),
            _ => None,
        })
}

fn split_tags(input: &str) -> Vec<String> {
    input
        .split(['/', ',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

const PRIORITY_MIN: i64 = -100;
const PRIORITY_MAX: i64 = 100;

fn check_priority_range(priority: i64) -> Option<String> {
    (!(PRIORITY_MIN..=PRIORITY_MAX).contains(&priority)).then(|| {
        format!("`group-priority` must be between {PRIORITY_MIN} and {PRIORITY_MAX}.")
    })
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };

    let Some(watched) = ({ ctx.data.read().await.get::<WatchedGroupsKey>().cloned() }) else {
        super::defer(ctx, command).await?;
        return super::reply_ephemeral(ctx, command, "Watched groups are not available.").await;
    };

    if subcommand.name == "set-notes" {
        return run_set_notes_modal(ctx, command, sub_options, &watched).await;
    }
    super::defer(ctx, command).await?;

    match subcommand.name {
        "list" => run_list(ctx, command, sub_options, &watched).await,
        "remove-menu" => run_remove_menu(ctx, command, &watched).await,
        "export" => run_export(ctx, command, &watched).await,
        "import" => run_import(ctx, command, sub_options, &watched).await,
        "add" => run_add(ctx, command, sub_options, &watched).await,
        "delete-watched-group" => run_delete(ctx, command, sub_options, &watched).await,
        "view" => run_view(ctx, command, sub_options, &watched).await,
        "set-critical" => run_set_critical(ctx, command, sub_options, &watched).await,
        "set-silent" => run_set_silent(ctx, command, sub_options, &watched).await,
        "set-type" => run_set_type(ctx, command, sub_options, &watched).await,
        "set-priority" => run_set_priority(ctx, command, sub_options, &watched).await,
        "set-message" => run_set_message(ctx, command, sub_options, &watched).await,
        "set-tags" => run_set_tags(ctx, command, sub_options, &watched).await,
        "add-tag" => run_add_tag(ctx, command, sub_options, &watched).await,
        "remove-tag" => run_remove_tag(ctx, command, sub_options, &watched).await,
        other => {
            super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await
        }
    }
}

async fn run_export(
    ctx: &Context,
    command: &CommandInteraction,
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    match watched.export_json() {
        Ok(json) => {
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content("Exported watched groups.")
                        .add_file(CreateAttachment::bytes(json, "watched_groups.json")),
                )
                .await?;
            Ok(())
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not export watched groups: {err}"),
            )
            .await
        }
    }
}

async fn run_import(
    ctx: &Context,
    command: &CommandInteraction,
    options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(attachment) = find_attachment(options, "import-file") else {
        return super::reply_ephemeral(ctx, command, "`import-file` is required.").await;
    };
    let bytes = match attachment.download().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not download `{}`: {err}", attachment.filename),
            )
            .await
        }
    };
    let message = import_attachment(watched, &attachment.filename, &bytes);
    super::reply_ephemeral(ctx, command, &message).await
}

pub(crate) fn import_attachment(
    watched: &scarlet_core::watched_groups::WatchedGroups,
    filename: &str,
    bytes: &[u8],
) -> String {
    let lower = filename.to_lowercase();
    let result = if lower.ends_with(".csv") {
        watched.import_legacy_csv(bytes, true).map(|count| (count, "legacy CSV"))
    } else if lower.ends_with(".json") {
        watched.import_json(bytes, true).map(|count| (count, "JSON"))
    } else {
        return format!("File `{filename}` is not importable; use `.json` or `.csv`.");
    };
    match result {
        Ok((count, format)) => format!("Successfully imported watched groups {format} ({count} valid entries)."),
        Err(error) => format!("Exception while importing `{filename}`: {error}"),
    }
}

async fn run_remove_menu(
    ctx: &Context,
    command: &CommandInteraction,
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let context = super::vrchat_context(ctx).await.ok();
    let mut entries = Vec::new();
    for group in watched.get_all() {
        let name = match &context {
            Some(context) => context
                .client
                .get_group(&group.id, false)
                .await
                .ok()
                .map(|group| group.name),
            None => None,
        }
        .unwrap_or_else(|| group.id.clone());
        entries.push((group.id, name));
    }
    entries.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    super::watched_remove_menu(ctx, command, "group", "groups", entries).await
}

async fn run_set_notes_modal(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    serenity::builder::CreateInteractionResponseMessage::new()
                        .content("`vrchat-group` is required.")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let Some(group) = watched.get_watched_group(group_id) else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    serenity::builder::CreateInteractionResponseMessage::new()
                        .content("That group is not watched")
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let mut input = CreateInputText::new(InputTextStyle::Paragraph, "Notes", "notes")
        .required(false)
        .max_length(1024);
    if let Some(notes) = group.notes.filter(|notes| !notes.is_empty()) {
        input = input.value(notes);
    }
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new(format!("watched-group-set-notes:{group_id}"), "Edit notes")
                    .components(vec![CreateActionRow::InputText(input)]),
            ),
        )
        .await
}

fn tags_joined(group: &WatchedGroup) -> String {
    group.tags.iter().cloned().collect::<Vec<_>>().join(", ")
}

async fn run_list(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let all = watched.get_all();
    if all.is_empty() {
        return super::reply_ephemeral(ctx, command, "No groups are watched.").await;
    }
    let mut lines = Vec::new();
    for group in &all {
        lines.push(format!(
            "[`{}`](<https://vrchat.com/home/group/{}>) — type: {}, priority: {}, critical: {}, silent: {}",
            group.id,
            group.id,
            group.r#type.as_str(),
            group.priority,
            group.critical,
            group.silent
        ));
    }
    super::send_pages(
        ctx,
        command,
        super::paginate_entries(
            &lines,
            super::entries_per_page(sub_options),
            "\n",
            Some("### Watched groups"),
        ),
    )
    .await
}

async fn run_add(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    if watched.get_watched_group(group_id).is_some() {
        return super::reply_ephemeral(ctx, command, "That group is already watched").await;
    }
    let Some(group_type) = find_string(sub_options, "group-type").and_then(WatchedGroupType::parse)
    else {
        return super::reply_ephemeral(ctx, command, "`group-type` is required.").await;
    };
    let group_tags = find_string(sub_options, "group-tags");
    let group_priority = find_int(sub_options, "group-priority").unwrap_or(0);
    if let Some(message) = check_priority_range(group_priority) {
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    let message = find_string(sub_options, "message");

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let group_name = match context.client.get_group(group_id, false).await {
        Ok(group) => group.name,
        Err(_) => {
            return super::reply_ephemeral(ctx, command, "That group doesn't seem to exist").await
        }
    };

    let mut watched_group = WatchedGroup::new(group_id);
    watched_group.r#type = group_type;
    if let Some(group_tags) = group_tags {
        watched_group.tags.clear();
        watched_group.tags.add_all(split_tags(group_tags));
    }
    if let Some(message) = message {
        watched_group.message = Some(message.to_string());
    }
    watched_group.priority = group_priority as i32;
    watched.add_watched_group(watched_group);

    super::reply_ephemeral(
        ctx,
        command,
        &format!("Added group [{group_name}](<https://vrchat.com/home/group/{group_id}>)"),
    )
    .await
}

async fn run_delete(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    if !watched.remove_watched_group(group_id) {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    }
    let name = match super::vrchat_context(ctx).await {
        Ok(context) => context
            .client
            .get_group(group_id, false)
            .await
            .ok()
            .map(|g| g.name),
        Err(_) => None,
    };
    let name = name.unwrap_or_else(|| group_id.to_string());
    super::reply_ephemeral(
        ctx,
        command,
        &format!("Removed group [{name}](<https://vrchat.com/home/group/{group_id}>)"),
    )
    .await
}

async fn run_view(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    let Some(group) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    let message = format!(
        "### Watch info for `{}`\ntype: {}\npriority: {}\ncritical: {}\nsilent: {}\nmessage: {}\ntags: {}\nnotes: {}",
        group.id,
        group.r#type.as_str(),
        group.priority,
        group.critical,
        group.silent,
        group.message.as_deref().unwrap_or("none"),
        if group.tags.is_empty() { "none".to_string() } else { tags_joined(&group) },
        group.notes.as_deref().unwrap_or("none"),
    );
    super::reply_ephemeral(ctx, command, &message).await
}

async fn run_set_critical(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let (Some(group_id), Some(critical)) = (
        find_string(sub_options, "vrchat-group"),
        find_bool(sub_options, "critical"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`vrchat-group` and `critical` are both required.",
        )
        .await;
    };
    let Some(current) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    if current.critical == critical {
        let message = if critical {
            "That group is already flagged as critical"
        } else {
            "That group is already not flagged as critical"
        };
        return super::reply_ephemeral(ctx, command, message).await;
    }
    watched.update(group_id, |g| g.critical = critical);
    let message = if critical {
        "Flagged group as critical"
    } else {
        "Unflagged group as critical"
    };
    super::reply_ephemeral(ctx, command, message).await
}

async fn run_set_silent(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let (Some(group_id), Some(silent)) = (
        find_string(sub_options, "vrchat-group"),
        find_bool(sub_options, "silent"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`vrchat-group` and `silent` are both required.",
        )
        .await;
    };
    let Some(current) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    if current.silent == silent {
        let message = if silent {
            "That group is already flagged as silent"
        } else {
            "That group is already not flagged as silent"
        };
        return super::reply_ephemeral(ctx, command, message).await;
    }
    watched.update(group_id, |g| g.silent = silent);
    let message = if silent {
        "Flagged group as silent"
    } else {
        "Unflagged group as silent"
    };
    super::reply_ephemeral(ctx, command, message).await
}

async fn run_set_type(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    let Some(group_type) = find_string(sub_options, "group-type").and_then(WatchedGroupType::parse)
    else {
        return super::reply_ephemeral(ctx, command, "`group-type` is required.").await;
    };
    let Some(current) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    if current.r#type == group_type {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That group is already marked as {}", group_type.as_str()),
        )
        .await;
    }
    watched.update(group_id, |g| g.r#type = group_type);
    super::reply_ephemeral(
        ctx,
        command,
        &format!("Marking group as {}", group_type.as_str()),
    )
    .await
}

async fn run_set_priority(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    let Some(priority) = find_int(sub_options, "group-priority") else {
        return super::reply_ephemeral(ctx, command, "`group-priority` is required.").await;
    };
    if let Some(message) = check_priority_range(priority) {
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    if watched.get_watched_group(group_id).is_none() {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    }
    watched.update(group_id, |g| g.priority = priority as i32);
    super::reply_ephemeral(
        ctx,
        command,
        &format!("Setting group priority to {priority}"),
    )
    .await
}

async fn run_set_message(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    let message = find_string(sub_options, "message");
    let Some(current) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    let reply = match message {
        None => {
            if current.message.is_none() {
                return super::reply_ephemeral(ctx, command, "That group already has no message")
                    .await;
            }
            format!(
                "Removing group's message (was `{}`)",
                current.message.as_deref().unwrap_or("")
            )
        }
        Some(message) => {
            if current.message.as_deref() == Some(message) {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "That group's message is already exactly that",
                )
                .await;
            }
            format!(
                "Setting group TTS announcement to `{message}` (was `{}`)",
                current.message.as_deref().unwrap_or("none")
            )
        }
    };
    watched.update(group_id, |g| g.message = message.map(str::to_string));
    super::reply_ephemeral(ctx, command, &reply).await
}

async fn run_set_tags(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let Some(group_id) = find_string(sub_options, "vrchat-group") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-group` is required.").await;
    };
    let group_tags = find_string(sub_options, "group-tags");
    let Some(current) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    let old_tags = tags_joined(&current);
    let reply = match group_tags {
        None => {
            if current.tags.is_empty() {
                return super::reply_ephemeral(ctx, command, "That group already has no tags")
                    .await;
            }
            format!("Removing group's tags (was `{old_tags}`)")
        }
        Some(group_tags) => {
            let new_tags = split_tags(group_tags);
            let unchanged = {
                let mut a: Vec<&String> = current.tags.iter().collect();
                let mut b: Vec<&String> = new_tags.iter().collect();
                a.sort();
                b.sort();
                a.dedup();
                b.dedup();
                a == b
            };
            if unchanged {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "That group already has those exact tags",
                )
                .await;
            }
            format!(
                "Setting group tags to `{}` (was `{old_tags}`)",
                new_tags.join(", ")
            )
        }
    };
    let new_tags: Vec<String> = group_tags.map(split_tags).unwrap_or_default();
    watched.update(group_id, |g| {
        g.tags.clear();
        g.tags.add_all(new_tags);
    });
    super::reply_ephemeral(ctx, command, &reply).await
}

async fn run_add_tag(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let (Some(group_id), Some(tag)) = (
        find_string(sub_options, "vrchat-group"),
        find_string(sub_options, "group-tag"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`vrchat-group` and `group-tag` are both required.",
        )
        .await;
    };
    let tag = tag.trim().to_string();
    if watched.get_watched_group(group_id).is_none() {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    }
    let mut added = false;
    let mut was_empty = false;
    let updated = watched
        .update(group_id, |g| {
            was_empty = g.tags.is_empty();
            added = g.tags.add(tag.clone());
        })
        .unwrap();
    let message = if added {
        if was_empty {
            format!("Added tag `{tag}` (was empty)")
        } else {
            format!("Added tag `{tag}` (was `{}`)", tags_joined(&updated))
        }
    } else {
        "That group already has that".to_string()
    };
    super::reply_ephemeral(ctx, command, &message).await
}

async fn run_remove_tag(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    watched: &scarlet_core::watched_groups::WatchedGroups,
) -> serenity::Result<()> {
    let (Some(group_id), Some(tag)) = (
        find_string(sub_options, "vrchat-group"),
        find_string(sub_options, "group-tag"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`vrchat-group` and `group-tag` are both required.",
        )
        .await;
    };
    let Some(current) = watched.get_watched_group(group_id) else {
        return super::reply_ephemeral(ctx, command, "That group is not watched").await;
    };
    if current.tags.is_empty() {
        return super::reply_ephemeral(ctx, command, "That group has no tags").await;
    }
    let mut removed = false;
    watched.update(group_id, |g| removed = g.tags.remove(tag));
    let message = if removed {
        format!("Removed tag `{tag}`")
    } else {
        format!("That group doesn't have the tag `{tag}`")
    };
    super::reply_ephemeral(ctx, command, &message).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_tags_splits_on_any_delimiter_and_trims() {
        assert_eq!(split_tags("a, b ; c/d"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn split_tags_drops_empty_entries() {
        assert_eq!(split_tags("a,,b"), vec!["a", "b"]);
    }

    #[test]
    fn attachment_import_reports_json_and_rejects_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let watched = scarlet_core::watched_groups::WatchedGroups::open(
            dir.path(),
            "watched_groups.json",
        );
        let result = import_attachment(&watched, "groups.json", br#"[{"id":"grp_one"}]"#);
        assert!(result.contains("Successfully imported watched groups JSON (1 valid entries)"));
        assert!(watched.get_watched_group("grp_one").is_some());
        assert!(import_attachment(&watched, "groups.txt", b"ignored").contains("not importable"));
    }

    #[test]
    fn tags_joined_renders_comma_separated() {
        let mut group = WatchedGroup::new("grp_1");
        group.tags.add("a");
        group.tags.add("b");
        assert_eq!(tags_joined(&group), "a, b");
    }

    #[test]
    fn group_priority_rejects_values_discord_rejects_in_java() {
        assert!(check_priority_range(-100).is_none());
        assert!(check_priority_range(100).is_none());
        assert!(check_priority_range(-101).is_some());
        assert!(check_priority_range(101).is_some());
    }
}
