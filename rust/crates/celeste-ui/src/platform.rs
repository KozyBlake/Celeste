//! Port of the headless-vs-graphical decision in `IScarletUI.create`
//! (`Platform.forceHeadlessUi() || GraphicsEnvironment.isHeadless()`).
//!
//! Rust has no `GraphicsEnvironment.isHeadless()` equivalent (that's a JVM
//! AWT concept: it probes for a usable display and caches the answer). The
//! closest practical equivalent on Linux is checking whether a display
//! server is reachable at all (`DISPLAY`/`WAYLAND_DISPLAY`); other
//! platforms are assumed to have a display unless explicitly overridden.
//! `CELESTE_HEADLESS` is this port's equivalent of the Java version's
//! `Platform.forceHeadlessUi()` explicit override. `SCARLET_HEADLESS` is
//! still accepted for launch-script compatibility.

pub fn is_headless() -> bool {
    if let Ok(value) = std::env::var("CELESTE_HEADLESS")
        .or_else(|_| std::env::var("SCARLET_HEADLESS"))
    {
        if value == "1" || value.eq_ignore_ascii_case("true") {
            return true;
        }
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Windows' built-in fallback OpenGL implementation is only version 1.1,
/// which cannot run egui_glow. Prefer wgpu there so normal installations
/// and GPU-passthrough VMs use DirectX 12. Linux keeps the proven Glow path.
pub fn native_renderer() -> eframe::Renderer {
    if cfg!(windows) {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    }
}

/// The two audio configuration surfaces Windows exposes. Linux has no
/// equivalent split, so [`System`] opens the best installed desktop sound
/// tool there and [`Classic`] is only presented by the Windows UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSettingsPanel {
    System,
    Classic,
}

/// Open the operating system's audio-output controls without attempting to
/// change a device on Celeste's behalf. An application cannot reliably know
/// which of several valid devices the user intended, and changing the global
/// default silently would be much more surprising than taking them directly
/// to the relevant control panel.
#[cfg(target_os = "windows")]
pub fn open_audio_settings(panel: AudioSettingsPanel) -> Result<&'static str, String> {
    use std::process::Command;

    let (program, args, opened) = match panel {
        AudioSettingsPanel::System => (
            "explorer.exe",
            &["ms-settings:sound"][..],
            "Windows Sound Settings",
        ),
        AudioSettingsPanel::Classic => (
            "control.exe",
            &["mmsys.cpl,,0"][..],
            "the classic Windows Sound control panel",
        ),
    };
    Command::new(program)
        .args(args)
        .spawn()
        .map(|_| opened)
        .map_err(|error| format!("Could not open {opened}: {error}"))
}

#[cfg(target_os = "linux")]
pub fn open_audio_settings(_panel: AudioSettingsPanel) -> Result<&'static str, String> {
    use std::process::Command;

    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    for (program, args, label) in linux_audio_settings_candidates(&desktop) {
        let Some(executable) = celeste_core::tts_voices::find_executable(program) else {
            continue;
        };
        return Command::new(executable)
            .args(args)
            .spawn()
            .map(|_| label)
            .map_err(|error| format!("Could not open {label}: {error}"));
    }
    Err(
        "No supported graphical sound controls were found. Open your desktop's Sound settings, or install pavucontrol/pwvucontrol."
            .to_string(),
    )
}

#[cfg(target_os = "linux")]
fn linux_audio_settings_candidates(
    desktop: &str,
) -> Vec<(&'static str, &'static [&'static str], &'static str)> {
    let desktop = desktop.to_ascii_lowercase();
    let mut candidates = Vec::new();
    if desktop.contains("kde") || desktop.contains("plasma") {
        candidates.push((
            "plasma-open-settings",
            &["kcm_pulseaudio"][..],
            "KDE Plasma Sound settings",
        ));
        candidates.push((
            "systemsettings",
            &["kcm_pulseaudio"][..],
            "KDE Plasma Sound settings",
        ));
    }
    if desktop.contains("gnome") || desktop.contains("unity") {
        candidates.push((
            "gnome-control-center",
            &["sound"][..],
            "GNOME Sound settings",
        ));
    }
    if desktop.contains("cinnamon") {
        candidates.push((
            "cinnamon-settings",
            &["sound"][..],
            "Cinnamon Sound settings",
        ));
    }
    if desktop.contains("mate") {
        candidates.push(("mate-volume-control", &[][..], "MATE Sound Preferences"));
    }
    if desktop.contains("xfce") {
        candidates.push(("xfce4-mixer", &[][..], "Xfce audio mixer"));
    }
    // These work across desktops and with both PulseAudio and PipeWire's
    // PulseAudio compatibility layer, so they are the dependable fallback.
    candidates.push(("pavucontrol", &[][..], "PulseAudio Volume Control"));
    candidates.push(("pwvucontrol", &[][..], "PipeWire Volume Control"));
    candidates
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn open_audio_settings(_panel: AudioSettingsPanel) -> Result<&'static str, String> {
    Err("Celeste cannot open sound settings automatically on this operating system.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_env_var_forces_headless() {
        std::env::set_var("SCARLET_HEADLESS", "1");
        assert!(is_headless());
        std::env::remove_var("SCARLET_HEADLESS");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_sound_settings_prefer_the_current_desktop_then_generic_mixers() {
        let kde = linux_audio_settings_candidates("KDE");
        assert_eq!(kde[0].0, "plasma-open-settings");
        assert_eq!(kde[kde.len() - 2].0, "pavucontrol");

        let unknown = linux_audio_settings_candidates("something-custom");
        assert_eq!(unknown.iter().map(|entry| entry.0).collect::<Vec<_>>(), [
            "pavucontrol",
            "pwvucontrol",
        ]);
    }
}
