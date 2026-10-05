//! Rust port of `net.sybyline.scarlet.server.discord` (JDA -> serenity).
//!
//! **Status: real, working skeleton, not a full port.** `ScarletDiscordJDA`
//! (4,068 lines) and `ScarletDiscordCommands` (5,765 lines) are ported
//! incrementally, one command/feature at a time, not all at once — see
//! PORTING.md. What's here is genuinely functional: [`client::build_client`]
//! stands up a real serenity client with a token, [`handler::Handler`]
//! handles the gateway `ready` event and dispatches slash-command
//! interactions, and [`commands::version`] is one real end-to-end command
//! (registration -> dispatch -> reply) proving the pipeline works, for
//! future commands to follow the same shape.

pub mod avatar_search;
pub mod client;
pub mod commands;
pub mod data;
pub mod events;
pub mod health;
pub mod handler;
pub mod interactions;
pub mod permissions;
pub mod pending_moderation;
pub mod token;
pub mod url_guard;
pub mod voice_tts;
