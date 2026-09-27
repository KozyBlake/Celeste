//! Port of `ScarletDiscordCommands.configInfo`, reduced to what's actually
//! ported on the Rust side: the last-audit-query/last-auth-refresh/
//! next-mod-summary timestamps from `Settings`, plus the full
//! `SettingsRegistry` inventory. Java's version also lists per-audit-type
//! Discord channel routing (`GroupAuditType`/`GroupAuditTypeEx` -> channel
//! snowflake); that config layer isn't ported yet (see PORTING.md), so
//! this doesn't fabricate an "unassigned" section for a config surface
//! that doesn't exist here rather than silently answer with something
//! that isn't real data.

use serenity::builder::CreateCommand;
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

use crate::data::SettingsKey;

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

    let mut lines = vec!["Configuration Information:".to_string()];

    if let Some(ts) = settings.last_audit_query.get_or_supply() {
        let epoch = ts.timestamp();
        lines.push(format!("### Last Audit Query:\n<t:{epoch}:F> (<t:{epoch}:R>)"));
    }
    if let Some(ts) = settings.last_auth_refresh.get_or_supply() {
        let epoch = ts.timestamp();
        lines.push(format!("### Last Auth Refresh:\n<t:{epoch}:F> (<t:{epoch}:R>)"));
    }
    if let Some(ts) = settings.next_mod_summary.get_or_null() {
        let epoch = ts.timestamp();
        lines.push(format!("### Next Moderation Summary:\n<t:{epoch}:F> (<t:{epoch}:R>)"));
    }

    lines.push("### Registered settings:".to_string());
    for descriptor in settings.registry.all() {
        lines.push(format!(
            "- **{}** (`{}`, {}): `{}`",
            descriptor.name(),
            descriptor.id(),
            descriptor.category().label(),
            descriptor.get_as_string()
        ));
    }

    let pages = super::paginate_on_lines(&lines.join("\n"), 2000);
    super::send_pages(ctx, command, pages).await
}
