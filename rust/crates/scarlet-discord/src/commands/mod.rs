//! Command registration and dispatch. Port target: `DCommands`/`DInteractions`
//! (JDA-specific slash-command plumbing in `net.sybyline.scarlet.server.discord`)
//! and the command bodies in `ScarletDiscordCommands.java`. Only
//! [`version`] is ported so far — see PORTING.md for why this crate is a
//! skeleton rather than a full command-surface port.

pub mod version;

use serenity::client::Context;
use serenity::model::application::CommandInteraction;

/// Registers every ported slash command as a global command. Global
/// registration can take up to an hour to propagate on Discord's side;
/// swap to per-guild registration during development if faster iteration
/// is needed (not done here to keep this skeleton simple).
pub async fn register_global_commands(ctx: &Context) {
    let commands = vec![version::register()];
    if let Err(err) = serenity::model::application::Command::set_global_commands(&ctx.http, commands).await {
        tracing::error!("Failed to register global slash commands: {}", err);
    }
}

/// Dispatches an incoming command interaction to its handler by name.
pub async fn dispatch(ctx: &Context, command: &CommandInteraction) {
    let result = match command.data.name.as_str() {
        version::NAME => version::run(ctx, command).await,
        other => {
            tracing::warn!("Received unknown slash command: {}", other);
            Ok(())
        }
    };
    if let Err(err) = result {
        tracing::error!("Error handling command `{}`: {}", command.data.name, err);
    }
}
