//! Shared implementation for `/watched-user` and `/watched-avatar`
//! (`ScarletDiscordCommands.WatchedEntity_<E>`, the abstract base both
//! `WatchedUser_`/`WatchedAvatar_` extend), backed by
//! `scarlet_core::watched_entities::WatchedEntities`. See
//! [`super::watched_user`]/[`super::watched_avatar`] for the thin
//! per-entity-kind wrappers, and [`super::watched_group`]'s docs for the
//! simplifications (`entity-priority`'s `-100..=100` range is enforced in
//! the handler because serenity cannot encode a negative option minimum).
//!
//! Ported subcommands: `list`, `add`, `remove`, `view`, `set-critical`,
//! `set-silent`, `set-type`, `set-priority`, `set-message`, `set-tags`,
//! `add-tag`, `remove-tag`, `set-notes`, `remove-menu`, `export`, and
//! `import`. Note Java names the deletion subcommand
//! `remove` here (unlike `watched-group`'s `delete-watched-group`).

use scarlet_vrchat::entity_cache::ALWAYS_PREFER_CACHED;
use serenity::builder::{
    CreateActionRow, CreateAttachment, CreateAutocompleteResponse, CreateCommand,
    CreateCommandOption, CreateInputText, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateModal,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::model::channel::Attachment;
use serenity::model::prelude::InputTextStyle;

use scarlet_core::watched_entities::{WatchedEntities, WatchedEntity};
use scarlet_core::watched_groups::WatchedGroupType;

#[derive(Clone, Copy)]
pub(crate) enum EntityKind {
    User,
    Avatar,
}

impl EntityKind {
    fn singular(&self) -> &'static str {
        match self {
            EntityKind::User => "user",
            EntityKind::Avatar => "avatar",
        }
    }

    fn id_option_name(&self) -> &'static str {
        match self {
            EntityKind::User => "vrchat-user",
            EntityKind::Avatar => "vrchat-avatar",
        }
    }

    fn id_option_description(&self) -> &'static str {
        match self {
            EntityKind::User => "The VRChat user id (usr_XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX)",
            EntityKind::Avatar => {
                "The VRChat avatar id (avtr_XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX)"
            }
        }
    }

    fn url(&self, id: &str) -> String {
        match self {
            EntityKind::User => format!("https://vrchat.com/home/user/{id}"),
            EntityKind::Avatar => format!("https://vrchat.com/home/avatar/{id}"),
        }
    }

    fn plural(&self) -> &'static str {
        match self {
            EntityKind::User => "users",
            EntityKind::Avatar => "avatars",
        }
    }

    fn export_filename(&self) -> &'static str {
        match self {
            EntityKind::User => "watched_users.json",
            EntityKind::Avatar => "watched_avatars.json",
        }
    }

    /// Java's `_getEntityName(_getEntity(id, minEpoch))`. Every call site
    /// that renders a *list* of watched entities passes
    /// `ScarletJsonCache.ALWAYS_PREFER_CACHED`, so after the first lookup
    /// the name comes off disk and costs no API call — which is what keeps
    /// autocomplete from firing one request per watched entity per
    /// keystroke.
    async fn fetch_name_cached(
        &self,
        client: &scarlet_vrchat::client::VrChatClient,
        id: &str,
        min_epoch: i64,
    ) -> Option<String> {
        match self {
            EntityKind::User => client
                .get_user_cached(id, min_epoch)
                .await
                .ok()
                .and_then(|u| u.display_name),
            EntityKind::Avatar => {
                client.get_avatar_cached(id, min_epoch).await.ok().map(|a| a.name)
            }
        }
    }

    /// `fetch_name_cached` with Java's `ALWAYS_FETCH`, for the call sites
    /// that genuinely want current data.
    async fn fetch_name(
        &self,
        client: &scarlet_vrchat::client::VrChatClient,
        id: &str,
    ) -> Option<String> {
        self.fetch_name_cached(client, id, scarlet_vrchat::entity_cache::ALWAYS_FETCH).await
    }

    /// Whether the entity still exists in VRChat -- used only by `add`
    /// (matching Java's `_getEntity(id, minEpoch) == null` existence check).
    async fn exists(&self, client: &scarlet_vrchat::client::VrChatClient, id: &str) -> bool {
        match self {
            EntityKind::User => client.get_user(id).await.is_ok(),
            EntityKind::Avatar => client.get_avatar(id).await.is_ok(),
        }
    }
}

/// Java registers `entity-priority` with `setRequiredRange(-100L, 100L)`, so
/// Discord itself refuses an out-of-range value before the command is ever
/// dispatched. serenity 0.12's `min_int_value` takes a `u64` and so cannot
/// express a negative minimum, which means the range has to be enforced
/// here instead; [`check_priority_range`] rejects with the same effect the
/// user sees in Java (the value simply doesn't take).
const PRIORITY_MIN: i64 = -100;
const PRIORITY_MAX: i64 = 100;

/// `None` when `priority` is inside Java's registered range, otherwise the
/// message to reply with.
fn check_priority_range(priority: i64) -> Option<String> {
    (!(PRIORITY_MIN..=PRIORITY_MAX).contains(&priority)).then(|| {
        format!("`entity-priority` must be between {PRIORITY_MIN} and {PRIORITY_MAX}.")
    })
}

fn group_type_option(name: &str) -> CreateCommandOption {
    let mut option = CreateCommandOption::new(
        CommandOptionType::String,
        name,
        "The type of watched entity",
    )
    .required(true);
    for t in WatchedGroupType::ALL {
        option = option.add_string_choice(t.as_str(), t.as_str());
    }
    option
}

pub(crate) fn register_generic(name: &str, description: &str, kind: EntityKind) -> CreateCommand {
    let id_option = || {
        CreateCommandOption::new(
            CommandOptionType::String,
            kind.id_option_name(),
            kind.id_option_description(),
        )
        .required(true)
        .set_autocomplete(true)
    };

    CreateCommand::new(name)
        .description(description)
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list",
            format!("Lists all watched {}s", kind.singular()),
        ).add_sub_option(super::pagination_option()))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "export",
            format!("Exports watched {} as a JSON file", kind.plural()),
        ))
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "import",
                format!("Imports watched {} from an attached file", kind.plural()),
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Attachment,
                    "import-file",
                    "A watched-entity JSON or legacy CSV file",
                )
                .required(true),
            ),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "remove-menu",
            format!(
                "Remove watched {}s from a dropdown of their names",
                kind.singular()
            ),
        ))
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "add",
                format!("Adds a watched {}", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(group_type_option("entity-type"))
            .add_sub_option(CreateCommandOption::new(
                CommandOptionType::String,
                "entity-tags",
                "A list of tags, separated by one of ',', ';', '/'",
            ))
            // Java: `_entityPriority` ... `.setRequiredRange(-100L, 100L)`.
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "entity-priority",
                    "The priority of this entity (-100 to 100)",
                )
            )
            .add_sub_option(CreateCommandOption::new(
                CommandOptionType::String,
                "entity-message",
                "A message to announce with TTS",
            )),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "remove",
                format!("Removes a watched {}", kind.singular()),
            )
            .add_sub_option(id_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "view",
                format!("Views a {}'s watch information", kind.singular()),
            )
            .add_sub_option(id_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-critical",
                format!("Sets a {}'s critical status", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Boolean,
                    "entity-critical",
                    "The critical status of the entity",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-notes",
                format!("Sets a {}'s notes", kind.singular()),
            )
            .add_sub_option(id_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-silent",
                format!("Sets a {}'s silent status", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Boolean,
                    "entity-silent",
                    "The silent status of the entity",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-type",
                format!("Sets a {}'s watch type", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(group_type_option("entity-type")),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-priority",
                format!("Sets a {}'s priority", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "entity-priority",
                    "The priority of this entity (-100 to 100)",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-message",
                format!("Sets a {}'s TTS announcement message", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "entity-message",
                    "A message to announce with TTS",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-tags",
                format!("Sets a {}'s tags", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "entity-tags",
                    "A list of tags, separated by one of ',', ';', '/'",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "add-tag",
                format!("Adds a tag for a {}", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "entity-tag", "A tag")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "remove-tag",
                format!("Removes a tag from a {}", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "entity-tag", "A tag")
                    .required(true),
            ),
        )
}

/// Java's `_vrchatEntity` autocomplete resolves every watched entity's name
/// with `ScarletJsonCache.ALWAYS_PREFER_CACHED`, so a cached name is used
/// whatever its age and only a never-before-seen entity costs a request.
pub(crate) async fn autocomplete(
    ctx: &Context,
    command: &CommandInteraction,
    entities: std::sync::Arc<WatchedEntities>,
    kind: EntityKind,
) -> serenity::Result<()> {
    let query = command.data.autocomplete().map(|option| option.value.trim().to_lowercase()).unwrap_or_default();
    let client = {
        let slot = ctx.data.read().await.get::<crate::data::VrChatClientKey>().cloned();
        match slot { Some(slot) => slot.read().await.clone(), None => None }
    };
    let mut response = CreateAutocompleteResponse::new();
    let mut choices = 0usize;
    for entity in entities.get_all() {
        if !query.is_empty() && !entity.id.to_lowercase().contains(&query) {
            let name = match &client {
                Some(client) => {
                    kind.fetch_name_cached(client, &entity.id, ALWAYS_PREFER_CACHED).await
                }
                None => None,
            };
            if !name.as_deref().is_some_and(|name| name.to_lowercase().contains(&query)) {
                continue;
            }
            let name = name.unwrap_or_else(|| entity.id.clone());
            response = response.add_string_choice(
                format!("{name} ({})", entity.id).chars().take(100).collect::<String>(),
                entity.id,
            );
        } else {
            let name = match &client {
                Some(client) => {
                    kind.fetch_name_cached(client, &entity.id, ALWAYS_PREFER_CACHED).await
                }
                None => None,
            }
            .unwrap_or_else(|| entity.id.clone());
            response = response.add_string_choice(
                format!("{name} ({})", entity.id).chars().take(100).collect::<String>(),
                entity.id,
            );
        }
        choices += 1;
        if choices >= 25 {
            break;
        }
    }
    command.create_response(&ctx.http, CreateInteractionResponse::Autocomplete(response)).await
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

fn tags_joined(entity: &WatchedEntity) -> String {
    entity.tags.iter().cloned().collect::<Vec<_>>().join(", ")
}

pub(crate) async fn run_generic(
    ctx: &Context,
    command: &CommandInteraction,
    entities: std::sync::Arc<WatchedEntities>,
    kind: EntityKind,
) -> serenity::Result<()> {
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        super::defer(ctx, command).await?;
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };

    if subcommand.name == "set-notes" {
        return run_set_notes_modal(ctx, command, sub_options, &entities, kind).await;
    }
    super::defer(ctx, command).await?;

    match subcommand.name {
        "list" => run_list(ctx, command, sub_options, &entities, kind).await,
        "remove-menu" => run_remove_menu(ctx, command, &entities, kind).await,
        "export" => run_export(ctx, command, &entities, kind).await,
        "import" => run_import(ctx, command, sub_options, &entities, kind).await,
        "add" => run_add(ctx, command, sub_options, &entities, kind).await,
        "remove" => run_remove(ctx, command, sub_options, &entities, kind).await,
        "view" => run_view(ctx, command, sub_options, &entities, kind).await,
        "set-critical" => run_set_critical(ctx, command, sub_options, &entities, kind).await,
        "set-silent" => run_set_silent(ctx, command, sub_options, &entities, kind).await,
        "set-type" => run_set_type(ctx, command, sub_options, &entities, kind).await,
        "set-priority" => run_set_priority(ctx, command, sub_options, &entities, kind).await,
        "set-message" => run_set_message(ctx, command, sub_options, &entities, kind).await,
        "set-tags" => run_set_tags(ctx, command, sub_options, &entities, kind).await,
        "add-tag" => run_add_tag(ctx, command, sub_options, &entities, kind).await,
        "remove-tag" => run_remove_tag(ctx, command, sub_options, &entities, kind).await,
        other => {
            super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await
        }
    }
}

async fn run_export(
    ctx: &Context,
    command: &CommandInteraction,
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    match entities.export_json() {
        Ok(json) => {
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!("Exported watched {}.", kind.plural()))
                        .add_file(CreateAttachment::bytes(json, kind.export_filename())),
                )
                .await?;
            Ok(())
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not export watched {}: {err}", kind.plural()),
            )
            .await
        }
    }
}

async fn run_import(
    ctx: &Context,
    command: &CommandInteraction,
    options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
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
    let filename = attachment.filename.to_lowercase();
    let result = if filename.ends_with(".csv") {
        entities
            .import_legacy_csv(&bytes, true)
            .map(|count| (count, "legacy CSV"))
    } else if filename.ends_with(".json") {
        entities
            .import_json(&bytes, true)
            .map(|count| (count, "JSON"))
    } else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "File `{}` is not importable; use `.json` or `.csv`.",
                attachment.filename
            ),
        )
        .await;
    };
    match result {
        Ok((count, format)) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!(
                    "Successfully imported watched {} {format} ({count} valid entries).",
                    kind.plural()
                ),
            )
            .await
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Exception while importing `{}`: {err}", attachment.filename),
            )
            .await
        }
    }
}

async fn run_remove_menu(
    ctx: &Context,
    command: &CommandInteraction,
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let context = super::vrchat_context(ctx).await.ok();
    let mut entries = Vec::new();
    for entity in entities.get_all() {
        let name = match &context {
            Some(context) => kind
                .fetch_name_cached(&context.client, &entity.id, ALWAYS_PREFER_CACHED)
                .await,
            None => None,
        }
        .unwrap_or_else(|| entity.id.clone());
        entries.push((entity.id, name));
    }
    entries.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    super::watched_remove_menu(ctx, command, kind.singular(), kind.plural(), entries).await
}

async fn run_set_notes_modal(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    serenity::builder::CreateInteractionResponseMessage::new()
                        .content(format!("`{}` is required.", kind.id_option_name()))
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let Some(entity) = entities.get_watched_entity(id) else {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    serenity::builder::CreateInteractionResponseMessage::new()
                        .content(format!("That {} is not watched", kind.singular()))
                        .ephemeral(true),
                ),
            )
            .await;
    };
    let mut input = CreateInputText::new(InputTextStyle::Paragraph, "Notes", "notes")
        .required(false)
        .max_length(1024);
    if let Some(notes) = entity.notes.filter(|notes| !notes.is_empty()) {
        input = input.value(notes);
    }
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new(
                    format!("watched-entity-set-notes:{}:{id}", kind.singular()),
                    "Edit notes",
                )
                .components(vec![CreateActionRow::InputText(input)]),
            ),
        )
        .await
}

async fn run_list(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let all = entities.get_all();
    if all.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("No {}s are watched.", kind.singular()),
        )
        .await;
    }
    let mut lines = Vec::new();
    for entity in &all {
        lines.push(format!(
            "[`{}`](<{}>) — type: {}, priority: {}, critical: {}, silent: {}",
            entity.id,
            kind.url(&entity.id),
            entity.r#type.as_str(),
            entity.priority,
            entity.critical,
            entity.silent
        ));
    }
    let heading = format!("### Watched {}s", kind.singular());
    super::send_pages(
        ctx,
        command,
        super::paginate_entries(&lines, super::entries_per_page(sub_options), "\n", Some(&heading)),
    )
    .await
}

async fn run_add(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    if entities.get_watched_entity(id).is_some() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is already watched", kind.singular()),
        )
        .await;
    }
    let Some(entity_type) =
        find_string(sub_options, "entity-type").and_then(WatchedGroupType::parse)
    else {
        return super::reply_ephemeral(ctx, command, "`entity-type` is required.").await;
    };
    let entity_tags = find_string(sub_options, "entity-tags");
    let entity_priority = find_int(sub_options, "entity-priority").unwrap_or(0);
    if let Some(message) = check_priority_range(entity_priority) {
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    let entity_priority = entity_priority as i32;
    let message = find_string(sub_options, "entity-message");

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if !kind.exists(&context.client, id).await {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} doesn't seem to exist", kind.singular()),
        )
        .await;
    }
    let name = kind
        .fetch_name(&context.client, id)
        .await
        .unwrap_or_else(|| id.to_string());

    let mut entity = WatchedEntity::new(id);
    entity.r#type = entity_type;
    if let Some(entity_tags) = entity_tags {
        entity.tags.clear();
        entity.tags.add_all(split_tags(entity_tags));
    }
    if let Some(message) = message {
        entity.message = Some(message.to_string());
    }
    entity.priority = entity_priority;
    entities.add_watched_entity(entity);

    super::reply_ephemeral(
        ctx,
        command,
        &format!("Added {} [{name}](<{}>)", kind.singular(), kind.url(id)),
    )
    .await
}

async fn run_remove(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    if !entities.remove_watched_entity(id) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    }
    let name = match super::vrchat_context(ctx).await {
        Ok(context) => kind
            .fetch_name_cached(&context.client, id, ALWAYS_PREFER_CACHED)
            .await,
        Err(_) => None,
    }
    .unwrap_or_else(|| id.to_string());
    super::reply_ephemeral(
        ctx,
        command,
        &format!("Removed {} [{name}](<{}>)", kind.singular(), kind.url(id)),
    )
    .await
}

async fn run_view(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    let Some(entity) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    let message = format!(
        "### Watch info for `{}`\ntype: {}\npriority: {}\ncritical: {}\nsilent: {}\nmessage: {}\ntags: {}\nnotes: {}",
        entity.id,
        entity.r#type.as_str(),
        entity.priority,
        entity.critical,
        entity.silent,
        entity.message.as_deref().unwrap_or("none"),
        if entity.tags.is_empty() { "none".to_string() } else { tags_joined(&entity) },
        entity.notes.as_deref().unwrap_or("none"),
    );
    super::reply_ephemeral(ctx, command, &message).await
}

async fn run_set_critical(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let (Some(id), Some(critical)) = (
        find_string(sub_options, kind.id_option_name()),
        find_bool(sub_options, "entity-critical"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "`{}` and `entity-critical` are both required.",
                kind.id_option_name()
            ),
        )
        .await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    if current.critical == critical {
        let message = if critical {
            format!("That {} is already flagged as critical", kind.singular())
        } else {
            format!(
                "That {} is already not flagged as critical",
                kind.singular()
            )
        };
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    entities.update(id, |e| e.critical = critical);
    let message = if critical { "Flagged" } else { "Unflagged" };
    super::reply_ephemeral(
        ctx,
        command,
        &format!("{message} {} as critical", kind.singular()),
    )
    .await
}

async fn run_set_silent(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let (Some(id), Some(silent)) = (
        find_string(sub_options, kind.id_option_name()),
        find_bool(sub_options, "entity-silent"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "`{}` and `entity-silent` are both required.",
                kind.id_option_name()
            ),
        )
        .await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    if current.silent == silent {
        let message = if silent {
            format!("That {} is already flagged as silent", kind.singular())
        } else {
            format!("That {} is already not flagged as silent", kind.singular())
        };
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    entities.update(id, |e| e.silent = silent);
    let message = if silent { "Flagged" } else { "Unflagged" };
    super::reply_ephemeral(
        ctx,
        command,
        &format!("{message} {} as silent", kind.singular()),
    )
    .await
}

async fn run_set_type(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    let Some(entity_type) =
        find_string(sub_options, "entity-type").and_then(WatchedGroupType::parse)
    else {
        return super::reply_ephemeral(ctx, command, "`entity-type` is required.").await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    if current.r#type == entity_type {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "That {} is already marked as {}",
                kind.singular(),
                entity_type.as_str()
            ),
        )
        .await;
    }
    entities.update(id, |e| e.r#type = entity_type);
    super::reply_ephemeral(
        ctx,
        command,
        &format!("Marking {} as {}", kind.singular(), entity_type.as_str()),
    )
    .await
}

async fn run_set_priority(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    let Some(priority) = find_int(sub_options, "entity-priority") else {
        return super::reply_ephemeral(ctx, command, "`entity-priority` is required.").await;
    };
    if let Some(message) = check_priority_range(priority) {
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    if entities.get_watched_entity(id).is_none() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    }
    entities.update(id, |e| e.priority = priority as i32);
    super::reply_ephemeral(
        ctx,
        command,
        &format!("Setting {} priority to {priority}", kind.singular()),
    )
    .await
}

async fn run_set_message(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    let message = find_string(sub_options, "entity-message");
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    let reply = match message {
        None => {
            if current.message.is_none() {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    &format!("That {} already has no message", kind.singular()),
                )
                .await;
            }
            format!(
                "Removing {}'s message (was `{}`)",
                kind.singular(),
                current.message.as_deref().unwrap_or("")
            )
        }
        Some(message) => {
            if current.message.as_deref() == Some(message) {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    &format!("That {}'s message is already exactly that", kind.singular()),
                )
                .await;
            }
            format!(
                "Setting {} TTS announcement to `{message}` (was `{}`)",
                kind.singular(),
                current.message.as_deref().unwrap_or("none")
            )
        }
    };
    entities.update(id, |e| e.message = message.map(str::to_string));
    super::reply_ephemeral(ctx, command, &reply).await
}

async fn run_set_tags(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("`{}` is required.", kind.id_option_name()),
        )
        .await;
    };
    let entity_tags = find_string(sub_options, "entity-tags");
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    let old_tags = tags_joined(&current);
    let reply = match entity_tags {
        None => {
            if current.tags.is_empty() {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    &format!("That {} already has no tags", kind.singular()),
                )
                .await;
            }
            format!("Removing {}'s tags (was `{old_tags}`)", kind.singular())
        }
        Some(entity_tags) => {
            let new_tags = split_tags(entity_tags);
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
                    &format!("That {} already has those exact tags", kind.singular()),
                )
                .await;
            }
            format!(
                "Setting {} tags to `{}` (was `{old_tags}`)",
                kind.singular(),
                new_tags.join(", ")
            )
        }
    };
    let new_tags: Vec<String> = entity_tags.map(split_tags).unwrap_or_default();
    entities.update(id, |e| {
        e.tags.clear();
        e.tags.add_all(new_tags);
    });
    super::reply_ephemeral(ctx, command, &reply).await
}

async fn run_add_tag(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let (Some(id), Some(tag)) = (
        find_string(sub_options, kind.id_option_name()),
        find_string(sub_options, "entity-tag"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "`{}` and `entity-tag` are both required.",
                kind.id_option_name()
            ),
        )
        .await;
    };
    let tag = tag.trim().to_string();
    if entities.get_watched_entity(id).is_none() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    }
    let mut added = false;
    let mut was_empty = false;
    let updated = entities
        .update(id, |e| {
            was_empty = e.tags.is_empty();
            added = e.tags.add(tag.clone());
        })
        .unwrap();
    let message = if added {
        if was_empty {
            format!("Added tag `{tag}` (was empty)")
        } else {
            format!("Added tag `{tag}` (was `{}`)", tags_joined(&updated))
        }
    } else {
        format!("That {} already has that tag", kind.singular())
    };
    super::reply_ephemeral(ctx, command, &message).await
}

async fn run_remove_tag(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let (Some(id), Some(tag)) = (
        find_string(sub_options, kind.id_option_name()),
        find_string(sub_options, "entity-tag"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "`{}` and `entity-tag` are both required.",
                kind.id_option_name()
            ),
        )
        .await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} is not watched", kind.singular()),
        )
        .await;
    };
    if current.tags.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("That {} already has no tags", kind.singular()),
        )
        .await;
    }
    let mut removed = false;
    entities.update(id, |e| removed = e.tags.remove(tag));
    let message = if removed {
        format!("Removed {} tag `{tag}`)", kind.singular())
    } else {
        format!("That {} doesn't have that tag", kind.singular())
    };
    super::reply_ephemeral(ctx, command, &message).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn priority_range_matches_javas_registered_range() {
        // Java: `.setRequiredRange(-100L, 100L)`.
        assert!(check_priority_range(0).is_none());
        assert!(check_priority_range(-100).is_none());
        assert!(check_priority_range(100).is_none());
        assert!(check_priority_range(-101).is_some());
        assert!(check_priority_range(101).is_some());
    }

    use super::*;

    #[test]
    fn split_tags_splits_on_any_delimiter_and_trims() {
        assert_eq!(split_tags("a, b ; c/d"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn entity_kind_urls_differ_by_kind() {
        assert_eq!(
            EntityKind::User.url("usr_1"),
            "https://vrchat.com/home/user/usr_1"
        );
        assert_eq!(
            EntityKind::Avatar.url("avtr_1"),
            "https://vrchat.com/home/avatar/avtr_1"
        );
    }

    #[test]
    fn tags_joined_renders_comma_separated() {
        let mut entity = WatchedEntity::new("usr_1");
        entity.tags.add("a");
        entity.tags.add("b");
        assert_eq!(tags_joined(&entity), "a, b");
    }
}
