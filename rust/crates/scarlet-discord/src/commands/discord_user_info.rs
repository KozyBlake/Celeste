//! Port of `ScarletDiscordCommands.discordUserInfo`.
//!
//! **Simplified**: Java's version lists which `ScarletPermission`s the
//! member holds by cross-referencing a Discord role -> permission mapping
//! (`discord.scarletPermission2roleSf`) that isn't ported yet (`ScarletPermission`
//! itself is real — `scarlet_core::permission` — but the role-mapping config
//! layer around it isn't; see PORTING.md). This lists the member's raw
//! Discord roles instead of pretending to resolve permissions from a
//! mapping that doesn't exist here.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::mention::Mentionable;

use crate::data::{ScarletDataKey, VrChatClientKey};

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
    let Some(ResolvedValue::User(user, member)) = resolved else {
        return super::reply_ephemeral(ctx, command, "A `discord-user` option is required.").await;
    };

    let mut lines = Vec::new();

    let scarlet_data = { ctx.data.read().await.get::<ScarletDataKey>().cloned() };
    if let Some(scarlet_data) = &scarlet_data {
        if let Some(vrc_id) = scarlet_data.global_get_snowflake_id(&user.id.to_string()) {
            let vrchat_client = { ctx.data.read().await.get::<VrChatClientKey>().cloned() };
            let display_name = match &vrchat_client {
                Some(slot) => {
                    let guard = slot.read().await;
                    match guard.as_ref() {
                        Some(client) => client.get_user(&vrc_id).await.ok().and_then(|u| u.display_name),
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

    lines.push("### Discord roles:".to_string());
    match &member {
        Some(member) if !member.roles.is_empty() => {
            let mentions = member.roles.iter().map(|r| r.mention().to_string()).collect::<Vec<_>>().join(", ");
            lines.push(mentions);
        }
        _ => lines.push("no roles".to_string()),
    }

    let pages = super::paginate_on_lines(&lines.join("\n"), 2000);
    super::send_pages(ctx, command, pages).await
}
