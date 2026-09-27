//! Port target: `ScarletUI.java` (5,716 lines — the full Swing main window)
//! and `ScarletDiscordUI.java` (2,345 lines — Discord bot config panel).
//!
//! This covers the *shape* real-to-real: the toolbar row, the Instance/
//! Settings/CLI tab structure, the live player table with the same columns
//! as `propstable` in the Java source, and the status bar text format — all
//! driven by real state ([`crate::state::UiState`] for players/status,
//! [`scarlet_core::settings`] for the Settings tab), not fixtures. What
//! isn't ported: most of the toolbar buttons' actual behavior (Create
//! Instance, Import Groups, Sync Discord are logged as not-yet-wired
//! rather than faked), moderation actions (Ban/Unban/Invite are stubs),
//! and the CLI tab's real command interpreter (currently just echoes and
//! answers `help`/`clear`).
//!
//! Toolkit: `egui`/`eframe` (see prior module docs in git history for the
//! rationale) with `egui_extras::TableBuilder` for the grid.
//!
//! **Not visually verified by me** — confirmed working by the repo owner
//! on their own machine (this container has no display server). If
//! something here looks wrong against a fresh run, that's expected until
//! iterated on further, not a regression from a "known good" state.

use std::path::PathBuf;
use std::sync::Arc;

use scarlet_core::settings::file_valued::SettingKind;
use scarlet_core::settings::Settings;

use crate::state::UiState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Instance,
    Settings,
    Cli,
}

pub struct ScarletApp {
    state: UiState,
    settings: Option<Arc<Settings>>,
    data_dir: PathBuf,
    staff_mode: bool,
    tab: Tab,
    cli_input: String,
    settings_edit_buffers: std::collections::HashMap<String, String>,
}

impl ScarletApp {
    pub fn new(state: UiState, settings: Option<Arc<Settings>>, data_dir: PathBuf, staff_mode: bool) -> Self {
        ScarletApp {
            state,
            settings,
            data_dir,
            staff_mode,
            tab: Tab::Instance,
            cli_input: String::new(),
            settings_edit_buffers: std::collections::HashMap::new(),
        }
    }

    fn window_title(&self) -> String {
        if self.staff_mode {
            format!("{} (staff mode)", scarlet_core::identity::NAME)
        } else {
            format!(
                "{} {} \u{2014} {}",
                scarlet_core::identity::APP_NAME,
                scarlet_core::identity::VERSION,
                scarlet_core::identity::FORK_NOTE
            )
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Data Folder").on_hover_text("Open the KozyBlake/Scarlet data directory").clicked() {
                if let Err(err) = open::that(&self.data_dir) {
                    self.state.append_cli_output(format!("Could not open data folder: {err}"));
                }
            }
            if ui.button("Create Instance").clicked() {
                self.state.append_cli_output("Create Instance: not wired up yet (needs the VRChat instance-creation flow).");
            }
            if ui.button("Import Groups (URL)").clicked() {
                self.state.append_cli_output("Import Groups (URL): not wired up yet.");
            }
            if ui.button("Import Groups (File)").clicked() {
                self.state.append_cli_output("Import Groups (File): not wired up yet.");
            }
            if ui.button("Sync Discord").clicked() {
                self.state.append_cli_output("Sync Discord: not wired up yet.");
            }
            if ui.button("Clear Instance").clicked() {
                self.state.clear_instance();
                self.state.append_cli_output("Instance cleared.");
            }
            let tts_on = self.state.tts_enabled();
            if ui.button(if tts_on { "TTS: On" } else { "TTS: Off" }).clicked() {
                self.state.set_tts_enabled(!tts_on);
            }
            if ui.button("Skip TTS").clicked() {
                self.state.append_cli_output("Skip TTS: not wired up yet (no TTS engine ported).");
            }
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Instance, "  Instance  ");
            ui.selectable_value(&mut self.tab, Tab::Settings, "  Settings  ");
            ui.selectable_value(&mut self.tab, Tab::Cli, "  CLI  ");
        });
    }

    fn instance_tab(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let players = self.state.ordered_players();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .column(Column::auto().at_least(80.0)) // Name
                .column(Column::auto().at_least(70.0)) // Id
                .column(Column::auto().at_least(60.0)) // Pronouns
                .column(Column::auto().at_least(80.0)) // Avatar
                .column(Column::auto().at_least(70.0)) // Performance
                .column(Column::auto().at_least(50.0)) // Avatar Stats
                .column(Column::auto().at_least(70.0)) // Acc-Age
                .column(Column::auto().at_least(90.0)) // Joined
                .column(Column::auto().at_least(90.0)) // Left
                .column(Column::auto().at_least(80.0)) // Advisory
                .column(Column::auto().at_least(40.0)) // 18+
                .column(Column::auto().at_least(60.0)) // Rank
                .column(Column::auto().at_least(50.0)) // Profile
                .column(Column::auto().at_least(60.0)) // Copy Id
                .column(Column::auto().at_least(40.0)) // Ban
                .header(20.0, |mut header| {
                    for label in [
                        "Name",
                        "Id",
                        "Pronouns",
                        "Avatar",
                        "Performance",
                        "Avatar Stats",
                        "Acc-Age",
                        "Joined",
                        "Left",
                        "Advisory",
                        "18+",
                        "Rank",
                        "Profile",
                        "Copy Id",
                        "Ban",
                    ] {
                        header.col(|ui| {
                            ui.strong(label);
                        });
                    }
                })
                .body(|mut body| {
                    for player in &players {
                        body.row(18.0, |mut row| {
                            row.col(|ui| {
                                ui.label(&player.name);
                            });
                            row.col(|ui| {
                                ui.label(&player.id);
                            });
                            row.col(|ui| {
                                ui.label(player.pronouns.as_deref().unwrap_or(""));
                            });
                            row.col(|ui| {
                                ui.label(player.avatar_name.as_deref().unwrap_or(""));
                            });
                            row.col(|ui| {
                                ui.label(player.performance_rating.as_deref().unwrap_or(""));
                            });
                            row.col(|ui| {
                                if ui.button("View").clicked() {
                                    self.state.append_cli_output(format!("Avatar stats for {}: not wired up yet.", player.name));
                                }
                            });
                            row.col(|ui| {
                                let age = player
                                    .account_age
                                    .map(|(y, m, d)| format!("{y}y {m}m {d}d"))
                                    .unwrap_or_default();
                                ui.label(age);
                            });
                            row.col(|ui| {
                                ui.label(player.joined.format("%Y-%m-%d %H:%M").to_string());
                            });
                            row.col(|ui| {
                                let left = player.left.map(|l| l.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                                ui.label(left);
                            });
                            row.col(|ui| {
                                ui.label(player.advisory.as_deref().unwrap_or(""));
                            });
                            row.col(|ui| {
                                ui.label(player.age_verification_status.as_deref().unwrap_or(""));
                            });
                            row.col(|ui| {
                                let color = player
                                    .trust_rank
                                    .color_rgb()
                                    .map(|(r, g, b)| egui::Color32::from_rgb(r, g, b))
                                    .unwrap_or(egui::Color32::GRAY);
                                ui.colored_label(color, player.trust_rank.label());
                            });
                            row.col(|ui| {
                                if ui.button("Open").clicked() {
                                    ui.ctx().open_url(egui::OpenUrl::same_tab(format!(
                                        "https://vrchat.com/home/user/{}",
                                        player.id
                                    )));
                                }
                            });
                            row.col(|ui| {
                                if ui.button("Copy").clicked() {
                                    ui.ctx().copy_text(player.id.clone());
                                }
                            });
                            row.col(|ui| {
                                if ui.button("Ban").clicked() {
                                    self.state.append_cli_output(format!(
                                        "Ban {}: not wired up yet (needs the VRChat group-ban call).",
                                        player.name
                                    ));
                                }
                            });
                        });
                    }
                });
        });
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let Some(settings) = &self.settings else {
            ui.label("Settings unavailable (no settings store attached).");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            for descriptor in settings.registry.all() {
                ui.horizontal(|ui| {
                    ui.label(descriptor.name());
                    match descriptor.kind() {
                        SettingKind::Boolean { .. } => {
                            let mut value = descriptor.get_as_string() == "true";
                            if ui.checkbox(&mut value, "").changed() {
                                descriptor.set_from_string(&value.to_string(), "ui");
                            }
                        }
                        SettingKind::IntegerRange { min, max, .. } => {
                            let mut value: i32 = descriptor.get_as_string().parse().unwrap_or(*min);
                            if ui.add(egui::Slider::new(&mut value, *min..=*max)).changed() {
                                descriptor.set_from_string(&value.to_string(), "ui");
                            }
                        }
                        _ => {
                            let buf = self
                                .settings_edit_buffers
                                .entry(descriptor.id().to_string())
                                .or_insert_with(|| descriptor.get_as_string());
                            ui.text_edit_singleline(buf);
                            if ui.button("Set").clicked() {
                                descriptor.set_from_string(buf, "ui");
                            }
                        }
                    }
                });
            }
            if settings.registry.all().is_empty() {
                ui.label("(No settings registered yet — Scarlet.java's ~40 settings fields aren't ported here yet, see PORTING.md.)");
            }
        });
    }

    fn cli_tab(&mut self, ui: &mut egui::Ui) {
        let available_height = ui.available_height() - 30.0;
        egui::ScrollArea::vertical().max_height(available_height.max(0.0)).stick_to_bottom(true).show(ui, |ui| {
            for line in self.state.cli_log() {
                ui.monospace(&line.0);
            }
        });
        ui.horizontal(|ui| {
            let response = ui.text_edit_singleline(&mut self.cli_input);
            let run = ui.button("Run").clicked() || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if run && !self.cli_input.trim().is_empty() {
                self.run_cli_command(self.cli_input.trim().to_string());
                self.cli_input.clear();
            }
        });
    }

    /// Minimal real command interpreter — not a port of Scarlet's actual CLI
    /// (which dispatches into the full command set once ported), just
    /// enough to prove the tab works end to end.
    fn run_cli_command(&mut self, command: String) {
        self.state.append_cli_output(format!("> {command}"));
        match command.as_str() {
            "help" => self.state.append_cli_output("Commands: help, clear. The full command interpreter isn't ported yet."),
            "clear" => self.state.clear_cli_log(),
            other => self.state.append_cli_output(format!("Unknown command: {other} (type 'help')")),
        }
    }
}

impl eframe::App for ScarletApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
            ui.separator();
            self.tab_bar(ui);
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            let (present, left, total) = self.state.status_counts();
            let text = if total == 0 {
                "No instance loaded".to_string()
            } else {
                format!("\u{25CF} {present} present   \u{25CB} {left} left   \u{2022} {total} total this session")
            };
            ui.horizontal(|ui| {
                ui.label(text);
                if let Some(announcement) = self.state.announcement() {
                    ui.separator();
                    ui.colored_label(egui::Color32::YELLOW, announcement.title.as_deref().unwrap_or("Scarlet announcement"));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Instance => self.instance_tab(ui),
            Tab::Settings => self.settings_tab(ui),
            Tab::Cli => self.cli_tab(ui),
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
pub fn run(state: UiState, settings: Option<Arc<Settings>>, data_dir: PathBuf, staff_mode: bool) -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        scarlet_core::identity::APP_NAME,
        options,
        Box::new(move |_cc| Ok(Box::new(ScarletApp::new(state, settings, data_dir, staff_mode)))),
    )
}
