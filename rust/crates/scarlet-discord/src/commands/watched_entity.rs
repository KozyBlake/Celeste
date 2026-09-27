//! Shared implementation for `/watched-user` and `/watched-avatar`
//! (`ScarletDiscordCommands.WatchedEntity_<E>`, the abstract base both
//! `WatchedUser_`/`WatchedAvatar_` extend), backed by
//! `scarlet_core::watched_entities::WatchedEntities`. See
//! [`super::watched_user`]/[`super::watched_avatar`] for the thin
//! per-entity-kind wrappers, and [`super::watched_group`]'s docs for the
//! gaps this shares with `/watched-group` (`remove-menu`/`set-notes`/
//! `export`/`import` not ported) and simplifications (no autocomplete,
//! `entity-priority`'s `-100..=100` range not enforced at the Discord-option
//! level).
//!
//! Ported subcommands: `list`, `add`, `remove`, `view`, `set-critical`,
//! `set-silent`, `set-type`, `set-priority`, `set-message`, `set-tags`,
//! `add-tag`, `remove-tag`. Note Java names the deletion subcommand
//! `remove` here (unlike `watched-group`'s `delete-watched-group`).

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue};

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
            EntityKind::Avatar => "The VRChat avatar id (avtr_XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX)",
        }
    }

    fn url(&self, id: &str) -> String {
        match self {
            EntityKind::User => format!("https://vrchat.com/home/user/{id}"),
            EntityKind::Avatar => format!("https://vrchat.com/home/avatar/{id}"),
        }
    }

    async fn fetch_name(&self, client: &scarlet_vrchat::client::VrChatClient, id: &str) -> Option<String> {
        match self {
            EntityKind::User => client.get_user(id).await.ok().and_then(|u| u.display_name),
            EntityKind::Avatar => client.get_avatar(id).await.ok().map(|a| a.name),
        }
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

fn group_type_option(name: &str) -> CreateCommandOption {
    let mut option = CreateCommandOption::new(CommandOptionType::String, name, "The type of watched entity").required(true);
    for t in WatchedGroupType::ALL {
        option = option.add_string_choice(t.as_str(), t.as_str());
    }
    option
}

pub(crate) fn register_generic(name: &str, description: &str, kind: EntityKind) -> CreateCommand {
    let id_option = || CreateCommandOption::new(CommandOptionType::String, kind.id_option_name(), kind.id_option_description()).required(true);

    CreateCommand::new(name)
        .description(description)
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list",
            format!("Lists all watched {}s", kind.singular()),
        ))
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "add", format!("Adds a watched {}", kind.singular()))
                .add_sub_option(id_option())
                .add_sub_option(group_type_option("entity-type"))
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "entity-tags",
                    "A list of tags, separated by one of ',', ';', '/'",
                ))
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "entity-priority",
                    "The priority of this entity (-100 to 100)",
                ))
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "entity-message",
                    "A message to announce with TTS",
                )),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "remove", format!("Removes a watched {}", kind.singular()))
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
                CreateCommandOption::new(CommandOptionType::Boolean, "entity-critical", "The critical status of the entity")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set-silent",
                format!("Sets a {}'s silent status", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Boolean, "entity-silent", "The silent status of the entity")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "set-type", format!("Sets a {}'s watch type", kind.singular()))
                .add_sub_option(id_option())
                .add_sub_option(group_type_option("entity-type")),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "set-priority", format!("Sets a {}'s priority", kind.singular()))
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
                CreateCommandOption::new(CommandOptionType::String, "entity-message", "A message to announce with TTS")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "set-tags", format!("Sets a {}'s tags", kind.singular()))
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
            CreateCommandOption::new(CommandOptionType::SubCommand, "add-tag", format!("Adds a tag for a {}", kind.singular()))
                .add_sub_option(id_option())
                .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "entity-tag", "A tag").required(true)),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "remove-tag",
                format!("Removes a tag from a {}", kind.singular()),
            )
            .add_sub_option(id_option())
            .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "entity-tag", "A tag").required(true)),
        )
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(*s),
        _ => None,
    })
}

fn find_int(options: &[ResolvedOption<'_>], name: &str) -> Option<i64> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::Integer(n) => Some(*n),
        _ => None,
    })
}

fn find_bool(options: &[ResolvedOption<'_>], name: &str) -> Option<bool> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::Boolean(b) => Some(*b),
        _ => None,
    })
}

fn split_tags(input: &str) -> Vec<String> {
    input.split(['/', ',', ';']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
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
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };

    match subcommand.name {
        "list" => run_list(ctx, command, &entities, kind).await,
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
        other => super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await,
    }
}

async fn run_list(ctx: &Context, command: &CommandInteraction, entities: &WatchedEntities, kind: EntityKind) -> serenity::Result<()> {
    let all = entities.get_all();
    if all.is_empty() {
        return super::reply_ephemeral(ctx, command, &format!("No {}s are watched.", kind.singular())).await;
    }
    let mut lines = vec![format!("### Watched {}s", kind.singular())];
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
    super::send_pages(ctx, command, super::paginate_on_lines(&lines.join("\n"), 2000)).await
}

async fn run_add(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    if entities.get_watched_entity(id).is_some() {
        return super::reply_ephemeral(ctx, command, &format!("That {} is already watched", kind.singular())).await;
    }
    let Some(entity_type) = find_string(sub_options, "entity-type").and_then(WatchedGroupType::parse) else {
        return super::reply_ephemeral(ctx, command, "`entity-type` is required.").await;
    };
    let entity_tags = find_string(sub_options, "entity-tags");
    let entity_priority = find_int(sub_options, "entity-priority").unwrap_or(0) as i32;
    let message = find_string(sub_options, "entity-message");

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if !kind.exists(&context.client, id).await {
        return super::reply_ephemeral(ctx, command, &format!("That {} doesn't seem to exist", kind.singular())).await;
    }
    let name = kind.fetch_name(&context.client, id).await.unwrap_or_else(|| id.to_string());

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

    super::reply_ephemeral(ctx, command, &format!("Added {} [{name}](<{}>)", kind.singular(), kind.url(id))).await
}

async fn run_remove(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    if !entities.remove_watched_entity(id) {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    }
    let name = match super::vrchat_context(ctx).await {
        Ok(context) => kind.fetch_name(&context.client, id).await,
        Err(_) => None,
    }
    .unwrap_or_else(|| id.to_string());
    super::reply_ephemeral(ctx, command, &format!("Removed {} [{name}](<{}>)", kind.singular(), kind.url(id))).await
}

async fn run_view(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    let Some(entity) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
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
    let (Some(id), Some(critical)) = (find_string(sub_options, kind.id_option_name()), find_bool(sub_options, "entity-critical"))
    else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` and `entity-critical` are both required.", kind.id_option_name()))
            .await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    };
    if current.critical == critical {
        let message = if critical {
            format!("That {} is already flagged as critical", kind.singular())
        } else {
            format!("That {} is already not flagged as critical", kind.singular())
        };
        return super::reply_ephemeral(ctx, command, &message).await;
    }
    entities.update(id, |e| e.critical = critical);
    let message = if critical { "Flagged" } else { "Unflagged" };
    super::reply_ephemeral(ctx, command, &format!("{message} {} as critical", kind.singular())).await
}

async fn run_set_silent(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let (Some(id), Some(silent)) = (find_string(sub_options, kind.id_option_name()), find_bool(sub_options, "entity-silent")) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` and `entity-silent` are both required.", kind.id_option_name()))
            .await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
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
    super::reply_ephemeral(ctx, command, &format!("{message} {} as silent", kind.singular())).await
}

async fn run_set_type(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    let Some(entity_type) = find_string(sub_options, "entity-type").and_then(WatchedGroupType::parse) else {
        return super::reply_ephemeral(ctx, command, "`entity-type` is required.").await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    };
    if current.r#type == entity_type {
        return super::reply_ephemeral(ctx, command, &format!("That {} is already marked as {}", kind.singular(), entity_type.as_str()))
            .await;
    }
    entities.update(id, |e| e.r#type = entity_type);
    super::reply_ephemeral(ctx, command, &format!("Marking {} as {}", kind.singular(), entity_type.as_str())).await
}

async fn run_set_priority(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    let Some(priority) = find_int(sub_options, "entity-priority") else {
        return super::reply_ephemeral(ctx, command, "`entity-priority` is required.").await;
    };
    if entities.get_watched_entity(id).is_none() {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    }
    entities.update(id, |e| e.priority = priority as i32);
    super::reply_ephemeral(ctx, command, &format!("Setting {} priority to {priority}", kind.singular())).await
}

async fn run_set_message(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    entities: &WatchedEntities,
    kind: EntityKind,
) -> serenity::Result<()> {
    let Some(id) = find_string(sub_options, kind.id_option_name()) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    let message = find_string(sub_options, "entity-message");
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    };
    let reply = match message {
        None => {
            if current.message.is_none() {
                return super::reply_ephemeral(ctx, command, &format!("That {} already has no message", kind.singular())).await;
            }
            format!("Removing {}'s message (was `{}`)", kind.singular(), current.message.as_deref().unwrap_or(""))
        }
        Some(message) => {
            if current.message.as_deref() == Some(message) {
                return super::reply_ephemeral(ctx, command, &format!("That {}'s message is already exactly that", kind.singular()))
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
        return super::reply_ephemeral(ctx, command, &format!("`{}` is required.", kind.id_option_name())).await;
    };
    let entity_tags = find_string(sub_options, "entity-tags");
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    };
    let old_tags = tags_joined(&current);
    let reply = match entity_tags {
        None => {
            if current.tags.is_empty() {
                return super::reply_ephemeral(ctx, command, &format!("That {} already has no tags", kind.singular())).await;
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
                return super::reply_ephemeral(ctx, command, &format!("That {} already has those exact tags", kind.singular())).await;
            }
            format!("Setting {} tags to `{}` (was `{old_tags}`)", kind.singular(), new_tags.join(", "))
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
    let (Some(id), Some(tag)) = (find_string(sub_options, kind.id_option_name()), find_string(sub_options, "entity-tag")) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` and `entity-tag` are both required.", kind.id_option_name())).await;
    };
    let tag = tag.trim().to_string();
    if entities.get_watched_entity(id).is_none() {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
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
    let (Some(id), Some(tag)) = (find_string(sub_options, kind.id_option_name()), find_string(sub_options, "entity-tag")) else {
        return super::reply_ephemeral(ctx, command, &format!("`{}` and `entity-tag` are both required.", kind.id_option_name())).await;
    };
    let Some(current) = entities.get_watched_entity(id) else {
        return super::reply_ephemeral(ctx, command, &format!("That {} is not watched", kind.singular())).await;
    };
    if current.tags.is_empty() {
        return super::reply_ephemeral(ctx, command, &format!("That {} already has no tags", kind.singular())).await;
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
    use super::*;

    #[test]
    fn split_tags_splits_on_any_delimiter_and_trims() {
        assert_eq!(split_tags("a, b ; c/d"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn entity_kind_urls_differ_by_kind() {
        assert_eq!(EntityKind::User.url("usr_1"), "https://vrchat.com/home/user/usr_1");
        assert_eq!(EntityKind::Avatar.url("avtr_1"), "https://vrchat.com/home/avatar/avtr_1");
    }

    #[test]
    fn tags_joined_renders_comma_separated() {
        let mut entity = WatchedEntity::new("usr_1");
        entity.tags.add("a");
        entity.tags.add("b");
        assert_eq!(tags_joined(&entity), "a, b");
    }
}
