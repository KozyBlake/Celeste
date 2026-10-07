//! Port of Java's generated `/scarlet-discord-permissions` command, exposed
//! by Celeste as `/celeste-discord-permissions`.

use serenity::builder::{
    CreateAutocompleteResponse, CreateCommand, CreateCommandOption, CreateInteractionResponse,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::model::mention::Mentionable;

use crate::data::DiscordPermissionsKey;
use crate::permissions::{PermissionTarget, PermissionType};

pub const NAME: &str = "celeste-discord-permissions";

pub fn register() -> CreateCommand {
    let target = || {
        CreateCommandOption::new(
            CommandOptionType::Mentionable,
            "target",
            "The target of the permission",
        )
        .required(true)
    };
    let kind = || {
        let mut option = CreateCommandOption::new(
            CommandOptionType::String,
            "type",
            "The type of the permission",
        )
        .required(true);
        for kind in PermissionType::ALL {
            option = option.add_string_choice(kind.display(), kind.value());
        }
        option
    };
    let set = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "set",
        "Sets the value of the permission for a specific target",
    )
    .add_sub_option(target())
    .add_sub_option(kind())
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "name",
            "The name of the permission",
        )
        .required(true)
        .set_autocomplete(true),
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "value",
            "The value of the permission",
        )
        .required(true)
        .add_string_choice("Default (fall through)", "null")
        .add_string_choice("Allow", "true")
        .add_string_choice("Deny", "false"),
    );
    let mut list_kind = CreateCommandOption::new(
        CommandOptionType::String,
        "type",
        "The type of the permission",
    );
    for kind in PermissionType::ALL {
        list_kind = list_kind.add_string_choice(kind.display(), kind.value());
    }
    let list = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "list",
        "Lists the permissions for a specific target",
    )
    .add_sub_option(target())
    .add_sub_option(list_kind);
    CreateCommand::new(NAME)
        .description("View and edit permissions")
        .add_option(set)
        .add_option(list)
}

pub async fn autocomplete(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let options = command.data.options();
    let kind = options
        .first()
        .and_then(|subcommand| match &subcommand.value {
            ResolvedValue::SubCommand(options) => {
                string(options, "type").and_then(PermissionType::of)
            }
            _ => None,
        });
    let typing = command
        .data
        .autocomplete()
        .map(|option| option.value.trim())
        .unwrap_or_default();
    let mut suggestions = suggestions(kind);
    if !typing.is_empty() {
        let sanitized = sanitize(typing);
        suggestions.sort_by_key(|suggestion| {
            let candidate = sanitize(suggestion);
            candidate.len()
                * if candidate.starts_with(&sanitized) {
                    1
                } else if candidate.contains(&sanitized) {
                    2
                } else {
                    3
                }
        });
    }
    let mut response = CreateAutocompleteResponse::new();
    for suggestion in suggestions.into_iter().take(25) {
        response = response.add_string_choice(suggestion.clone(), suggestion);
    }
    command
        .create_response(&ctx.http, CreateInteractionResponse::Autocomplete(response))
        .await
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    if command.member.is_none() {
        return super::reply_ephemeral(
            ctx,
            command,
            "This command can only be used in the configured server.",
        )
        .await;
    }
    // Java delegates access to this command through DPerms. The shared
    // command handler has already checked that permission before dispatching
    // here, so a second administrator-only check would reject delegated roles.
    let permissions = {
        ctx.data
            .read()
            .await
            .get::<DiscordPermissionsKey>()
            .cloned()
    };
    let Some(permissions) = permissions else {
        return super::reply_ephemeral(ctx, command, "Discord permissions are unavailable.").await;
    };
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let Some((target, mention, roles)) = target(sub_options) else {
        return super::reply_ephemeral(ctx, command, "A user or role target is required.").await;
    };

    match subcommand.name {
        "set" => {
            let Some(kind) = string(sub_options, "type").and_then(PermissionType::of) else {
                return super::reply_ephemeral(
                    ctx,
                    command,
                    "A valid permission type is required.",
                )
                .await;
            };
            let Some(name) = string(sub_options, "name") else {
                return super::reply_ephemeral(ctx, command, "A permission name is required.")
                    .await;
            };
            let value = match string(sub_options, "value") {
                Some("true") => Some(true),
                Some("false") => Some(false),
                Some("null") => None,
                _ => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        "A valid permission value is required.",
                    )
                    .await
                }
            };
            let previous = permissions.set(kind, target, name, value);
            let previous = match previous {
                Ok(previous) => previous,
                Err(error) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Could not save permissions: {error}"),
                    )
                    .await
                }
            };
            let message = if previous == value {
                format!(
                    "The {} permission `{name}` for {mention} is already {}",
                    kind.display(),
                    bool_label(value)
                )
            } else {
                format!(
                    "Set {} permission `{name}` for {mention} to {} (was {})",
                    kind.display(),
                    bool_label(value),
                    bool_label(previous)
                )
            };
            super::reply_ephemeral(ctx, command, &message).await
        }
        "list" => {
            let kinds: Vec<PermissionType> = string(sub_options, "type")
                .and_then(PermissionType::of)
                .map_or_else(|| PermissionType::ALL.to_vec(), |kind| vec![kind]);
            let mut sections = Vec::new();
            for kind in kinds {
                let values = permissions.list(kind, target, &roles);
                let body = if values.is_empty() {
                    "(no explicit permissions)".to_string()
                } else {
                    values
                        .into_iter()
                        .map(|(name, value)| format!("`{name}`: {}", bool_label(Some(value))))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                sections.push(format!(
                    "{} permissions for {mention}:\n{body}",
                    kind.display()
                ));
            }
            super::reply_ephemeral(ctx, command, &sections.join("\n\n")).await
        }
        _ => super::reply_ephemeral(ctx, command, "Unknown permissions subcommand.").await,
    }
}

fn target(
    options: &[ResolvedOption<'_>],
) -> Option<(PermissionTarget, String, Vec<serenity::model::id::RoleId>)> {
    options
        .iter()
        .find(|option| option.name == "target")
        .and_then(|option| match &option.value {
            ResolvedValue::User(user, member) => Some((
                PermissionTarget::User(user.id),
                user.id.mention().to_string(),
                member
                    .as_ref()
                    .map(|member| member.roles.clone())
                    .unwrap_or_default(),
            )),
            ResolvedValue::Role(role) => Some((
                PermissionTarget::Role(role.id),
                role.id.mention().to_string(),
                Vec::new(),
            )),
            _ => None,
        })
}

fn string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|option| option.name == name)
        .and_then(|option| match option.value {
            ResolvedValue::String(value) => Some(value),
            _ => None,
        })
}

fn bool_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Allow",
        Some(false) => "Deny",
        None => "Default",
    }
}

fn sanitize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        .collect()
}

fn suggestions(kind: Option<PermissionType>) -> Vec<String> {
    let mut values = vec!["*".to_string()];
    match kind {
        Some(PermissionType::SlashCommand) => values.extend(super::slash_command_paths()),
        Some(PermissionType::MessageCommand) => values.extend(super::message_command_names()),
        Some(PermissionType::Other) => values.extend([
            "groupex-bans-manage".to_string(),
            "groupex-tags-edit".to_string(),
        ]),
        _ => {}
    }
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_does_not_override_delegated_permissions() {
        let value = serde_json::to_value(register()).unwrap();
        assert!(value.get("default_member_permissions").is_none());
    }

    #[test]
    fn suggestions_include_java_other_permissions_and_commands() {
        assert!(
            suggestions(Some(PermissionType::Other)).contains(&"groupex-bans-manage".to_string())
        );
        assert!(suggestions(Some(PermissionType::SlashCommand))
            .contains(&"vrchat-user-ban".to_string()));
        assert!(suggestions(Some(PermissionType::MessageCommand))
            .contains(&"fix-audit-thread".to_string()));
    }
}
