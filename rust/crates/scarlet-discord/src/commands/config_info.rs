//! Port of `ScarletDiscordCommands.configInfo`: timestamps followed by the
//! complete audit and extended-audit routing inventory, including auxiliary
//! webhook assignments.

use serenity::builder::CreateCommand;
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

use crate::data::{DiscordConfigKey, DiscordConfigStore, SettingsKey};

pub const NAME: &str = "config-info";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Shows information about the current configuration")
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return super::reply_ephemeral(ctx, command, "Settings are not available.").await;
    };
    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is not available.")
            .await;
    };

    let lines = config_lines(&settings, &config);
    let pages = super::paginate_on_lines(&lines.join("\n"), 2000);
    super::send_pages(ctx, command, pages).await
}

fn config_lines(
    settings: &scarlet_core::settings::Settings,
    config: &DiscordConfigStore,
) -> Vec<String> {
    let mut lines = vec!["Configuration Information:".to_string()];

    if let Some(ts) = settings.last_audit_query.get_or_supply() {
        let epoch = ts.timestamp();
        lines.push(format!(
            "### Last Audit Query:\n<t:{epoch}:F> (<t:{epoch}:R>)"
        ));
    }
    if let Some(ts) = settings.last_auth_refresh.get_or_supply() {
        let epoch = ts.timestamp();
        lines.push(format!(
            "### Last Auth Refresh:\n<t:{epoch}:F> (<t:{epoch}:R>)"
        ));
    }
    if let Some(ts) = settings.next_mod_summary.get_or_null() {
        let epoch = ts.timestamp();
        lines.push(format!(
            "### Next Moderation Summary:\n<t:{epoch}:F> (<t:{epoch}:R>)"
        ));
    }

    lines.push("### Auditing Channels:".to_string());
    for event_type in super::set_event_channel::audit_types() {
        let channel = config
            .map_value("auditType2channelSf", event_type)
            .map_or_else(|| "unassigned".to_string(), |id| format!("<#{id}>"));
        let auxiliary = config.map_strings("auditType2scarletAuxWh", event_type);
        let suffix = if auxiliary.is_empty() {
            String::new()
        } else {
            format!(" (Auxiliary webhooks: {})", auxiliary.join(", "))
        };
        lines.push(format!(
            "{} ({}): {}{}",
            super::set_event_channel::title(event_type),
            event_type,
            channel,
            suffix
        ));
    }

    lines.push("### Extended auditing Channels:".to_string());
    for event_type in super::set_event_channel::extended_types() {
        let channel = config
            .map_value("auditExType2channelSf", event_type)
            .map_or_else(|| "unassigned".to_string(), |id| format!("<#{id}>"));
        lines.push(format!(
            "{} ({}): {}",
            super::set_event_channel::title(event_type),
            event_type,
            channel
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_java_routing_sections_and_auxiliary_webhooks() {
        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        let config = DiscordConfigStore::open(dir.path());
        config
            .set_map_value("auditType2channelSf", "group.user.ban", Some("123"))
            .unwrap();
        config
            .set_map_strings(
                "auditType2scarletAuxWh",
                "group.user.ban",
                &["staff".to_string(), "archive".to_string()],
            )
            .unwrap();
        config
            .set_map_value(
                "auditExType2channelSf",
                "groupex.instance.user.join",
                Some("456"),
            )
            .unwrap();

        let output = config_lines(&settings, &config).join("\n");
        assert!(output.contains("### Auditing Channels:"));
        assert!(output
            .contains("User Ban (group.user.ban): <#123> (Auxiliary webhooks: staff, archive)"));
        assert!(output.contains("User Join (groupex.instance.user.join): <#456>"));
        assert!(output.contains("group.user.unban): unassigned"));
        assert!(!output.contains("Registered settings"));
    }
}
