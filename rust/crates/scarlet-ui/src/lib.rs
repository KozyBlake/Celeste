//! Rust port of `net.sybyline.scarlet.ui` (Swing + FlatLaf desktop UI).
//!
//! **Status: not started beyond this stub.** Rust has no drop-in equivalent
//! of Swing/FlatLaf; before porting the actual UI (splash screen, settings
//! panels, live player list, tray icon, etc. — `IScarletUI`, `ScarletUI`,
//! `ScarletDiscordUI`, `IScarletUISplash`) a toolkit needs to be picked
//! (egui/eframe is the most likely fit for a desktop app with live-updating
//! panels; iced and native GTK/tauri are the alternatives). See ../../PORTING.md.

/// Placeholder for `IScarletUI`. A real implementation needs a chosen UI
/// toolkit (see module docs) before this can do anything.
pub trait ScarletUi {
    fn set_ui_scale(&mut self, scale: f32);
}
