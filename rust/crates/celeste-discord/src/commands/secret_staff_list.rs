//! Port of `ScarletDiscordCommands.SecretStaffList` (`@SlashCmd("secret-staff-list")`,
//! backed by `Scarlet.secretStaffList`/`secret_staff_list.json`). Java
//! implements this as `_StaffList`'s other subclass — byte-for-byte
//! identical to `StaffList` except for which backing list it reads/writes
//! and the "secret staff" label in its messages. See
//! [`super::staff_list`]'s docs for the shared implementation this reuses.

use serenity::builder::CreateCommand;
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

pub const NAME: &str = "secret-staff-list";

pub fn register() -> CreateCommand {
    super::staff_list::register_generic(NAME, "Configures the secret staff list")
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let Some(list) = ({ ctx.data.read().await.get::<crate::data::SecretStaffListKey>().cloned() }) else {
        super::defer(ctx, command).await?;
        return super::reply_ephemeral(ctx, command, "The secret staff list is not available.").await;
    };
    super::staff_list::run_generic(ctx, command, list, "secret staff").await
}
