//! Adaptive startup splash used by the desktop executable.
//!
//! The splash runs in a short-lived copy of the Celeste executable. Winit
//! owns a process's GUI event loop and cannot reliably hand it from a splash
//! window to the main eframe window, especially on Windows. A companion
//! process keeps both windows ordinary, single-event-loop applications.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DONE_STATUS: &str = "__CELESTE_STARTUP_DONE__";
const SHOW_AFTER: Duration = Duration::from_millis(250);
const MAX_LIFETIME: Duration = Duration::from_secs(90);
const BUNDLED_ICON: &[u8] = include_bytes!("../../../assets/icons/celeste-256.png");

pub const STATUS_ENV: &str = "CELESTE_SPLASH_STATUS_FILE";

/// Wait briefly before creating a window. Fast parent launches write the
/// done marker during this grace period and never flash a splash on screen.
pub fn run(status_file: PathBuf) -> eframe::Result<()> {
    let waiting_since = Instant::now();
    while waiting_since.elapsed() < SHOW_AFTER {
        if read_status(&status_file).as_deref() == Some(DONE_STATUS) {
            let _ = std::fs::remove_file(status_file);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if read_status(&status_file).as_deref() == Some(DONE_STATUS) {
        let _ = std::fs::remove_file(status_file);
        return Ok(());
    }

    let icon = decode_icon(BUNDLED_ICON);
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id("dev.kozyblake.celeste.splash")
        .with_inner_size([440.0, 300.0])
        .with_resizable(false)
        .with_maximize_button(false)
        .with_minimize_button(false);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        "Celeste is starting",
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
    texture: Option<egui::TextureHandle>,
}

impl StartupSplash {
    fn new(status_file: PathBuf) -> Self {
        Self {
            status_file,
            status: "Starting Celeste...".to_string(),
            started: Instant::now(),
            texture: None,
        }
    }
}

impl eframe::App for StartupSplash {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        match read_status(&self.status_file) {
            Some(status) if status == DONE_STATUS => {
                let _ = std::fs::remove_file(&self.status_file);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            Some(status) if !status.is_empty() => self.status = status,
            _ => {}
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
}
