//! Small native TTS queue used by the desktop UI. It mirrors the operational
//! parts of Java's `TtsService`: serial playback, pause/drop semantics, and an
//! interruptible current utterance. Linux uses the same command-line engines
//! as `LinuxCommandTtsProvider`; macOS uses `say`, and Windows uses SAPI via
//! PowerShell.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const MIXED_CHARACTER_ALERT: &[u8] =
    include_bytes!("../resources/tts/mixed-character-alert.wav");
const NUISANCE_ALERT: &[u8] =
    include_bytes!("../resources/tts/bl_sfx_siren_chirp.wav");

#[derive(Debug, Clone)]
enum Engine {
    Espeak(PathBuf),
    SpdSay(PathBuf),
    Flite(PathBuf),
    MacSay(PathBuf),
    PowerShell(PathBuf),
}

impl Engine {
    fn detect() -> Option<Self> {
        if cfg!(target_os = "linux") {
            find_on_path("espeak-ng")
                .or_else(|| find_on_path("espeak"))
                .map(Self::Espeak)
                .or_else(|| find_on_path("spd-say").map(Self::SpdSay))
                .or_else(|| find_on_path("flite").map(Self::Flite))
        } else if cfg!(target_os = "macos") {
            find_on_path("say").map(Self::MacSay)
        } else if cfg!(target_os = "windows") {
            find_on_path("pwsh")
                .or_else(|| find_on_path("powershell"))
                .map(Self::PowerShell)
        } else {
            None
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Espeak(_) => "eSpeak",
            Self::SpdSay(_) => "Speech Dispatcher",
            Self::Flite(_) => "Flite",
            Self::MacSay(_) => "macOS say",
            Self::PowerShell(_) => "Windows SAPI",
        }
    }

    /// Port of `TtsService.playPreparedAudio` + `playViaSubprocess`
    /// (`util/tts/TtsService.java:345-358`, `:400-415`).
    ///
    /// Java never lets the synthesizer play its own audio on Linux. It
    /// writes a WAV and hands that to `pw-play`/`paplay`/`aplay`, because
    /// those route through PipeWire/PulseAudio — so the announcement shows
    /// up in the volume mixer and follows whichever output device the user
    /// selected. eSpeak left to itself opens a backend of its own choosing
    /// and ignores both, which is why TTS came out of the wrong device.
    ///
    /// Overridable with `CELESTE_TTS_PLAYER` (or `SCARLET_TTS_PLAYER`,
    /// Java's name), which may include arguments. Returns `None` when no
    /// player is available, and the caller then falls back to letting the
    /// engine play directly, as Java falls back to Java Sound.
    #[cfg(target_os = "linux")]
    fn play_via_player(&self, text: &str, voice: &str) -> Option<std::io::Result<Child>> {
        let Self::Espeak(engine) = self else {
            // Only eSpeak can render to a file; spd-say and flite are
            // handed the text and do their own playback.
            return None;
        };
        let prefix = resolve_player_prefix()?;
        let wav = std::env::temp_dir().join(format!(
            "celeste-tts-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let mut synth = Command::new(engine);
        synth.arg("-w").arg(&wav);
        if !voice.trim().is_empty() {
            let voice = voice.rsplit(['/', ':']).next().unwrap_or(voice);
            synth.args(["-v", voice]);
        }
        let synthesized = synth
            .arg(text)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match synthesized {
            Ok(status) if status.success() => {}
            Ok(status) => {
                tracing::warn!("TTS: the speech synthesizer exited with {status}");
                let _ = std::fs::remove_file(&wav);
                return None;
            }
            Err(err) => {
                tracing::warn!("TTS: could not run the speech synthesizer: {err}");
                return None;
            }
        }
        if std::fs::metadata(&wav).map(|meta| meta.len()).unwrap_or(0) == 0 {
            tracing::warn!("TTS: the speech synthesizer produced no audio");
            let _ = std::fs::remove_file(&wav);
            return None;
        }
        let (player, args) = prefix.split_first()?;
        let mut command = Command::new(player);
        command.args(args).arg(&wav);
        let spawned = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        // The player reads the file as it goes, so cleanup waits for the
        // clip to finish rather than racing it; the playback loop is serial
        // so at most one of these is ever outstanding.
        if spawned.is_err() {
            let _ = std::fs::remove_file(&wav);
        }
        Some(spawned)
    }

    fn spawn(&self, text: &str, voice: &str) -> std::io::Result<Child> {
        #[cfg(target_os = "linux")]
        if let Some(result) = self.play_via_player(text, voice) {
            return result;
        }
        let mut command = match self {
            Self::Espeak(path) => {
                let mut command = Command::new(path);
                if !voice.trim().is_empty() {
                    let voice = voice.rsplit(['/', ':']).next().unwrap_or(voice);
                    command.args(["-v", voice]);
                }
                command.arg(text);
                command
            }
            Self::SpdSay(path) => {
                let mut command = Command::new(path);
                command.args(["--wait", text]);
                command
            }
            Self::Flite(path) => {
                let mut command = Command::new(path);
                if !voice.trim().is_empty() {
                    command.args(["-voice", voice]);
                }
                command.args(["-t", text]);
                command
            }
            Self::MacSay(path) => {
                let mut command = Command::new(path);
                if !voice.trim().is_empty() {
                    command.args(["-v", voice]);
                }
                command.arg(text);
                command
            }
            Self::PowerShell(path) => {
                let mut command = Command::new(path);
                command.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Add-Type -AssemblyName System.Speech; $s=[System.Speech.Synthesis.SpeechSynthesizer]::new(); $s.Speak($args[0])",
                    text,
                ]);
                command
            }
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
}

/// Port of `TtsService.resolvePlayerPrefix`. Java tries the
/// `scarlet.tts.player` system property, then `SCARLET_TTS_PLAYER`, then
/// `pw-play`, `paplay`, `aplay` in order. A Rust binary has no system
/// properties, so the environment variable is the override — under Celeste's
/// name first, with Java's still honoured so an existing launcher keeps
/// working.
#[cfg(target_os = "linux")]
fn resolve_player_prefix() -> Option<Vec<String>> {
    for name in ["CELESTE_TTS_PLAYER", "SCARLET_TTS_PLAYER"] {
        if let Ok(value) = std::env::var(name) {
            let parts: Vec<String> =
                value.split_whitespace().map(str::to_string).collect();
            if !parts.is_empty() {
                return Some(parts);
            }
        }
    }
    for player in ["pw-play", "paplay", "aplay"] {
        if let Some(path) = find_on_path(player) {
            return Some(vec![path.to_string_lossy().into_owned()]);
        }
    }
    tracing::warn!(
        "TTS: no external audio player (pw-play/paplay/aplay) on PATH, so announcements \
         will play on whatever device the synthesizer picks rather than your selected \
         output (on Arch: `pacman -S libpipewire` for pw-play, or `libpulse` for paplay)"
    );
    None
}

enum Request {
    Speak { text: String, voice: String },
    Audio { path: PathBuf },
    Shutdown,
}

pub struct TtsService {
    sender: mpsc::Sender<Request>,
    paused: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
    active: Arc<Mutex<Option<Child>>>,
    engine_name: Option<&'static str>,
    mixed_character_alert: Option<PathBuf>,
    nuisance_alert: Option<PathBuf>,
}

impl TtsService {
    pub fn new() -> Self {
        let engine = Engine::detect();
        let engine_name = engine.as_ref().map(Engine::name);
        let mixed_character_alert = extract_alert("mixed-character-alert.wav", MIXED_CHARACTER_ALERT);
        let nuisance_alert = extract_alert("bl_sfx_siren_chirp.wav", NUISANCE_ALERT);
        let (sender, receiver) = mpsc::channel();
        let paused = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let active: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let worker_shutdown = shutting_down.clone();
        std::thread::Builder::new()
            .name("celeste-tts".into())
            .spawn(move || worker(receiver, engine, worker_active, worker_shutdown))
            .expect("spawn TTS worker");
        if let Some(name) = engine_name {
            tracing::info!(engine = name, "TTS service initialized");
        } else {
            tracing::warn!("No supported TTS engine was found on PATH");
        }
        Self {
            sender,
            paused,
            shutting_down,
            active,
            engine_name,
            mixed_character_alert,
            nuisance_alert,
        }
    }

    pub fn engine_name(&self) -> Option<&'static str> {
        self.engine_name
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn submit(&self, text: impl Into<String>, voice: impl Into<String>) -> Result<(), String> {
        if self.paused.load(Ordering::SeqCst) {
            return Ok(());
        }
        let text = text.into();
        if text.trim().is_empty() {
            return Ok(());
        }
        if self.engine_name.is_none() {
            return Err("No supported TTS engine is installed (tried eSpeak NG/eSpeak, Speech Dispatcher, and Flite).".into());
        }
        self.sender
            .send(Request::Speak {
                text,
                voice: voice.into(),
            })
            .map_err(|_| "The TTS worker stopped.".to_string())
    }

    fn submit_audio(&self, path: Option<&PathBuf>) -> Result<(), String> {
        if self.paused.load(Ordering::SeqCst) {
            return Ok(());
        }
        let path = path
            .cloned()
            .ok_or_else(|| "The bundled alert sound could not be prepared.".to_string())?;
        self.sender
            .send(Request::Audio { path })
            .map_err(|_| "The TTS worker stopped.".to_string())
    }

    /// Java's `submitMixedCharacterNameJoinAlert`: alert first, then the
    /// localized warning, on the same serial playback queue.
    pub fn submit_mixed_character_alert(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_audio(self.mixed_character_alert.as_ref())?;
        self.submit(text, voice)
    }

    /// Java's `submitNuisanceJoinAlert`: siren chirp first, then the joined
    /// user's combined announcement, without overlapping either clip.
    pub fn submit_nuisance_alert(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_audio(self.nuisance_alert.as_ref())?;
        self.submit(text, voice)
    }

    pub fn skip(&self) -> bool {
        let mut active = self.active.lock().unwrap();
        active.as_mut().is_some_and(|child| child.kill().is_ok())
    }
}

impl Drop for TtsService {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.sender.send(Request::Shutdown);
        let _ = self.active.lock().unwrap().as_mut().map(Child::kill);
    }
}

fn worker(
    receiver: mpsc::Receiver<Request>,
    engine: Option<Engine>,
    active: Arc<Mutex<Option<Child>>>,
    shutting_down: Arc<AtomicBool>,
) {
    while let Ok(request) = receiver.recv() {
        if shutting_down.load(Ordering::SeqCst) {
            break;
        }
        let spawned = match request {
            Request::Speak { text, voice } => {
                let Some(engine) = &engine else { continue };
                engine.spawn(&text, &voice)
            }
            Request::Audio { path } => spawn_audio_file(&path),
            Request::Shutdown => break,
        };
        match spawned {
            Ok(child) => *active.lock().unwrap() = Some(child),
            Err(err) => {
                tracing::warn!("Could not start TTS engine: {err}");
                continue;
            }
        }
        loop {
            if shutting_down.load(Ordering::SeqCst) {
                let _ = active.lock().unwrap().as_mut().map(Child::kill);
                break;
            }
            let finished = {
                let mut guard = active.lock().unwrap();
                match guard
                    .as_mut()
                    .and_then(|child| child.try_wait().ok())
                    .flatten()
                {
                    Some(_) => {
                        guard.take();
                        true
                    }
                    None => false,
                }
            };
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}

fn extract_alert(name: &str, bytes: &[u8]) -> Option<PathBuf> {
    if !bytes.starts_with(b"RIFF") || bytes.len() <= 44 {
        tracing::warn!(name, "Bundled TTS alert is not a valid WAV file");
        return None;
    }
    let directory = std::env::temp_dir().join("celeste-sfx");
    if let Err(error) = std::fs::create_dir_all(&directory) {
        tracing::warn!(%error, "Could not create the temporary alert-sound directory");
        return None;
    }
    let path = directory.join(name);
    if std::fs::write(&path, bytes).is_err() {
        tracing::warn!(path = %path.display(), "Could not extract a bundled TTS alert");
        return None;
    }
    Some(path)
}

fn spawn_audio_file(path: &Path) -> std::io::Result<Child> {
    #[cfg(target_os = "linux")]
    {
        let prefix = resolve_player_prefix().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no WAV player is available (tried pw-play, paplay, and aplay)",
            )
        })?;
        let (player, args) = prefix.split_first().expect("non-empty player command");
        return Command::new(player)
            .args(args)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let player = find_on_path("afplay").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "afplay is unavailable")
        })?;
        return Command::new(player).arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let player = find_on_path("pwsh")
            .or_else(|| find_on_path("powershell"))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "PowerShell is unavailable")
            })?;
        return Command::new(player)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$p=[System.Media.SoundPlayer]::new($args[0]); $p.PlaySync()",
            ])
            .arg(path)
            .spawn();
    }
    #[allow(unreachable_code)]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "WAV alert playback is unsupported on this operating system",
    ))
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(executable);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let candidate = directory.join(format!("{executable}.exe"));
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    /// Java lets `scarlet.tts.player` / `SCARLET_TTS_PLAYER` override the
    /// player, arguments included. Celeste's own variable wins, and Java's
    /// is still honoured so an existing launcher keeps working.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_player_override_is_read_from_either_variable() {
        // These run in-process, so keep the mutation tight and restore it.
        let saved = (
            std::env::var("CELESTE_TTS_PLAYER").ok(),
            std::env::var("SCARLET_TTS_PLAYER").ok(),
        );
        std::env::remove_var("CELESTE_TTS_PLAYER");
        std::env::set_var("SCARLET_TTS_PLAYER", "legacy-player --arg");
        assert_eq!(
            resolve_player_prefix(),
            Some(vec!["legacy-player".to_string(), "--arg".to_string()])
        );
        std::env::set_var("CELESTE_TTS_PLAYER", "new-player");
        assert_eq!(resolve_player_prefix(), Some(vec!["new-player".to_string()]));
        // A blank value is not an override.
        std::env::set_var("CELESTE_TTS_PLAYER", "   ");
        std::env::remove_var("SCARLET_TTS_PLAYER");
        let resolved = resolve_player_prefix();
        assert_ne!(resolved, Some(vec!["   ".to_string()]));

        std::env::remove_var("CELESTE_TTS_PLAYER");
        std::env::remove_var("SCARLET_TTS_PLAYER");
        if let Some(value) = saved.0 {
            std::env::set_var("CELESTE_TTS_PLAYER", value);
        }
        if let Some(value) = saved.1 {
            std::env::set_var("SCARLET_TTS_PLAYER", value);
        }
    }

    use super::*;

    #[test]
    fn java_alert_assets_are_bundled_as_playable_wav_files() {
        for bytes in [MIXED_CHARACTER_ALERT, NUISANCE_ALERT] {
            assert!(bytes.starts_with(b"RIFF"));
            assert_eq!(&bytes[8..12], b"WAVE");
            assert!(bytes.len() > 44);
        }
    }

    #[test]
    fn current_system_detects_an_engine_when_one_is_available() {
        if [
            "espeak-ng",
            "espeak",
            "spd-say",
            "flite",
            "say",
            "pwsh",
            "powershell",
        ]
        .into_iter()
        .any(|name| find_on_path(name).is_some())
        {
            assert!(Engine::detect().is_some());
        }
    }

    #[test]
    fn disabling_default_audio_selects_discord_only() {
        assert_eq!(select_output_route(true, true), OutputRoute::Local);
        assert_eq!(select_output_route(false, true), OutputRoute::Discord);
        assert_eq!(select_output_route(false, false), OutputRoute::Unavailable);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputRoute {
    Local,
    Discord,
    Unavailable,
}

fn select_output_route(use_default_audio_device: bool, discord_available: bool) -> OutputRoute {
    if use_default_audio_device {
        OutputRoute::Local
    } else if discord_available {
        OutputRoute::Discord
    } else {
        OutputRoute::Unavailable
    }
}

/// Adapts the desktop [`TtsService`] to the event listener's
/// [`scarlet_events::TtsSink`], which is how the ported
/// `ScarletEventListener` speaks without depending on this GUI crate.
///
/// The voice is read from settings on every utterance (rather than captured
/// once) so changing it in the Settings tab takes effect immediately, the
/// same way Java's `ttsVoiceName.get()` call inside each announcement does.
///
pub struct TtsSinkAdapter {
    service: std::sync::Arc<TtsService>,
    settings: std::sync::Arc<scarlet_core::settings::Settings>,
    discord: Option<scarlet_discord::voice_tts::DiscordVoiceTts>,
    ui: Option<crate::UiState>,
}

impl TtsSinkAdapter {
    pub fn new(
        service: std::sync::Arc<TtsService>,
        settings: std::sync::Arc<scarlet_core::settings::Settings>,
        discord: Option<scarlet_discord::voice_tts::DiscordVoiceTts>,
        ui: Option<crate::UiState>,
    ) -> Self {
        TtsSinkAdapter { service, settings, discord, ui }
    }

    /// Copies the Discord voice route's last problem into the status bar.
    /// The queue is asynchronous, so this reflects the *previous*
    /// announcement -- enough to turn "TTS isn't coming through" into a
    /// sentence naming the cause, without blocking the caller on playback.
    fn publish_route_status(&self) {
        let (Some(ui), Some(discord)) = (self.ui.as_ref(), self.discord.as_ref()) else {
            return;
        };
        ui.set_tts_status(discord.last_problem());
    }

    /// Port of `TtsService.playPreparedAudio`'s routing decision.
    ///
    /// `tts_use_default_audio_device` is a *route* switch, not a volume
    /// knob: on, the announcement plays on this machine's default output
    /// device; off, it goes to the Discord voice channel instead. This used
    /// to do both at once regardless of the setting, so turning it off
    /// changed nothing audible — which is what made the setting look
    /// broken.
    fn speak_now(&self, marker: &str, text: &str) {
        let voice = self.settings.tts_voice_name.get();
        match select_output_route(
            self.settings.tts_use_default_audio_device.get(),
            self.discord.is_some(),
        ) {
            OutputRoute::Local => {
                if let Err(err) = self.service.submit(text, voice) {
                    tracing::debug!("TTS({marker}): {err}");
                }
            }
            OutputRoute::Discord => {
                let discord = self.discord.as_ref().expect("route checked Discord availability");
                discord.submit(text, &voice);
                self.publish_route_status();
            }
            // Java logs exactly this and drops the utterance rather than
            // falling back to the speakers, because "not the local device"
            // is the thing the user asked for.
            OutputRoute::Unavailable => {
                let problem = "TTS has no output route: Discord voice isn't available and \
                               \"use default audio device\" is off.";
                tracing::warn!("TTS({marker}): {problem}");
                if let Some(ui) = self.ui.as_ref() {
                    ui.set_tts_status(Some(problem.to_string()));
                }
            }
        }
    }

    /// Route an explicit utterance through the same switch used by event
    /// callouts. The desktop CLI uses this so `tts <text>` does not bypass
    /// Discord whenever local-device playback is disabled.
    pub fn submit_routed(&self, marker: &str, text: &str) {
        self.speak_now(marker, text);
    }
}

impl scarlet_events::TtsSink for TtsSinkAdapter {
    fn speak(&self, marker: &str, text: &str) {
        self.speak_now(marker, text);
    }

    fn speak_after_nuisance_alert(&self, marker: &str, text: &str) {
        let voice = self.settings.tts_voice_name.get();
        if self.settings.tts_use_default_audio_device.get() {
            if let Err(err) = self.service.submit_nuisance_alert(text, voice) {
                tracing::debug!("TTS({marker}): {err}");
            }
        } else if let Some(discord) = &self.discord {
            discord.submit_audio(NUISANCE_ALERT, "nuisance-alert");
            discord.submit(text, &voice);
            self.publish_route_status();
        } else {
            self.speak_now(marker, text);
        }
    }

    fn speak_mixed_character_name_alert(&self, marker: &str) {
        let text = scarlet_core::i18n::tr("adv.ttsMixedCharAlert", &[]);
        let voice = self.settings.tts_voice_name.get();
        if self.settings.tts_use_default_audio_device.get() {
            if let Err(err) = self.service.submit_mixed_character_alert(text, voice) {
                tracing::debug!("TTS({marker}): {err}");
            }
        } else if let Some(discord) = &self.discord {
            discord.submit_audio(MIXED_CHARACTER_ALERT, "mixed-character-alert");
            discord.submit(&text, &voice);
            self.publish_route_status();
        } else {
            self.speak_now(marker, &text);
        }
    }
}
