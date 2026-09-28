//! Shared VRChat entity autocomplete used by Java's typed slash options.

use serenity::builder::{CreateAutocompleteResponse, CreateInteractionResponse};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

pub async fn respond(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let Some(focused) = command.data.autocomplete() else {
        return empty(ctx, command).await;
    };
    let query = focused.value.trim();
    let client = {
        let slot = ctx.data.read().await.get::<crate::data::VrChatClientKey>().cloned();
        match slot {
            Some(slot) => slot.read().await.clone(),
            None => None,
        }
    };
    let Some(client) = client else { return empty(ctx, command).await };
    let mut response = CreateAutocompleteResponse::new();

    match focused.name {
        "vrchat-user" | "vrchat-user-id" => {
            let users = if query.starts_with("usr_") {
                client.get_user(query).await.ok().map(|user| vec![(
                    user.id,
                    user.display_name.unwrap_or_else(|| query.to_string()),
                )]).unwrap_or_default()
            } else if query.is_empty() {
                Vec::new()
            } else {
                client.search_users(query, 25, 0).await.unwrap_or_default().into_iter()
                    .map(|user| {
                        let label = user.display_name.unwrap_or_else(|| user.id.clone());
                        (user.id, label)
                    }).collect()
            };
            for (id, label) in users.into_iter().take(25) {
                response = response.add_string_choice(choice_label(&label, &id), id);
            }
        }
        "vrchat-group" => {
            let groups = if query.starts_with("grp_") {
                client.get_group(query, false).await.ok().into_iter().collect()
            } else if query.is_empty() {
                Vec::new()
            } else {
                client.search_groups(query, 25, 0).await.unwrap_or_default()
            };
            for group in groups.into_iter().take(25) {
                response = response.add_string_choice(choice_label(&group.name, &group.id), group.id);
            }
        }
        "vrchat-role" | "vrchat-role-id" | "post-role" => {
            let group_id = {
                let data = ctx.data.read().await;
                data.get::<crate::data::SettingsKey>()
                    .and_then(|settings| settings.store.get_string(scarlet_vrchat::client::GROUP_ID_KEY))
            };
            if let Some(group_id) = group_id {
                let query = query.to_lowercase();
                let mut roles = client.get_group_roles(&group_id).await.unwrap_or_default();
                roles.retain(|role| query.is_empty()
                    || role.id.to_lowercase().contains(&query)
                    || role.name.to_lowercase().contains(&query));
                for role in roles.into_iter().take(25) {
                    response = response.add_string_choice(choice_label(&role.name, &role.id), role.id);
                }
            }
        }
        "vrchat-world" => {
            let worlds = if query.starts_with("wrld_") {
                client.get_world(query).await.ok().into_iter().collect()
            } else if query.is_empty() {
                Vec::new()
            } else {
                client.search_worlds(query, 25, 0).await.unwrap_or_default()
            };
            for world in worlds.into_iter().take(25) {
                response = response.add_string_choice(choice_label(&world.name, &world.id), world.id);
            }
        }
        _ => {}
    }
    command.create_response(&ctx.http, CreateInteractionResponse::Autocomplete(response)).await
}

async fn empty(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    command.create_response(
        &ctx.http,
        CreateInteractionResponse::Autocomplete(CreateAutocompleteResponse::new()),
    ).await
}

fn choice_label(name: &str, id: &str) -> String {
    let suffix = format!(" ({id})");
    let keep = 100usize.saturating_sub(suffix.chars().count());
    format!("{}{}", name.chars().take(keep).collect::<String>(), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_fit_discords_limit_and_keep_the_id() {
        let id = "usr_12345678-1234-1234-1234-123456789012";
        let label = choice_label(&"x".repeat(200), id);
        assert!(label.chars().count() <= 100);
        assert!(label.ends_with(&format!(" ({id})")));
    }
}
