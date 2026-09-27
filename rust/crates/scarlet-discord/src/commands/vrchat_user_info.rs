//! Port of `ScarletDiscordCommands.vrchatUserInfo`.
//!
//! **Simplified**: takes a plain VRChat user id string (`vrchat-user-id`)
//! rather than Java's autocomplete-resolved `io.github.vrchatapi.model.User`
//! option — live-search-as-you-type autocomplete is a separate feature
//! (interaction autocomplete handling) not wired up yet, see the
//! `commands` module docs. Also: `ScarletData`'s `AuditEntryMetadata.entry`
//! is a `serde_json::Value` placeholder rather than a typed
//! `GroupAuditLogEntry` (see `scarlet_core::data`'s docs), so its fields
//! are read generically here instead of through typed accessors.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

use crate::data::{ScarletDataKey, VrChatClientKey};

pub const NAME: &str = "vrchat-user-info";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Shows Scarlet's internal metadata for a VRChat user").add_option(
        CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...)")
            .required(true),
    )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let resolved = options.iter().find(|o| o.name == "vrchat-user-id").map(|o| o.value.clone());
    let Some(ResolvedValue::String(vrc_id)) = resolved else {
        return super::reply_ephemeral(ctx, command, "A `vrchat-user-id` option is required.").await;
    };
    let vrc_id = vrc_id.to_string();

    let Some(scarlet_data) = ({ ctx.data.read().await.get::<ScarletDataKey>().cloned() }) else {
        return super::reply_ephemeral(ctx, command, "Scarlet data is not available.").await;
    };

    let vrchat_client = { ctx.data.read().await.get::<VrChatClientKey>().cloned() };
    let Some(client_slot) = vrchat_client else {
        return super::reply_ephemeral(ctx, command, "Scarlet isn't logged in to VRChat right now.").await;
    };
    let client = { client_slot.read().await.clone() };
    let Some(client) = client else {
        return super::reply_ephemeral(ctx, command, "Scarlet isn't logged in to VRChat right now.").await;
    };

    let display_name = match client.get_user(&vrc_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| vrc_id.clone()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {vrc_id} ({err})")).await;
        }
    };

    let user_meta = scarlet_data.user_metadata(&vrc_id);
    let mut lines =
        vec![format!("VRChat user metadata for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>):")];

    if !user_meta.audit_entry_ids.is_empty() {
        lines.push("### Moderation events:".to_string());
        for audit_entry_id in &user_meta.audit_entry_ids {
            let mut line = format!("`{audit_entry_id}`");
            let entry_meta = scarlet_data.audit_entry_metadata(audit_entry_id);
            if !entry_meta.entry_redacted && entry_meta.parent_event_id.is_none() {
                if let Some(entry) = &entry_meta.entry {
                    let description = entry.get("description").and_then(|v| v.as_str()).unwrap_or("(no description)");
                    if let Some(created_at) =
                        entry.get("createdAt").and_then(|v| v.as_str()).and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    {
                        line.push_str(&format!(" <t:{}:f>:", created_at.timestamp()));
                    }
                    match entry_meta.message_url() {
                        Some(url) => line.push_str(&format!(" [{description}]({url})")),
                        None => line.push_str(&format!(" {description}")),
                    }
                }
            }
            lines.push(line);
        }
    }

    if !user_meta.evidence_submissions.is_empty() {
        lines.push("### Evidence submissions:".to_string());
        for submission in &user_meta.evidence_submissions {
            let file_name = submission.file_name.as_deref().unwrap_or("(unnamed)");
            let url = submission.url.as_deref().unwrap_or("");
            let mut line = format!("[`{file_name}`](<{url}>)");
            if let Some(proxy) = &submission.proxy_url {
                line.push_str(&format!(" [(proxy)](<{proxy}>)"));
            }
            lines.push(line);
        }
    }

    let pages = super::paginate_on_lines(&lines.join("\n"), 2000);
    super::send_pages(ctx, command, pages).await
}
