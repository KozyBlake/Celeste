//! Adaptive startup splash used by the desktop executable.
//!
//! The splash runs in a short-lived copy of the Celeste executable. Winit
//! owns a process's GUI event loop and cannot reliably hand it from a splash
//! window to the main eframe window, especially on Windows. A companion
//! process keeps both windows ordinary, single-event-loop applications.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DONE_STATUS: &str = "__CELESTE_STARTUP_DONE__";
const PROGRESS_STATUS: &str = "__CELESTE_PROGRESS__";
const MIN_VISIBLE: Duration = Duration::from_millis(900);
const MAX_LIFETIME: Duration = Duration::from_secs(90);
const BUNDLED_ICON: &[u8] = include_bytes!("../../../assets/icons/celeste-256.png");

pub const STATUS_ENV: &str = "CELESTE_SPLASH_STATUS_FILE";

pub fn run(status_file: PathBuf) -> eframe::Result<()> {
    let icon = decode_icon(BUNDLED_ICON);
    let mut viewport = egui::ViewportBuilder::default()
        // Match the installed desktop file and the main window. On Wayland,
        // KDE resolves the title-bar/taskbar icon from this application id;
        // the old splash-only id fell back to winit's generic "W" icon.
        .with_app_id(scarlet_core::identity::DESKTOP_APP_ID)
        .with_inner_size([440.0, 300.0])
        .with_resizable(false)
        .with_maximize_button(false)
        .with_minimize_button(false);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        "Celeste — Starting",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(StartupSplash::new(status_file)))),
    )
}

struct StartupSplash {
    status_file: PathBuf,
    status: String,
    started: Instant,
    done: bool,
    progress: Option<f32>,
    texture: Option<egui::TextureHandle>,
}

impl StartupSplash {
    fn new(status_file: PathBuf) -> Self {
        Self {
            status_file,
            status: "Starting Celeste...".to_string(),
            started: Instant::now(),
            done: false,
            progress: None,
            texture: None,
        }
    }
}

impl eframe::App for StartupSplash {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        match read_status(&self.status_file) {
            Some(status) if status.starts_with(DONE_STATUS) => {
                self.done = true;
                self.progress = Some(1.0);
                if let Some(final_status) = status.strip_prefix(DONE_STATUS) {
                    let final_status = final_status.trim();
                    if !final_status.is_empty() {
                        self.status = final_status.to_string();
                    }
                }
            }
            Some(status) if status.starts_with(PROGRESS_STATUS) => {
                let mut lines = status.lines();
                let _ = lines.next();
                self.progress = lines.next().and_then(|value| value.parse::<f32>().ok());
                let label = lines.collect::<Vec<_>>().join("\n");
                if !label.is_empty() {
                    self.status = label;
                }
            }
            Some(status) if !status.is_empty() => {
                self.progress = None;
                self.status = status;
            }
            _ => {}
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(operation_title(
            &self.status,
            self.done,
        )));

        if self.done && self.started.elapsed() >= MIN_VISIBLE {
                let _ = std::fs::remove_file(&self.status_file);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
        }

        // Do not leave an orphaned window behind if the parent is killed in
        // a way that prevents its normal drop guard from writing DONE.
        if self.started.elapsed() >= MAX_LIFETIME {
            let _ = std::fs::remove_file(&self.status_file);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.texture.is_none() {
            if let Ok(image) = image::load_from_memory(BUNDLED_ICON) {
                let rgba = image.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                self.texture = Some(ctx.load_texture(
                    "celeste-startup-logo",
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }

        let mut style = (*ctx.style()).clone();
        style.visuals.panel_fill = crate::theme::INK;
        style.visuals.window_fill = crate::theme::INK;
        style.visuals.override_text_color = Some(crate::theme::TEXT);
        ctx.set_style(style);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::INK)
                    .inner_margin(egui::Margin::same(28.0)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(texture) = &self.texture {
                        ui.image((texture.id(), egui::vec2(112.0, 112.0)));
                    } else {
                        ui.add_space(112.0);
                    }
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Celeste")
                            .size(28.0)
                            .strong()
                            .color(crate::theme::TEXT),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "Version {}",
                            scarlet_core::identity::VERSION
                        ))
                        .size(crate::theme::TEXT_SM)
                        .color(crate::theme::TEXT_MUTED),
                    );
                    ui.add_space(16.0);
                    if let Some(progress) = self.progress {
                        ui.add(
                            egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                                .desired_width(300.0)
                                .show_percentage(),
                        );
                        ui.add_space(8.0);
                    }
                    ui.horizontal(|ui| {
                        let available = ui.available_width();
                        ui.add_space((available - 260.0).max(0.0) / 2.0);
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(&self.status)
                                .size(crate::theme::TEXT_BODY)
                                .color(crate::theme::TEXT_MUTED),
                        );
                    });
                });
            });

        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

fn read_status(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|status| status.trim().to_string())
}

fn decode_icon(bytes: &[u8]) -> Option<egui::IconData> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn operation_title(status: &str, done: bool) -> String {
    if done {
        return "Celeste — Ready".to_string();
    }
    let operation = status.trim().trim_end_matches('.');
    if operation.is_empty() {
        "Celeste — Starting".to_string()
    } else {
        format!("Celeste — {operation}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_splash_icon_is_valid() {
        let icon = decode_icon(BUNDLED_ICON).expect("bundled icon should decode");
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 128);
    }

    #[test]
    fn status_reader_trims_completed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status");
        std::fs::write(&path, "  Loading settings...\n").unwrap();
        assert_eq!(read_status(&path).as_deref(), Some("Loading settings..."));
    }

    #[test]
    fn title_tracks_the_current_startup_operation() {
        assert_eq!(
            operation_title("Importing watched_groups.json...", false),
            "Celeste — Importing watched_groups.json"
        );
        assert_eq!(operation_title("Celeste is ready.", true), "Celeste — Ready");
    }
}
