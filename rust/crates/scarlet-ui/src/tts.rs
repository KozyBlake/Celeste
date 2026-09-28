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

    fn spawn(&self, text: &str, voice: &str) -> std::io::Result<Child> {
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

enum Request {
    Speak { text: String, voice: String },
    Shutdown,
}

pub struct TtsService {
    sender: mpsc::Sender<Request>,
    paused: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
    active: Arc<Mutex<Option<Child>>>,
    engine_name: Option<&'static str>,
}

impl TtsService {
    pub fn new() -> Self {
        let engine = Engine::detect();
        let engine_name = engine.as_ref().map(Engine::name);
        let (sender, receiver) = mpsc::channel();
        let paused = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let active: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let worker_shutdown = shutting_down.clone();
        std::thread::Builder::new()
            .name("scarlet-tts".into())
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
        let Request::Speak { text, voice } = request else {
            break;
        };
        let Some(engine) = &engine else { continue };
        match engine.spawn(&text, &voice) {
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
    use super::*;

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
}

/// Adapts the desktop [`TtsService`] to the event listener's
/// [`scarlet_events::TtsSink`], which is how the ported
/// `ScarletEventListener` speaks without depending on this GUI crate.
///
/// The voice is read from settings on every utterance (rather than captured
/// once) so changing it in the Settings tab takes effect immediately, the
/// same way Java's `ttsVoiceName.get()` call inside each announcement does.
///
/// **Gap, tracked not hidden:** Java precedes the nuisance-rank and
/// mixed-character-name announcements with bundled alert tones
/// (`BL_SFX_*.mp3`, played through `submitAlertThenSpeak`). This service has
/// no audio-file playback yet, so those two calls speak the announcement
/// without its chime. The distinction a moderator relies on — that a
/// nuisance-rank join *sounds* different — is therefore not reproduced yet;
/// see PORTING.md.
pub struct TtsSinkAdapter {
    service: std::sync::Arc<TtsService>,
    settings: std::sync::Arc<scarlet_core::settings::Settings>,
    discord: Option<scarlet_discord::voice_tts::DiscordVoiceTts>,
}

impl TtsSinkAdapter {
    pub fn new(
        service: std::sync::Arc<TtsService>,
        settings: std::sync::Arc<scarlet_core::settings::Settings>,
        discord: Option<scarlet_discord::voice_tts::DiscordVoiceTts>,
    ) -> Self {
        TtsSinkAdapter { service, settings, discord }
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
        if self.settings.tts_use_default_audio_device.get() {
            if let Err(err) = self.service.submit(text, voice) {
                tracing::debug!("TTS({marker}): {err}");
            }
            return;
        }
        match &self.discord {
            Some(discord) => discord.submit(text, &voice),
            // Java logs exactly this and drops the utterance rather than
            // falling back to the speakers, because "not the local device"
            // is the thing the user asked for.
            None => tracing::warn!("TTS({marker}): No audio output route available (Discord voice is not connected, and \
                 \"use default audio device\" is off)"),
        }
    }
}

impl scarlet_events::TtsSink for TtsSinkAdapter {
    fn speak(&self, marker: &str, text: &str) {
        self.speak_now(marker, text);
    }

    fn speak_after_nuisance_alert(&self, marker: &str, text: &str) {
        // No chime yet (see the type's docs); the announcement itself still
        // goes out, which is the part that carries the information.
        self.speak_now(marker, text);
    }

    fn speak_mixed_character_name_alert(&self, marker: &str) {
        self.speak_now(marker, &scarlet_core::i18n::tr("adv.ttsMixedCharAlert", &[]));
    }
}
