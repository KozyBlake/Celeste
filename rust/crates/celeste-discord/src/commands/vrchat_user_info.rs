//! Port of `ScarletDiscordCommands.vrchatUserInfo`.
//!
//! Takes Java's VRChat user id string (`vrchat-user`) with live autocomplete
//! rather than Java's autocomplete-resolved `io.github.vrchatapi.model.User`
//! option — live-search-as-you-type autocomplete is a separate feature
//! (interaction autocomplete handling) not wired up yet, see the
//! `commands` module docs. Also: `CelesteData`'s `AuditEntryMetadata.entry`
//! is a `serde_json::Value` placeholder rather than a typed
//! `GroupAuditLogEntry` (see `celeste_core::data`'s docs), so its fields
//! are read generically here instead of through typed accessors.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

use crate::data::{CelesteDataKey, VrChatClientKey};

pub const NAME: &str = "vrchat-user-info";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Shows Celeste's internal metadata for a VRChat user").add_option(
        CreateCommandOption::new(CommandOptionType::String, "vrchat-user", "The VRChat user id (usr_...)")
            .required(true)
            .set_autocomplete(true),
    )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let resolved = options.iter().find(|o| o.name == "vrchat-user").map(|o| o.value.clone());
    let Some(ResolvedValue::String(vrc_id)) = resolved else {
        return super::reply_ephemeral(ctx, command, "A `vrchat-user` option is required.").await;
    };
    let vrc_id = vrc_id.to_string();

    let Some(celeste_data) = ({ ctx.data.read().await.get::<CelesteDataKey>().cloned() }) else {
        return super::reply_ephemeral(ctx, command, "Celeste data is not available.").await;
    };

    let vrchat_client = { ctx.data.read().await.get::<VrChatClientKey>().cloned() };
    let Some(client_slot) = vrchat_client else {
        return super::reply_ephemeral(ctx, command, "Celeste isn't logged in to VRChat right now.").await;
    };
    let client = { client_slot.read().await.clone() };
    let Some(client) = client else {
        return super::reply_ephemeral(ctx, command, "Celeste isn't logged in to VRChat right now.").await;
    };

    // Java `vrchatUserInfo`: `getUser(vrcId, within1day)`.
    let display_name = match client.get_user_cached(&vrc_id, celeste_vrchat::entity_cache::within_1_day()).await {
        Ok(user) => user.display_name.unwrap_or_else(|| vrc_id.clone()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {vrc_id} ({err})")).await;
        }
    };

    if !celeste_data.user_metadata_exists(&vrc_id) {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "No VRChat user metadata found for [{display_name}](https://vrchat.com/home/user/{vrc_id})"
            ),
        )
        .await;
    }
    let user_meta = celeste_data.user_metadata(&vrc_id);
    let mut lines =
        vec![format!("VRChat user metadata for [{display_name}](<https://vrchat.com/home/user/{vrc_id}>):")];

    if !user_meta.audit_entry_ids.is_empty() {
        lines.push("### Moderation events:".to_string());
        for audit_entry_id in &user_meta.audit_entry_ids {
            let mut line = format!("`{audit_entry_id}`");
            let entry_meta = celeste_data.audit_entry_metadata(audit_entry_id);
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
            lines.push(evidence_line(&celeste_data, submission));
        }
    }

    let pages = super::paginate_on_lines(&lines.join("\n"), 2000);
    super::send_pages(ctx, command, pages).await
}

fn evidence_line(
    celeste_data: &celeste_core::data::CelesteData,
    submission: &celeste_core::data::EvidenceSubmission,
) -> String {
    let file_name = submission.file_name.as_deref().unwrap_or("(unnamed)");
    let url = submission.url.as_deref().unwrap_or("");
    let mut line = format!("[`{file_name}`](<{url}>)");
    if let Some(proxy) = &submission.proxy_url {
        line.push_str(&format!(" [(proxy)](<{proxy}>)"));
    }
    if let Some(audit_entry_id) = submission.audit_entry_id.as_deref() {
        let entry = celeste_data.audit_entry_metadata(audit_entry_id);
        match entry.some_url() {
            Some(url) => line.push_str(&format!(" for [`{audit_entry_id}`](<{url}>)")),
            None => line.push_str(&format!(" for `{audit_entry_id}`")),
        }
    }
    if let Some(display_name) = submission.submitter_display_name.as_deref() {
        line.push_str(&format!(" from {display_name}"));
        if let Some(snowflake) = submission.submitter_snowflake.as_deref() {
            line.push_str(&format!(" (<@{snowflake}>)"));
        }
    } else if let Some(snowflake) = submission.submitter_snowflake.as_deref() {
        line.push_str(&format!(" from <@{snowflake}>"));
    }
    if let Some(time) = submission.submission_time.as_ref() {
        line.push_str(&format!(" at <t:{}:f>", time.timestamp()));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_lines_include_every_java_clause() {
        let dir = tempfile::tempdir().unwrap();
        let data = celeste_core::data::CelesteData::open(dir.path());
        data.set_audit_entry_metadata(
            "audit_1",
            celeste_core::data::AuditEntryMetadata {
                guild_snowflake: Some("1".into()),
                channel_snowflake: Some("2".into()),
                message_snowflake: Some("3".into()),
                ..Default::default()
            },
        );
        let submission = celeste_core::data::EvidenceSubmission {
            audit_entry_id: Some("audit_1".into()),
            submitter_snowflake: Some("4".into()),
            submitter_display_name: Some("Moderator".into()),
            submission_time: Some(chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
            file_name: Some("proof.png".into()),
            url: Some("https://example.invalid/proof".into()),
            proxy_url: Some("https://example.invalid/proxy".into()),
            ..Default::default()
        };
        let line = evidence_line(&data, &submission);
        assert!(line.contains("proof.png"));
        assert!(line.contains("(proxy)"));
        assert!(line.contains("[`audit_1`](<https://discord.com/channels/1/2/3>)"));
        assert!(line.contains("from Moderator (<@4>)"));
        assert!(line.contains("at <t:1700000000:f>"));
    }
}
