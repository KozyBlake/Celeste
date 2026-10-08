//! Small native TTS queue used by the desktop UI. It mirrors the operational
//! parts of Java's `TtsService`: serial playback, pause/drop semantics, and an
//! interruptible current utterance. Linux uses the same command-line engines
//! as `LinuxCommandTtsProvider`; macOS uses `say`, and Windows uses SAPI via
//! PowerShell.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const MIXED_CHARACTER_ALERT: &[u8] = include_bytes!("../resources/tts/mixed-character-alert.wav");
const NUISANCE_ALERT: &[u8] = include_bytes!("../resources/tts/bl_sfx_siren_chirp.wav");

#[derive(Debug, Clone)]
enum Engine {
    Espeak(PathBuf),
    SpdSay(PathBuf),
    Flite(PathBuf),
    /// SVOX Pico. Enumerated by `tts_voices` because Java offers it, so it
    /// has to be speakable too — offering a voice that cannot be spoken is
    /// worse than not offering it.
    Pico(PathBuf),
    MacSay(PathBuf),
    PowerShell(PathBuf),
}

impl Engine {
    /// The engine named by a `engine/voice` id, falling back to whatever
    /// is installed when the id has no prefix (a setting saved before
    /// prefixes existed) or names an engine that is not present.
    ///
    /// Without this the app picked one engine at startup and used it for
    /// every voice, so choosing a Flite voice ran eSpeak with a voice name
    /// eSpeak has never heard of.
    fn for_voice(voice: &str) -> Option<Self> {
        let engine = celeste_core::tts_voices::VoiceId::parse(voice).engine;
        let found = match engine.as_str() {
            "espeak-ng" | "espeak" => find_on_path(&engine).map(Self::Espeak),
            "flite" => find_on_path("flite").map(Self::Flite),
            "pico2wave" => find_on_path("pico2wave").map(Self::Pico),
            "say" => find_on_path("say").map(Self::MacSay),
            "sapi" => find_on_path("pwsh")
                .or_else(|| find_on_path("powershell"))
                .map(Self::PowerShell),
            _ => None,
        };
        found.or_else(Self::detect)
    }

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
            Self::Pico(_) => "Pico",
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
        let prefix = resolve_player_prefix()?;
        let wav = std::env::temp_dir().join(format!(
            "celeste-tts-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // The voice id carries its engine as a prefix; the engine only
        // wants the name.
        let name = celeste_core::tts_voices::VoiceId::parse(voice).voice;
        let name = name.trim();
        // How each engine is asked to render to a file. spd-say is absent
        // on purpose: it hands text to speech-dispatcher, which does its
        // own playback and cannot write a wav for us.
        let mut synth = match self {
            Self::Espeak(engine) => {
                let mut command = Command::new(engine);
                command.arg("-w").arg(&wav);
                if !name.is_empty() {
                    command.args(["-v", name]);
                }
                command.arg(text);
                command
            }
            Self::Flite(engine) => {
                let mut command = Command::new(engine);
                if !name.is_empty() {
                    command.args(["-voice", name]);
                }
                command.args(["-t", text]).arg("-o").arg(&wav);
                command
            }
            Self::Pico(engine) => {
                let mut command = Command::new(engine);
                if !name.is_empty() {
                    command.args(["-l", name]);
                }
                command.arg("-w").arg(&wav).arg(text);
                command
            }
            _ => return None,
        };
        let synthesized = synth
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
        // The stored id is `engine/voice`; command-line synthesizers only
        // understand the voice portion. Keeping the prefix here made macOS
        // and direct Linux playback silently fall back to their defaults.
        let voice = celeste_core::tts_voices::VoiceId::parse(voice).voice;
        let voice = voice.trim();
        let mut command = match self {
            Self::Espeak(path) => {
                let mut command = Command::new(path);
                if !voice.is_empty() {
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
                if !voice.is_empty() {
                    command.args(["-voice", voice]);
                }
                command.args(["-t", text]);
                command
            }
            Self::Pico(path) => {
                // pico2wave cannot play audio itself: it only writes a wav,
                // so without the external player stage there is nothing to
                // hear. `play_via_player` handles it above; reaching here
                // means no player was found.
                let mut command = Command::new(path);
                command.args(["-w", "/dev/null"]).arg(text);
                command
            }
            Self::MacSay(path) => {
                let mut command = Command::new(path);
                if !voice.is_empty() {
                    command.args(["-v", voice]);
                }
                command.arg(text);
                command
            }
            Self::PowerShell(path) => {
                let mut command = Command::new(path);
                // Passing an empty default-voice argument through
                // `powershell -Command` is unreliable: Windows PowerShell
                // can collapse it, shifting the text into `$args[0]` and
                // leaving nothing to speak. Environment values preserve
                // both strings verbatim, including quotes and non-ASCII.
                command
                    .env("CELESTE_TTS_VOICE", voice)
                    .env("CELESTE_TTS_TEXT", text);
                command.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Add-Type -AssemblyName System.Speech; $s=[System.Speech.Synthesis.SpeechSynthesizer]::new(); try { $v=$env:CELESTE_TTS_VOICE; if (![string]::IsNullOrWhiteSpace($v)) { $s.SelectVoice($v) }; $s.Speak($env:CELESTE_TTS_TEXT) } finally { $s.Dispose() }",
                ]);
                // A GUI application must not flash a console window for
                // every announcement or voice preview.
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    command.creation_flags(CREATE_NO_WINDOW);
                }
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
            let parts: Vec<String> = value.split_whitespace().map(str::to_string).collect();
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
    Speak {
        text: String,
        voice: String,
        generation: u64,
    },
    Audio {
        path: PathBuf,
        generation: u64,
    },
    Shutdown,
}

pub struct TtsService {
    sender: mpsc::Sender<Request>,
    paused: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
    active: Arc<Mutex<Option<Child>>>,
    /// Set by [`TtsService::skip`], consumed by the worker.
    ///
    /// Killing the active child alone is not enough: synthesis runs to
    /// completion *before* anything is playing, so a skip pressed while
    /// the engine is still rendering had nothing to kill, returned false,
    /// and the clip then started anyway — pressing Skip and hearing the
    /// line play is exactly the reported symptom.
    skip_requested: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    /// Number of speech/sound requests either queued or currently playing.
    /// The guided tour uses this to wait for an introduction or example to
    /// actually finish before moving to the next part of its demonstration.
    pending: Arc<AtomicU64>,
    engine_name: Option<&'static str>,
    mixed_character_alert: Option<PathBuf>,
    nuisance_alert: Option<PathBuf>,
}

impl TtsService {
    pub fn new() -> Self {
        let engine = Engine::detect();
        let engine_name = engine.as_ref().map(Engine::name);
        let mixed_character_alert =
            extract_alert("mixed-character-alert.wav", MIXED_CHARACTER_ALERT);
        let nuisance_alert = extract_alert("bl_sfx_siren_chirp.wav", NUISANCE_ALERT);
        let (sender, receiver) = mpsc::channel();
        let paused = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let skip_requested = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(AtomicU64::new(0));
        let active: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let worker_shutdown = shutting_down.clone();
        let worker_skip = skip_requested.clone();
        let worker_generation = generation.clone();
        let worker_pending = pending.clone();
        std::thread::Builder::new()
            .name("celeste-tts".into())
            .spawn(move || {
                worker(
                    receiver,
                    engine,
                    worker_active,
                    worker_shutdown,
                    worker_skip,
                    worker_generation,
                    worker_pending,
                )
            })
            .expect("spawn TTS worker");
        if let Some(name) = engine_name {
            tracing::info!(engine = name, "TTS service initialized");
        } else {
            tracing::warn!("No supported TTS engine was found on PATH");
        }
        Self {
            sender,
            skip_requested,
            generation,
            pending,
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
        self.submit_speech(text.into(), voice.into(), true)
    }

    /// Speak accessibility or preview text through the local system output,
    /// even if ordinary moderation callouts are paused. Guided-tour speech
    /// is explicitly opted into and must not enable operational callouts or
    /// route anything through Discord as a side effect.
    pub fn submit_accessibility(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_speech(text.into(), voice.into(), false)
    }

    fn submit_speech(
        &self,
        text: String,
        voice: String,
        respect_pause: bool,
    ) -> Result<(), String> {
        if respect_pause && self.paused.load(Ordering::SeqCst) {
            return Ok(());
        }
        if text.trim().is_empty() {
            return Ok(());
        }
        if self.engine_name.is_none() {
            return Err("No supported TTS engine is installed (tried eSpeak NG/eSpeak, Speech Dispatcher, and Flite).".into());
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self
            .sender
            .send(Request::Speak {
                text,
                voice,
                generation: self.generation.load(Ordering::SeqCst),
            })
            .is_err()
        {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            return Err("The TTS worker stopped.".to_string());
        }
        Ok(())
    }

    fn submit_audio(&self, path: Option<&PathBuf>, respect_pause: bool) -> Result<(), String> {
        if respect_pause && self.paused.load(Ordering::SeqCst) {
            return Ok(());
        }
        let path = path
            .cloned()
            .ok_or_else(|| "The bundled alert sound could not be prepared.".to_string())?;
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self
            .sender
            .send(Request::Audio {
                path,
                generation: self.generation.load(Ordering::SeqCst),
            })
            .is_err()
        {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            return Err("The TTS worker stopped.".to_string());
        }
        Ok(())
    }

    /// Java's `submitMixedCharacterNameJoinAlert`: alert first, then the
    /// localized warning, on the same serial playback queue.
    pub fn submit_mixed_character_alert(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_audio(self.mixed_character_alert.as_ref(), true)?;
        self.submit(text, voice)
    }

    /// Java's `submitNuisanceJoinAlert`: siren chirp first, then the joined
    /// user's combined announcement, without overlapping either clip.
    pub fn submit_nuisance_alert(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_audio(self.nuisance_alert.as_ref(), true)?;
        self.submit(text, voice)
    }

    /// Tour-only version of the nuisance example. It keeps the exact same
    /// serialized alert-then-speech order as a real nuisance notification,
    /// but honours the user's explicit tour-narration choice rather than the
    /// operational Voice pause button. It is always local and never routes
    /// demo audio into Discord.
    pub fn submit_accessibility_nuisance_alert(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_audio(self.nuisance_alert.as_ref(), false)?;
        self.submit_speech(text.into(), voice.into(), false)
    }

    /// Tour-only mixed-script demonstration. This plays the same bundled
    /// warning used by the real local-log check, followed by its explanation,
    /// without enabling ordinary callouts or routing demo audio to Discord.
    pub fn submit_accessibility_mixed_character_alert(
        &self,
        text: impl Into<String>,
        voice: impl Into<String>,
    ) -> Result<(), String> {
        self.submit_audio(self.mixed_character_alert.as_ref(), false)?;
        self.submit_speech(text.into(), voice.into(), false)
    }

    /// True only after every request submitted before this check has either
    /// finished, failed, or been discarded by Skip.
    pub fn is_idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0 && !self.busy()
    }

    /// Skip whatever is speaking or about to speak.
    ///
    /// Returns whether there was anything to skip: either a running clip
    /// (killed) or a synthesis in flight (flagged, so its audio is
    /// discarded instead of played once rendering finishes).
    pub fn skip(&self) -> bool {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let killed = {
            let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
            active.as_mut().is_some_and(|child| child.kill().is_ok())
        };
        // Always raise the flag, even when a clip was killed: the engine
        // may already be rendering the next one.
        self.skip_requested.store(true, Ordering::SeqCst);
        killed || self.busy()
    }

    /// Whether the worker is mid-utterance — rendering or playing.
    fn busy(&self) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

impl Drop for TtsService {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.sender.send(Request::Shutdown);
        let _ = self
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .map(Child::kill);
    }
}

fn worker(
    receiver: mpsc::Receiver<Request>,
    engine: Option<Engine>,
    active: Arc<Mutex<Option<Child>>>,
    shutting_down: Arc<AtomicBool>,
    skip_requested: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    pending: Arc<AtomicU64>,
) {
    struct PendingRequest<'a>(&'a AtomicU64);
    impl Drop for PendingRequest<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    while let Ok(request) = receiver.recv() {
        if shutting_down.load(Ordering::SeqCst) {
            break;
        }
        if matches!(&request, Request::Shutdown) {
            break;
        }
        let _pending_request = PendingRequest(&pending);
        // A skip raised while the previous clip was finishing must not eat
        // this one.
        skip_requested.store(false, Ordering::SeqCst);
        let spawned = match request {
            Request::Speak {
                text,
                voice,
                generation: request_generation,
            } => {
                if request_generation != generation.load(Ordering::SeqCst) {
                    continue;
                }
                // Pick the engine the chosen voice belongs to, not the one
                // detected at startup: with a Flite voice selected and
                // eSpeak also installed, the startup engine would run
                // eSpeak with a voice name it has never heard of.
                let Some(engine) = Engine::for_voice(&voice).or_else(|| engine.clone()) else {
                    continue;
                };
                engine.spawn(&text, &voice)
            }
            Request::Audio {
                path,
                generation: request_generation,
            } => {
                if request_generation != generation.load(Ordering::SeqCst) {
                    continue;
                }
                spawn_audio_file(&path)
            }
            Request::Shutdown => unreachable!("shutdown handled before pending request guard"),
        };
        match spawned {
            Ok(mut child) => {
                // Synthesis blocks, so Skip may have been pressed while
                // this clip was still being rendered and had nothing to
                // kill. Honour it now, before a sound is made.
                if skip_requested.swap(false, Ordering::SeqCst) {
                    let _ = child.kill();
                    let _ = child.wait();
                    continue;
                }
                *active.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
            }
            Err(err) => {
                tracing::warn!("Could not start TTS engine: {err}");
                continue;
            }
        }
        loop {
            if shutting_down.load(Ordering::SeqCst) {
                let _ = active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                    .map(Child::kill);
                break;
            }
            if skip_requested.swap(false, Ordering::SeqCst) {
                let mut guard = active.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(child) = guard.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                guard.take();
                break;
            }
            let finished = {
                let mut guard = active.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut command = Command::new(player);
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$p=[System.Media.SoundPlayer]::new($args[0]); $p.PlaySync()",
            ])
            .arg(path);
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        return command.spawn();
    }
    #[allow(unreachable_code)]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "WAV alert playback is unsupported on this operating system",
    ))
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    celeste_core::tts_voices::find_executable(executable)
        .filter(|candidate| is_executable(candidate))
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
    /// A voice id names its engine, and the speak path must honour it.
    /// Choosing a Flite voice while eSpeak is also installed previously
    /// ran eSpeak with a Flite voice name.
    #[test]
    fn the_engine_follows_the_voice_rather_than_whatever_was_detected() {
        // Only assert on engines actually present, so this is meaningful
        // on a machine with none of them rather than vacuous or flaky.
        if find_on_path("espeak-ng").is_some() {
            assert!(
                matches!(
                    Engine::for_voice("espeak-ng/en-us"),
                    Some(Engine::Espeak(_))
                ),
                "an espeak-ng voice must select eSpeak"
            );
        }
        if find_on_path("flite").is_some() {
            assert!(
                matches!(Engine::for_voice("flite/slt"), Some(Engine::Flite(_))),
                "a flite voice must select Flite"
            );
        }
        // An unprefixed id is a setting saved before prefixes existed, and
        // an unknown engine may simply not be installed; both fall back to
        // whatever is available rather than refusing to speak.
        assert_eq!(
            Engine::for_voice("en-us").is_some(),
            Engine::detect().is_some()
        );
        assert_eq!(
            Engine::for_voice("nosuchengine/x").is_some(),
            Engine::detect().is_some()
        );
    }

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
        assert_eq!(
            resolve_player_prefix(),
            Some(vec!["new-player".to_string()])
        );
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
/// [`celeste_events::TtsSink`], which is how the ported
/// `CelesteEventListener` speaks without depending on this GUI crate.
///
/// The voice is read from settings on every utterance (rather than captured
/// once) so changing it in the Settings tab takes effect immediately, the
/// same way Java's `ttsVoiceName.get()` call inside each announcement does.
///
pub struct TtsSinkAdapter {
    service: std::sync::Arc<TtsService>,
    settings: std::sync::Arc<celeste_core::settings::Settings>,
    discord: Option<celeste_discord::voice_tts::DiscordVoiceTts>,
    ui: Option<crate::UiState>,
}

impl TtsSinkAdapter {
    pub fn new(
        service: std::sync::Arc<TtsService>,
        settings: std::sync::Arc<celeste_core::settings::Settings>,
        discord: Option<celeste_discord::voice_tts::DiscordVoiceTts>,
        ui: Option<crate::UiState>,
    ) -> Self {
        TtsSinkAdapter {
            service,
            settings,
            discord,
            ui,
        }
    }

    /// Skip the current announcement on whichever route it is playing on.
    ///
    /// The toolbar button used to call [`TtsService::skip`] directly,
    /// which only ever touches the local audio player. With the route set
    /// to Discord there is nothing local playing, so Skip did nothing and
    /// the announcement continued. Skipping has to follow the same route
    /// decision the announcement did.
    ///
    /// Returns whether anything was skipped. Discord's queue is
    /// asynchronous, so a skip sent there is reported as sent.
    pub fn skip(&self) -> bool {
        match select_output_route(
            self.settings.tts_use_default_audio_device.get(),
            self.discord.is_some(),
        ) {
            OutputRoute::Local => self.service.skip(),
            OutputRoute::Discord => {
                let discord = self
                    .discord
                    .as_ref()
                    .expect("route checked Discord availability");
                discord.skip();
                // A clip may also be finishing locally from before the
                // route changed; silence that too rather than leaving it.
                self.service.skip();
                true
            }
            // If Discord was selected but cannot currently be used, speech
            // falls back to the local device rather than disappearing.
            OutputRoute::Unavailable => self.service.skip(),
        }
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
                let discord = self
                    .discord
                    .as_ref()
                    .expect("route checked Discord availability");
                discord.submit(text, &voice);
                self.publish_route_status();
            }
            OutputRoute::Unavailable => {
                let problem = "Discord TTS is unavailable, so Celeste used this computer's default audio device instead. Check the bot token and configured voice channel.";
                tracing::warn!("TTS({marker}): {problem}");
                if let Some(ui) = self.ui.as_ref() {
                    ui.set_tts_status(Some(problem.to_string()));
                }
                if let Err(err) = self.service.submit(text, voice) {
                    tracing::debug!("TTS({marker}) fallback: {err}");
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

impl celeste_events::TtsSink for TtsSinkAdapter {
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
        let text = celeste_core::i18n::tr("adv.ttsMixedCharAlert", &[]);
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
