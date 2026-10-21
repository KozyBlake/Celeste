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

pub const RENDERER_ENV: &str = "CELESTE_RENDERER";
pub const RENDERER_RECOVERY_ENV: &str = "CELESTE_RENDERER_RECOVERY";
pub const RENDERER_RECOVERY_REASON_ENV: &str = "CELESTE_RENDERER_RECOVERY_REASON";

/// Resolve the renderer requested for this process. A recovery relaunch has
/// highest priority so an existing `--renderer auto` argument cannot send it
/// straight back into the backend that just failed. Ordinary command-line
/// overrides win over the environment, which wins over the saved setting.
pub fn renderer_preference(saved: &str) -> Result<String, String> {
    if let Some(value) = std::env::var_os(RENDERER_RECOVERY_ENV) {
        return validate_renderer(&value.to_string_lossy());
    }

    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let argument = argument.to_string_lossy();
        if argument == "--safe-mode" {
            // Safe mode has to mean "the renderer with the fewest
            // requirements", and on Windows that is the software rasterizer,
            // not OpenGL: a Windows box with no graphics driver only offers
            // GDI's OpenGL 1.1, far below egui_glow's 3.3. Elsewhere Mesa's
            // llvmpipe makes OpenGL the softest landing.
            return Ok(if cfg!(windows) { "software" } else { "opengl" }.to_string());
        }
        if argument == "--renderer" {
            let value = arguments
                .next()
                .ok_or_else(|| "--renderer needs one of: auto, directx12, vulkan, opengl".to_string())?;
            return validate_renderer(&value.to_string_lossy());
        }
        if let Some(value) = argument.strip_prefix("--renderer=") {
            return validate_renderer(value);
        }
    }

    if let Some(value) = std::env::var_os(RENDERER_ENV) {
        return validate_renderer(&value.to_string_lossy());
    }
    validate_renderer(saved)
}

fn validate_renderer(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "auto" | "vulkan" | "opengl" | "software" => Ok(value),
        "directx12" if cfg!(windows) => Ok(value),
        "directx12" => Err("DirectX 12 is only available on Windows".to_string()),
        _ => Err(format!(
            "Unknown graphics renderer `{value}`; use auto, directx12, vulkan, opengl, or software"
        )),
    }
}

/// The next renderer to try after `current` failed to open a window.
///
/// Each rung asks less of the machine than the one above it, and the ladder
/// ends, so recovery cannot loop. On Windows the software rasterizer comes
/// before OpenGL: a Windows machine with no graphics driver has only GDI's
/// OpenGL 1.1, far below the 3.3 egui_glow needs, while the Direct3D 12
/// software rasterizer is part of the operating system and always present.
/// Everywhere else OpenGL is the softer landing, because Mesa's llvmpipe is
/// a complete software OpenGL, so "auto" already went through Glow.
pub fn next_recovery_renderer(current: &str) -> Option<&'static str> {
    match current {
        "auto" | "directx12" | "vulkan" => Some("software"),
        "software" if cfg!(windows) => Some("opengl"),
        _ => None,
    }
}

pub fn renderer_recovery_reason() -> Option<String> {
    std::env::var(RENDERER_RECOVERY_REASON_ENV).ok()
}

pub fn is_renderer_recovery() -> bool {
    std::env::var_os(RENDERER_RECOVERY_ENV).is_some()
}

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
/// which cannot run egui_glow, so a Windows machine with no graphics driver
/// can only be drawn to through wgpu, which can reach the Direct3D 12
/// software rasterizer. Linux keeps the proven Glow path, where Mesa's
/// llvmpipe is a complete software OpenGL 4.5 and needs no special handling.
pub fn native_renderer(preference: &str) -> eframe::Renderer {
    match preference {
        "opengl" => eframe::Renderer::Glow,
        "directx12" | "vulkan" | "software" => eframe::Renderer::Wgpu,
        _ if cfg!(windows) => eframe::Renderer::Wgpu,
        _ => eframe::Renderer::Glow,
    }
}

pub fn native_renderer_label(preference: &str) -> &'static str {
    match preference {
        "directx12" => "DirectX 12 (wgpu)",
        "vulkan" => "Vulkan (wgpu)",
        "opengl" => "OpenGL (Glow)",
        "software" => "Software rendering (wgpu, CPU adapter)",
        _ if cfg!(windows) => "Automatic: DirectX 12, Vulkan or OpenGL, then software (wgpu)",
        _ => "Automatic: OpenGL (Glow)",
    }
}

/// Graphics APIs to enumerate adapters from. `None` keeps eframe's defaults.
fn requested_backends(preference: &str) -> Option<eframe::wgpu::Backends> {
    use eframe::wgpu::Backends;

    // Enumerating every native backend costs nothing when one is absent --
    // it simply contributes no adapters -- and is what lets a single build
    // cover current hardware, hardware too old for Direct3D 12, and machines
    // with no usable GPU driver at all. Which API exposes the software
    // rasterizer differs by platform: WARP is a Direct3D 12 adapter on
    // Windows, while lavapipe (Vulkan) and llvmpipe (GL) fill that role
    // elsewhere, so no single backend can be the fallback everywhere.
    let every_native_backend = Backends::DX12 | Backends::VULKAN | Backends::GL;
    match preference {
        "directx12" => Some(Backends::DX12),
        "vulkan" => Some(Backends::VULKAN),
        "auto" | "software" => Some(every_native_backend),
        _ => None,
    }
}

/// Ordering between graphics APIs when one machine exposes the same GPU
/// through several of them, which is normal on Windows.
fn backend_preference_rank(backend: eframe::wgpu::Backend) -> u8 {
    use eframe::wgpu::Backend;
    match backend {
        Backend::Dx12 | Backend::Metal => 0,
        Backend::Vulkan => 1,
        Backend::Gl => 2,
        _ => 3,
    }
}

/// Ordering between kinds of adapter. A CPU adapter sorts last but remains a
/// valid answer -- on a machine with no usable GPU driver it is the only one.
fn device_type_rank(device_type: eframe::wgpu::DeviceType, prefer_low_power: bool) -> u8 {
    use eframe::wgpu::DeviceType;
    match device_type {
        DeviceType::DiscreteGpu => u8::from(prefer_low_power),
        DeviceType::IntegratedGpu => u8::from(!prefer_low_power),
        // Paravirtualised GPUs (virgl and friends) are real acceleration.
        DeviceType::VirtualGpu => 2,
        DeviceType::Other => 3,
        DeviceType::Cpu => 4,
    }
}

fn describe_adapter(info: &eframe::wgpu::AdapterInfo) -> String {
    let mut description = format!("{} [{:?}/{:?}]", info.name, info.backend, info.device_type);
    if !info.driver.is_empty() {
        description.push_str(" driver ");
        description.push_str(&info.driver);
    }
    if !info.driver_info.is_empty() {
        description.push_str(" (");
        description.push_str(&info.driver_info);
        description.push(')');
    }
    description
}

fn describe_adapters(adapters: &[&eframe::wgpu::Adapter]) -> String {
    adapters
        .iter()
        .map(|adapter| describe_adapter(&adapter.get_info()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Celeste's own adapter policy, replacing wgpu's `request_adapter`.
///
/// The rules it exists to satisfy, in order:
///
/// 1. A CPU adapter is a valid answer, not a failure. wgpu only returns one
///    from `request_adapter` when asked for a fallback adapter explicitly,
///    which eframe never does, so the default path fails outright on a
///    machine whose only renderer is the software rasterizer. That is the
///    common case on a virtual machine and the reason Celeste could not open
///    a window there at all.
/// 2. Real hardware still wins whenever it is present, through any API.
/// 3. Among equals, keep the order the operating system enumerated. Adapter
///    policy on a multi-GPU machine belongs to the OS and the graphics
///    driver, not to a moderation logger; the sort below is stable, so it
///    only ever reorders across the classes it names.
///
/// Every adapter found is logged, because the alternative when a user
/// reports a blank or missing window is guessing about their hardware.
fn choose_adapter(
    adapters: &[eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
    software_only: bool,
) -> Result<eframe::wgpu::Adapter, String> {
    use eframe::wgpu::DeviceType;

    if adapters.is_empty() {
        return Err(
            "this machine exposed no DirectX 12, Vulkan or OpenGL graphics adapters at all"
                .to_string(),
        );
    }

    for adapter in adapters {
        tracing::info!(
            adapter = %describe_adapter(&adapter.get_info()),
            "Celeste found a graphics adapter"
        );
    }

    let mut candidates = adapters
        .iter()
        .filter(|adapter| !software_only || adapter.get_info().device_type == DeviceType::Cpu)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        let found = adapters.iter().collect::<Vec<_>>();
        return Err(format!(
            "no software graphics adapter is available; this machine offers only: {}",
            describe_adapters(&found)
        ));
    }

    // An adapter that cannot present to Celeste's window is unusable however
    // capable it is. A display-only driver and a headless compute device
    // both show up here.
    if let Some(surface) = surface {
        let presentable = candidates
            .iter()
            .copied()
            .filter(|adapter| !surface.get_capabilities(adapter).formats.is_empty())
            .collect::<Vec<_>>();
        if presentable.is_empty() {
            return Err(format!(
                "none of this machine's graphics adapters can draw to a window: {}",
                describe_adapters(&candidates)
            ));
        }
        candidates = presentable;
    }

    let prefer_low_power = std::env::var("WGPU_POWER_PREF")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("low"));
    candidates.sort_by_key(|adapter| {
        let info = adapter.get_info();
        (
            device_type_rank(info.device_type, prefer_low_power),
            backend_preference_rank(info.backend),
        )
    });

    let chosen = candidates[0].clone();
    let info = chosen.get_info();
    if info.device_type == DeviceType::Cpu {
        tracing::warn!(
            adapter = %describe_adapter(&info),
            "Celeste is rendering in software because no usable GPU was found. This is expected \
             on a virtual machine or a system without graphics drivers: slower, but correct."
        );
    } else {
        tracing::info!(
            adapter = %describe_adapter(&info),
            "Celeste selected a graphics adapter"
        );
    }
    Ok(chosen)
}

/// Keep Celeste's tiny 2D interface from using game-style allocation, and
/// install the adapter policy above in place of wgpu's own.
///
/// `WGPU_BACKEND` still overrides the backend set, and `WGPU_POWER_PREF=low`
/// still asks for the integrated GPU ahead of the discrete one; both remain
/// escape hatches for unusual machines and diagnostics.
pub fn native_wgpu_options(preference: &str) -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};

    let mut options = WgpuConfiguration::default();
    let software_only = preference == "software";

    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        if std::env::var_os("WGPU_BACKEND").is_none() {
            if let Some(backends) = requested_backends(preference) {
                setup.instance_descriptor.backends = backends;
            }
        }
        setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters, surface| {
            choose_adapter(adapters, surface, software_only)
        }));

        // Celeste renders a handful of panels, text and one small icon, so it
        // has no reason to trade a larger GPU heap for game-style throughput.
        // Asking for exactly what the chosen adapter reports also stops a
        // weak or software adapter being rejected for failing to meet limits
        // Celeste never uses.
        setup.device_descriptor = std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
            label: Some("celeste wgpu device"),
            required_features: eframe::wgpu::Features::default(),
            required_limits: adapter.limits(),
            memory_hints: eframe::wgpu::MemoryHints::MemoryUsage,
        });
    }

    options.desired_maximum_frame_latency = Some(1);
    options
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
    let icon_theme_dir = data_home.join("icons/hicolor");

    std::fs::create_dir_all(&install_dir)
        .map_err(|error| format!("Could not create {}: {error}", install_dir.display()))?;
    std::fs::create_dir_all(&applications_dir)
        .map_err(|error| format!("Could not create {}: {error}", applications_dir.display()))?;

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

    const ICONS: &[(&str, &[u8])] = &[
        ("16x16", include_bytes!("../../../assets/icons/celeste-16.png")),
        ("24x24", include_bytes!("../../../assets/icons/celeste-24.png")),
        ("32x32", include_bytes!("../../../assets/icons/celeste-32.png")),
        ("48x48", include_bytes!("../../../assets/icons/celeste-48.png")),
        ("64x64", include_bytes!("../../../assets/icons/celeste-64.png")),
        ("128x128", include_bytes!("../../../assets/icons/celeste-128.png")),
        ("256x256", include_bytes!("../../../assets/icons/celeste-256.png")),
        ("512x512", include_bytes!("../../../assets/icons/celeste-512.png")),
    ];
    for (size, icon) in ICONS {
        let icon_dir = icon_theme_dir.join(size).join("apps");
        let icon_file = icon_dir.join(format!(
            "{}.png",
            celeste_core::identity::DESKTOP_APP_ID
        ));
        std::fs::create_dir_all(&icon_dir)
            .map_err(|error| format!("Could not create {}: {error}", icon_dir.display()))?;
        std::fs::write(&icon_file, icon)
            .map_err(|error| format!("Could not install {}: {error}", icon_file.display()))?;
    }

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

    // Most desktops notice these files eventually, but launchers and pinned
    // taskbar entries can retain an old icon until their caches are rebuilt.
    // Every command is optional and best-effort so minimal distributions can
    // still install the launcher without pulling in desktop-specific tools.
    if let Some(update_icons) =
        celeste_core::tts_voices::find_executable("gtk-update-icon-cache")
    {
        let _ = std::process::Command::new(update_icons)
            .args(["--force", "--ignore-theme-index"])
            .arg(&icon_theme_dir)
            .status();
    }
    if let Some(update_database) = celeste_core::tts_voices::find_executable("update-desktop-database") {
        let _ = std::process::Command::new(update_database)
            .arg(&applications_dir)
            .status();
    }
    if let Some(kbuildsycoca) = celeste_core::tts_voices::find_executable("kbuildsycoca6")
        .or_else(|| celeste_core::tts_voices::find_executable("kbuildsycoca5"))
    {
        let _ = std::process::Command::new(kbuildsycoca)
            .arg("--noincremental")
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

/// Raises a native desktop notification.
///
/// Shelled out rather than taken as a dependency, matching how Celeste
/// already reaches speech engines and audio players: no extra crate, and a
/// missing tool degrades to "no notification" instead of a build that only
/// works on one distribution.
///
/// Windows is deliberately absent. Its native idiom for "a window wants
/// you" is the taskbar flash, which Celeste already requests, and raising a
/// real toast needs either a module that is not installed by default or
/// hand-written WinRT XML.
pub fn desktop_notification(summary: &str, body: &str) {
    #[cfg(target_os = "linux")]
    {
        let spawned = std::process::Command::new("notify-send")
            .args(["--app-name=Celeste", "--urgency=normal", summary, body])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(error) = spawned {
            tracing::debug!(%error, "notify-send is unavailable; no desktop notification");
        }
    }
    #[cfg(target_os = "macos")]
    {
        // AppleScript string literals take backslash escapes, and an
        // unescaped quote in a player name would otherwise end the script.
        let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"Celeste\" subtitle \"{}\"",
            escape(body),
            escape(summary)
        );
        let spawned = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(error) = spawned {
            tracing::debug!(%error, "osascript is unavailable; no desktop notification");
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (summary, body);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn open_audio_settings(_panel: AudioSettingsPanel) -> Result<&'static str, String> {
    Err("Celeste cannot open sound settings automatically on this operating system.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_names_are_normalized_and_unknown_values_are_rejected() {
        assert_eq!(validate_renderer(" OpenGL ").unwrap(), "opengl");
        assert_eq!(validate_renderer("VULKAN").unwrap(), "vulkan");
        assert!(validate_renderer("potato").is_err());
        #[cfg(target_os = "windows")]
        assert_eq!(validate_renderer("directx12").unwrap(), "directx12");
        #[cfg(not(target_os = "windows"))]
        assert!(validate_renderer("directx12").is_err());
    }

    #[test]
    fn explicit_renderer_choices_select_the_expected_eframe_backend() {
        assert!(matches!(native_renderer("opengl"), eframe::Renderer::Glow));
        assert!(matches!(native_renderer("vulkan"), eframe::Renderer::Wgpu));
        assert!(matches!(native_renderer("directx12"), eframe::Renderer::Wgpu));
    }

    /// Software rendering has to be reachable on every platform, because a
    /// machine with no usable graphics driver exists on all of them.
    #[test]
    fn software_rendering_is_selectable_everywhere() {
        assert_eq!(validate_renderer(" Software ").unwrap(), "software");
        assert!(matches!(native_renderer("software"), eframe::Renderer::Wgpu));
        assert!(native_renderer_label("software").to_lowercase().contains("software"));
    }

    /// The recovery ladder must always reach the ground. If a rung ever
    /// pointed back up, Celeste would relaunch itself forever.
    #[test]
    fn the_recovery_ladder_always_terminates_without_repeating() {
        for start in ["auto", "directx12", "vulkan", "software", "opengl"] {
            let mut seen = vec![start.to_string()];
            let mut current = start.to_string();
            while let Some(next) = next_recovery_renderer(&current) {
                assert!(
                    !seen.iter().any(|rung| rung == next),
                    "recovery from {start} revisits {next}: {seen:?}"
                );
                assert!(
                    validate_renderer(next).is_ok() || !cfg!(windows),
                    "recovery from {start} proposes the invalid renderer {next}"
                );
                seen.push(next.to_string());
                current = next.to_string();
            }
            assert!(seen.len() <= 4, "recovery from {start} took too many steps: {seen:?}");
        }
        assert_eq!(next_recovery_renderer("opengl"), None);
    }

    /// Restricting Windows to Direct3D 12 is what left machines with Vulkan
    /// or OpenGL but no D3D12 -- which is to say older machines -- with
    /// nothing to render on.
    #[test]
    fn automatic_and_software_enumerate_every_native_backend() {
        use eframe::wgpu::Backends;
        for preference in ["auto", "software"] {
            let backends = requested_backends(preference)
                .unwrap_or_else(|| panic!("{preference} should pin an explicit backend set"));
            for (name, backend) in [
                ("DX12", Backends::DX12),
                ("Vulkan", Backends::VULKAN),
                ("GL", Backends::GL),
            ] {
                assert!(
                    backends.contains(backend),
                    "{preference} must still enumerate {name}"
                );
            }
        }
        assert_eq!(requested_backends("vulkan"), Some(Backends::VULKAN));
    }

    #[test]
    fn hardware_outranks_software_and_low_power_flips_only_the_gpus() {
        use eframe::wgpu::DeviceType;
        let rank = |device_type| device_type_rank(device_type, false);
        assert!(rank(DeviceType::DiscreteGpu) < rank(DeviceType::IntegratedGpu));
        assert!(rank(DeviceType::IntegratedGpu) < rank(DeviceType::VirtualGpu));
        assert!(rank(DeviceType::VirtualGpu) < rank(DeviceType::Other));
        // The whole point: a CPU adapter is last, but it is still on the list.
        assert!(rank(DeviceType::Other) < rank(DeviceType::Cpu));

        let low = |device_type| device_type_rank(device_type, true);
        assert!(low(DeviceType::IntegratedGpu) < low(DeviceType::DiscreteGpu));
        assert_eq!(low(DeviceType::Cpu), rank(DeviceType::Cpu));
    }

    /// The regression this whole renderer cascade exists for.
    ///
    /// wgpu's own `request_adapter` will not return a CPU adapter unless it
    /// is asked for a fallback adapter explicitly, which eframe never does,
    /// so a machine whose only renderer is a software rasterizer failed with
    /// "no suitable adapter found". Celeste's selector has to accept one.
    ///
    /// This runs against whatever the build machine really has. It asserts
    /// nothing when no adapter at all is present, since that is a property of
    /// the machine rather than of the code under test.
    #[test]
    fn the_adapter_policy_accepts_a_software_adapter() {
        use eframe::wgpu;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: requested_backends("auto").unwrap_or(wgpu::Backends::all()),
            ..Default::default()
        });
        let adapters = instance.enumerate_adapters(wgpu::Backends::all());
        // A hardware-dependent test should say what hardware it ran against,
        // or a pass on a machine with nothing installed looks like a pass on
        // a machine that was properly exercised.
        let found = adapters.iter().collect::<Vec<_>>();
        eprintln!("adapters on this machine: [{}]", describe_adapters(&found));
        if adapters.is_empty() {
            eprintln!("no graphics adapters here, so there is nothing to assert");
            return;
        }

        let chosen = choose_adapter(&adapters, None, false)
            .expect("an adapter exists, so selection must not fail");
        assert!(adapters.iter().any(|a| a.get_info() == chosen.get_info()));

        let software = adapters
            .iter()
            .any(|a| a.get_info().device_type == wgpu::DeviceType::Cpu);
        if software {
            let chosen = choose_adapter(&adapters, None, true)
                .expect("a CPU adapter is present, so forcing software must succeed");
            assert_eq!(
                chosen.get_info().device_type,
                wgpu::DeviceType::Cpu,
                "forcing software must not quietly hand back a GPU"
            );
        } else {
            assert!(
                choose_adapter(&adapters, None, true).is_err(),
                "with no CPU adapter present, forcing software must say so rather than \
                 silently falling back to hardware"
            );
        }
    }

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
