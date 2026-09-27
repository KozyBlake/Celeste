//! Port target: `ScarletUI.java` (5,716 lines — the full Swing main window:
//! settings panels, live player table with avatar/profile buttons, moderation
//! actions, CLI tab, ...) and `ScarletDiscordUI.java` (2,345 lines — Discord
//! bot config panel). **Not ported.** This is a minimal real window proving
//! the state -> render pipeline works (app identity, connection status, the
//! live player list from [`crate::state::UiState`]), for the real panels to
//! be built out onto incrementally — the same "real skeleton, not a stub"
//! approach as `scarlet-discord`/`scarlet-vrchat`.
//!
//! Toolkit choice: `egui`/`eframe` — immediate-mode, pure Rust (no native
//! toolkit bindings to cross-compile against), and a good fit for a
//! frequently-updating live player list (Swing's `JTable` model-update
//! dance has no equivalent needed here: every frame just renders current
//! state).
//!
//! **Not visually verified.** This container has no display server
//! (`DISPLAY`/`WAYLAND_DISPLAY` unset), so [`crate::platform::is_headless`]
//! is true here and `eframe::run_native` is never actually exercised in
//! this environment — only compiled. Confirm it actually renders correctly
//! on a machine with a display before trusting it further.

use crate::state::UiState;

pub struct ScarletApp {
    state: UiState,
}

impl ScarletApp {
    pub fn new(state: UiState) -> Self {
        ScarletApp { state }
    }
}

impl eframe::App for ScarletApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("{} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION));
            ui.separator();

            ui.label(format!("VRChat: {}", self.state.connection_status()));

            if let Some(announcement) = self.state.announcement() {
                ui.separator();
                let title = announcement.title.as_deref().unwrap_or("Scarlet announcement");
                ui.colored_label(egui::Color32::YELLOW, title);
                if let Some(message) = &announcement.message {
                    ui.label(message);
                }
            }

            ui.separator();
            ui.label("Instance players:");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for player in self.state.ordered_players() {
                    let color = player
                        .text_color
                        .map(|(r, g, b)| egui::Color32::from_rgb(r, g, b))
                        .unwrap_or(egui::Color32::WHITE);
                    let status = if player.left.is_some() { " (left)" } else { "" };
                    ui.colored_label(color, format!("{}{}", player.name, status));
                }
            });
        });
        // Immediate-mode redraw: request another frame so live updates from
        // background polling tasks (mutating `UiState` from other threads)
        // show up promptly instead of waiting for the next input event.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// Port of `showMainWindow`/the graphical half of `IScarletUI.create`.
/// Blocks the calling thread for the lifetime of the window — call it from
/// wherever the app decides to run graphically (e.g. the last step of
/// `main`, with Discord/VRChat running on background tasks that share
/// `state`), not concurrently with other blocking work on the same thread.
pub fn run(state: UiState) -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        scarlet_core::identity::APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::new(ScarletApp::new(state)))),
    )
}
