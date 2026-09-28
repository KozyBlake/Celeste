//! Port of `ScarletDiscordCommands.unlinkVrchatAccount`
//! (`@SlashCmd("unlink-vrchat-account")`) — a member unlinking their own
//! Discord<->VRChat link.
//!
//! **Simplified**: Java replies in the invoking user's own Discord locale
//! via `I18n.tr(userLocale, ...)`; no i18n/translation-string system is
//! ported in this crate (a large, separate subsystem), so replies are
//! plain English. `pendingVrcLinks` (in-flight bio-verification requests
//! from `/link-vrchat-account`, itself not ported -- see PORTING.md) has
//! nothing to clear here since it doesn't exist in this port either.

use serenity::builder::CreateCommand;
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

use crate::data::ScarletDataKey;

pub const NAME: &str = "unlink-vrchat-account";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Unlink your Discord account from your linked VRChat account")
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let Some(scarlet_data) = ({ ctx.data.read().await.get::<ScarletDataKey>().cloned() }) else {
        return super::reply_ephemeral(ctx, command, "Celeste data is not available.").await;
    };

    let snowflake = command.user.id.to_string();
    let Some(linked_vrc_id) = scarlet_data.global_get_snowflake_id(&snowflake) else {
        return super::reply_ephemeral(ctx, command, "You don't have a linked VRChat account.").await;
    };

    scarlet_data.unlink_by_snowflake(&snowflake);
    super::reply_ephemeral(
        ctx,
        command,
        &format!(
            "Unlinked your Discord account from VRChat user [{linked_vrc_id}](<https://vrchat.com/home/user/{linked_vrc_id}>).\n\
            This only removes the link record - it doesn't change your roles or VRChat group membership."
        ),
    )
    .await
}
