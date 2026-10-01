//! Port of `ScarletDiscordCommands.discordUserInfo`.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::mention::Mentionable;

use crate::data::{DiscordConfigKey, ScarletDataKey, SecretStaffListKey, VrChatClientKey};

pub const NAME: &str = "discord-user-info";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Lists internal information for a specific Discord user").add_option(
        CreateCommandOption::new(CommandOptionType::User, "discord-user", "The Discord user to look up").required(true),
    )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let resolved = options.iter().find(|o| o.name == "discord-user").map(|o| o.value.clone());
    let Some(ResolvedValue::User(user, _)) = resolved else {
        return super::reply_ephemeral(ctx, command, "A `discord-user` option is required.").await;
    };

    let mut lines = Vec::new();

    let scarlet_data = { ctx.data.read().await.get::<ScarletDataKey>().cloned() };
    if let Some(scarlet_data) = &scarlet_data {
        if let Some(vrc_id) = scarlet_data.global_get_snowflake_id(&user.id.to_string()) {
            let redact = {
                let data = ctx.data.read().await;
                data.get::<SecretStaffListKey>().is_some_and(|secret_staff| {
                    if !secret_staff.is_id(&vrc_id) { return false; }
                    let requester = scarlet_data.global_get_snowflake_id(&command.user.id.to_string());
                    !requester.as_deref().is_some_and(|id| secret_staff.is_id(id))
                })
            };
            if !redact {
                let vrchat_client = { ctx.data.read().await.get::<VrChatClientKey>().cloned() };
                let display_name = match &vrchat_client {
                    Some(slot) => {
                        let client = slot.read().await.clone();
                        match client {
                            // Java: `getUser(vrcId, within1day)`. This is an
                            // informational render, not a freshness-critical
                            // moderation decision, so bypassing the entity
                            // cache here needlessly hits VRChat on every use.
                            Some(client) => client
                                .get_user_cached(&vrc_id, linked_user_cache_age())
                                .await
                                .ok()
                                .and_then(|u| u.display_name),
                            None => None,
                        }
                    }
                    None => None,
                };
                match display_name {
                    Some(name) => lines.push(format!(
                        "### Linked VRChat user:\n[{name}](<https://vrchat.com/home/user/{vrc_id}>) `{vrc_id}`"
                    )),
                    None => lines.push(format!("### Linked VRChat user:\n`{vrc_id}` (not looked up right now)")),
                }
            }
        }
    }

    lines.push("### Celeste permissions:".to_string());
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    // Java intentionally reports which permission roles the invoker has,
    // not which roles the lookup target has (`event.getMember()`).
    let roles = command.member.as_ref().map(|member| member.roles.as_slice()).unwrap_or_default();
    for permission in scarlet_core::permission::ScarletPermission::ALL {
        let configured = config.as_ref()
            .map(|config| config.map_strings("scarletPermission2roleSf", permission.id()))
            .unwrap_or_default();
        let mentions = permission_role_mentions(roles, &configured);
        lines.push(format!("{}: from {}", permission.title(), if mentions.is_empty() {
            "no roles".to_string()
        } else {
            mentions.join(", ")
        }));
    }

    let pages = super::paginate_on_lines(&lines.join("\n"), 2000);
    super::send_pages(ctx, command, pages).await
}

fn linked_user_cache_age() -> i64 {
    scarlet_vrchat::entity_cache::within_1_day()
}

fn permission_role_mentions(
    roles: &[serenity::model::id::RoleId],
    configured: &[String],
) -> Vec<String> {
    roles
        .iter()
        .filter(|role| configured.iter().any(|id| id == &role.to_string()))
        .map(|role| role.mention().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_roles_are_filtered_from_the_supplied_invoker_roles() {
        let roles = [serenity::model::id::RoleId::new(1), serenity::model::id::RoleId::new(2)];
        assert_eq!(
            permission_role_mentions(&roles, &["2".to_string()]),
            vec!["<@&2>".to_string()]
        );
    }


    #[test]
    fn linked_vrchat_user_uses_javas_one_day_cache_window() {
        assert_eq!(linked_user_cache_age(), scarlet_vrchat::entity_cache::within_1_day());
    }
}
