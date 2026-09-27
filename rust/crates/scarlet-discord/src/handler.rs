//! Port target: `ScarletDiscordJDA.JDAEvents` (the JDA `ListenerAdapter`
//! wired up in the constructor) and the start of `ScarletEventListener`
//! (1,364 lines — moderation-event handling, not ported yet). This is the
//! skeleton every future event handler hangs off: connection lifecycle and
//! slash-command dispatch, both real and working.

use serenity::async_trait;
use serenity::client::{Context, EventHandler};
use serenity::model::application::Interaction;
use serenity::model::gateway::Ready;

use crate::commands;

pub struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Connected to Discord as {} ({})", ready.user.name, ready.user.id);
        commands::register_global_commands(&ctx).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) => commands::dispatch(&ctx, &command).await,
            Interaction::Component(component) => crate::interactions::dispatch_component(&ctx, &component).await,
            Interaction::Modal(modal) => crate::interactions::dispatch_modal(&ctx, &modal).await,
            _ => {},
        }
    }
}
