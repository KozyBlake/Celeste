//! Rust port of `net.sybyline.scarlet.ui` and the top-level `ScarletUI`/
//! `IScarletUI`/`ScarletDiscordUI` (Swing/FlatLaf desktop UI).
//!
//! **Status: real skeleton (state model + a minimal working window), not a
//! port of the ~8,500-line Swing UI.** See `app`'s module docs for the
//! toolkit choice (egui/eframe) and what isn't ported yet.
//!
//! [`state::UiState`] is the real, fully-ported, unit-tested part: the data
//! model behind `IScarletUI`'s player-list contract
//! (`playerJoin`/`playerUpdate`/`playerLeave`/`clearInstance`/`fireSort`/
//! `hasActivePlayers`) plus connection-status display,
//! usable from any thread. [`platform::is_headless`] ports the
//! headless-vs-graphical decision from `IScarletUI.create`. [`run`] opens
//! the (currently minimal) graphical window; on a headless system, skip
//! calling it — there is no separate `ScarletUIHeadless` type to
//! instantiate here, since `UiState` alone already behaves correctly with
//! no window ever opened (every mutator is a plain data operation; nothing
//! about it depends on a window existing, matching `ScarletUIHeadless`'s
//! all-no-op methods without needing a parallel implementation to keep in
//! sync).

pub mod app;
pub mod data_dir_prompt;
pub mod dialogs;
pub mod guide;
pub mod platform;
pub mod setup;
pub mod splash;
pub mod state;
pub mod theme;
pub mod tts;

pub use app::{run, CelesteAppContext};
pub use celeste_core::ui::{ConnectedPlayer, InstanceState, PlayerJoin};
pub use state::UiState;
