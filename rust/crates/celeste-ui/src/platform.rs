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

/// Install the currently running Linux build as a normal per-user desktop
/// application. A `.desktop` file by itself is not enough for a downloaded
/// standalone binary: it needs a stable executable path and an icon in the
/// user's XDG data hierarchy. Keeping this user-local avoids requiring root
/// or making assumptions about the distribution's package manager.
#[cfg(target_os = "linux")]
pub fn install_desktop_launcher() -> Result<String, String> {
    use std::path::PathBuf;

    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Celeste could not find your Linux home directory.".to_string())?;
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let install_dir = home.join(".local/lib/celeste");
    let installed_executable = install_dir.join("celeste");
    let applications_dir = data_home.join("applications");
    let desktop_file = applications_dir.join(format!(
        "{}.desktop",
        celeste_core::identity::DESKTOP_APP_ID
    ));
    let icon_dir = data_home.join(format!(
        "icons/hicolor/256x256/apps"
    ));
    let icon_file = icon_dir.join(format!(
        "{}.png",
        celeste_core::identity::DESKTOP_APP_ID
    ));

    std::fs::create_dir_all(&install_dir)
        .map_err(|error| format!("Could not create {}: {error}", install_dir.display()))?;
    std::fs::create_dir_all(&applications_dir)
        .map_err(|error| format!("Could not create {}: {error}", applications_dir.display()))?;
    std::fs::create_dir_all(&icon_dir)
        .map_err(|error| format!("Could not create {}: {error}", icon_dir.display()))?;

    let running_executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the running Celeste executable: {error}"))?;
    if !same_file_path(&running_executable, &installed_executable) {
        let temporary = install_dir.join("celeste.installing");
        std::fs::copy(&running_executable, &temporary).map_err(|error| {
            format!(
                "Could not copy Celeste from {} to {}: {error}",
                running_executable.display(),
                temporary.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&temporary)
                .map_err(|error| format!("Could not read installed file permissions: {error}"))?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&temporary, permissions)
                .map_err(|error| format!("Could not make Celeste executable: {error}"))?;
        }
        std::fs::rename(&temporary, &installed_executable).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("Could not finish installing Celeste: {error}")
        })?;
    }

    const ICON: &[u8] = include_bytes!("../../../assets/icons/celeste-256.png");
    std::fs::write(&icon_file, ICON)
        .map_err(|error| format!("Could not install {}: {error}", icon_file.display()))?;

    let executable = desktop_exec_path(&installed_executable);
    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Celeste\n\
         Comment=A VRChat moderation logger based on Scarlet, but coded in Rust.\n\
         Exec={executable}\n\
         TryExec={executable}\n\
         Icon={}\n\
         Terminal=false\n\
         Categories=Utility;Network;\n\
         Keywords=VRChat;Moderation;Discord;\n\
         StartupNotify=true\n\
         StartupWMClass={}\n",
        celeste_core::identity::DESKTOP_APP_ID,
        celeste_core::identity::DESKTOP_APP_ID,
    );
    std::fs::write(&desktop_file, desktop)
        .map_err(|error| format!("Could not install {}: {error}", desktop_file.display()))?;

    // Most desktops notice the file immediately. When the Freedesktop cache
    // utility is present, refresh it now so KDE/GNOME search can find Celeste
    // without waiting for the next login.
    if let Some(update_database) = celeste_core::tts_voices::find_executable("update-desktop-database") {
        let _ = std::process::Command::new(update_database)
            .arg(&applications_dir)
            .status();
    }

    Ok(format!(
        "Celeste was added to your application launcher. The installed copy is {}.",
        installed_executable.display()
    ))
}

#[cfg(target_os = "linux")]
fn same_file_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(target_os = "linux")]
fn desktop_exec_path(path: &std::path::Path) -> String {
    // Desktop-entry Exec quoting treats backslash, quote, backtick and dollar
    // specially inside double quotes. The install location is controlled by
    // Celeste, but HOME itself can legally contain spaces or those bytes.
    let raw = path.to_string_lossy();
    let mut quoted = String::with_capacity(raw.len() + 2);
    quoted.push('"');
    for character in raw.chars() {
        if matches!(character, '\\' | '"' | '`' | '$') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
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

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_entry_exec_path_quotes_spaces_and_reserved_characters() {
        assert_eq!(
            desktop_exec_path(std::path::Path::new("/home/Test User/$Celeste/celeste")),
            r#""/home/Test User/\$Celeste/celeste""#
        );
    }
}
