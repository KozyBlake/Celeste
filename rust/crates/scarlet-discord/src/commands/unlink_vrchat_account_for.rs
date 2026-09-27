//! Port of `ScarletDiscordCommands.unlinkVrchatAccountFor`
//! (`@SlashCmd("unlink-vrchat-account-for")`) — a staff member unlinking
//! someone else's Discord<->VRChat link by either side of the pair (or
//! both). Gated at the Discord level via `default_member_permissions`
//! (`@DefaultPerms(Permission.MODERATE_MEMBERS)` in Java), unlike most
//! other commands in this crate which register with no restriction
//! (`Permission.USE_APPLICATION_COMMANDS`, effectively "anyone").
//!
//! **Simplified**: `vrchat-user` is a plain string option rather than
//! Java's autocomplete-resolved `io.github.vrchatapi.model.User` (see the
//! `commands` module docs); no VRChat lookup is needed to unlink by id
//! either way, so this never calls the VRChat API at all, matching Java's
//! own behavior for this command (it only reads `vrchatUser.getId()` when
//! resolved, which a plain string already gives directly).

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::Permissions;

use crate::data::ScarletDataKey;

pub const NAME: &str = "unlink-vrchat-account-for";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Staff: unlink another member's Discord/VRChat link (by Discord user and/or VRChat user)")
        .default_member_permissions(Permissions::MODERATE_MEMBERS)
        .add_option(CreateCommandOption::new(CommandOptionType::User, "discord-user", "The Discord user to unlink"))
        .add_option(CreateCommandOption::new(
            CommandOptionType::String,
            "vrchat-user",
            "The VRChat user id (usr_...) to unlink",
        ))
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let discord_user_id = options.iter().find(|o| o.name == "discord-user").and_then(|o| match &o.value {
        ResolvedValue::User(user, _) => Some(user.id.to_string()),
        _ => None,
    });
    let vrchat_user_id = options.iter().find(|o| o.name == "vrchat-user").and_then(|o| match &o.value {
        ResolvedValue::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    });

    if discord_user_id.is_none() && vrchat_user_id.is_none() {
        return super::reply_ephemeral(ctx, command, "Specify who to unlink: provide `discord-user` and/or `vrchat-user`.").await;
    }

    let Some(scarlet_data) = ({ ctx.data.read().await.get::<ScarletDataKey>().cloned() }) else {
        return super::reply_ephemeral(ctx, command, "Scarlet data is not available.").await;
    };

    let mut any = false;
    let mut lines = Vec::new();

    if let Some(discord_user_id) = &discord_user_id {
        match scarlet_data.unlink_by_snowflake(discord_user_id) {
            Some(vrc_id) => {
                any = true;
                lines.push(format!(
                    "Unlinked <@{discord_user_id}> from VRChat user [{vrc_id}](<https://vrchat.com/home/user/{vrc_id}>)."
                ));
            }
            None => lines.push(format!("<@{discord_user_id}> wasn't linked to a VRChat account.")),
        }
    }

    if let Some(vrchat_user_id) = &vrchat_user_id {
        match scarlet_data.unlink_by_id(vrchat_user_id) {
            Some(snowflake) => {
                any = true;
                lines.push(format!(
                    "Unlinked VRChat user [{vrchat_user_id}](<https://vrchat.com/home/user/{vrchat_user_id}>) from <@{snowflake}>."
                ));
            }
            None => lines.push(format!(
                "VRChat user [{vrchat_user_id}](<https://vrchat.com/home/user/{vrchat_user_id}>) wasn't linked to a Discord account."
            )),
        }
    }

    lines.push(
        if any {
            "This only removes the link record - it doesn't change roles or VRChat group membership. They can run `/link-vrchat-account` to re-link.".to_string()
        } else {
            "Nothing was linked, so nothing changed.".to_string()
        },
    );

    super::reply_ephemeral(ctx, command, &lines.join("\n")).await
}

