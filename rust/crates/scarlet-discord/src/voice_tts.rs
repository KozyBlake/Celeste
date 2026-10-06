//! Queues Celeste TTS announcements into Discord voice through Songbird.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serenity::model::id::{ChannelId, GuildId};
use songbird::{Event, EventContext, EventHandler, TrackEvent};

#[derive(Clone)]
pub struct DiscordVoiceTts {
    sender: tokio::sync::mpsc::UnboundedSender<VoiceRequest>,
    /// Why the last announcement didn't reach voice, if it didn't. Read by
    /// the UI so "TTS isn't coming through on Discord" has an answer in the
    /// app rather than only in the log.
    status: Arc<std::sync::Mutex<TtsRouteStatus>>,
}

enum VoiceRequest {
    Speak { text: String, voice: String },
    Audio { bytes: &'static [u8], name: &'static str },
    /// Drop whatever is speaking in Discord voice and move to the next
    /// queued clip.
    ///
    /// The desktop Skip button only ever killed the *local* audio player.
    /// With `tts_use_default_audio_device` off, announcements are rendered
    /// into Discord instead and nothing local is playing — so Skip killed
    /// nothing, reported nothing, and the announcement carried on. That is
    /// the reported "it doesn't skip, it just continues".
    Skip,
}

#[derive(Default)]
struct TtsRouteStatus {
    last_problem: Option<String>,
}

/// Logs `problem` the first time it is seen and whenever it changes, so a
/// misconfiguration is loud once instead of once per announcement.
fn report(status: &Arc<std::sync::Mutex<TtsRouteStatus>>, problem: &str) {
    let mut status = match status.lock() {
        Ok(status) => status,
        Err(poisoned) => poisoned.into_inner(),
    };
    if status.last_problem.as_deref() != Some(problem) {
        tracing::warn!("{problem}");
        status.last_problem = Some(problem.to_string());
    }
}

fn clear(status: &Arc<std::sync::Mutex<TtsRouteStatus>>) {
    let mut status = match status.lock() {
        Ok(status) => status,
        Err(poisoned) => poisoned.into_inner(),
    };
    if status.last_problem.take().is_some() {
        tracing::info!("Discord TTS is working again");
    }
}

impl DiscordVoiceTts {
    pub fn new(
        manager: Arc<songbird::Songbird>,
        config: Arc<crate::data::DiscordConfigStore>,
        settings: Arc<scarlet_core::settings::Settings>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<VoiceRequest>();
        let status = Arc::new(std::sync::Mutex::new(TtsRouteStatus::default()));
        let worker_status = status.clone();
        runtime.spawn(async move {
            while let Some(request) = receiver.recv().await {
                // Every one of these used to be a bare `continue`, so a
                // missing voice channel looked exactly like a working
                // config that simply never spoke. Say why, once per
                // distinct reason, and remember it for the status bar.
                // Skip needs the call, not the configured channel, and
                // must not be dropped by the config checks below.
                if matches!(request, VoiceRequest::Skip) {
                    if let Some(guild_id) = crate::data::configured_guild_id(&settings, &config) {
                        if let Some(call) = manager.get(GuildId::new(guild_id)) {
                            let _ = call.lock().await.queue().skip();
                        }
                    }
                    continue;
                }
                let guild_id = crate::data::configured_guild_id(&settings, &config);
                let Some(guild_id) = guild_id else {
                    report(
                        &worker_status,
                        "No Discord server is configured, so TTS has nowhere to speak \
                         (run the setup wizard to choose one).",
                    );
                    continue;
                };
                let channel_id =
                    config.string("audioChannelSf").and_then(|id| id.parse::<u64>().ok());
                let Some(channel_id) = channel_id else {
                    report(
                        &worker_status,
                        "No Discord voice channel is set, so TTS announcements are being \
                         dropped - use `/set-voice-channel` to pick one.",
                    );
                    continue;
                };
                let result = match request {
                    VoiceRequest::Skip => unreachable!("handled above"),
                    VoiceRequest::Speak { text, voice } => {
                        synthesize_and_queue(manager.clone(), guild_id, channel_id, text, voice).await
                    }
                    VoiceRequest::Audio { bytes, name } => {
                        queue_bundled_audio(manager.clone(), guild_id, channel_id, bytes, name).await
                    }
                };
                match result {
                    Ok(()) => clear(&worker_status),
                    Err(error) => {
                        report(&worker_status, &format!("Discord TTS failed: {error}"));
                    }
                }
            }
        });
        Self { sender, status }
    }

    /// The reason the last announcement didn't reach Discord voice, if any.
    pub fn last_problem(&self) -> Option<String> {
        match self.status.lock() {
            Ok(status) => status.last_problem.clone(),
            Err(poisoned) => poisoned.into_inner().last_problem.clone(),
        }
    }

    pub fn submit(&self, text: &str, voice: &str) {
        if text.trim().is_empty() {
            return;
        }
        let _ = self.sender.send(VoiceRequest::Speak {
            text: text.to_string(),
            voice: voice.to_string(),
        });
    }

    /// Stop the clip currently playing in Discord voice; the queue
    /// carries on with the next one.
    pub fn skip(&self) {
        let _ = self.sender.send(VoiceRequest::Skip);
    }

    /// Queue a bundled WAV before a spoken warning. The worker and
    /// Songbird queue preserve submission order, matching Java's single
    /// playback executor.
    pub fn submit_audio(&self, bytes: &'static [u8], name: &'static str) {
        let _ = self.sender.send(VoiceRequest::Audio { bytes, name });
    }
}

async fn queue_bundled_audio(
    manager: Arc<songbird::Songbird>,
    guild_id: u64,
    channel_id: u64,
    bytes: &'static [u8],
    name: &'static str,
) -> anyhow::Result<()> {
    anyhow::ensure!(bytes.starts_with(b"RIFF") && bytes.len() > 44, "bundled alert is not a valid WAV");
    let output = std::env::temp_dir().join(format!(
        "celeste-{name}-{}.wav",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::write(&output, bytes).await?;
    queue_audio_file(manager, guild_id, channel_id, output).await
}

async fn synthesize_and_queue(
    manager: Arc<songbird::Songbird>,
    guild_id: u64,
    channel_id: u64,
    text: String,
    voice: String,
) -> anyhow::Result<()> {
    let output = std::env::temp_dir().join(format!("celeste-tts-{}.wav", uuid::Uuid::new_v4()));
    synthesize_wav(&output, &text, &voice).await?;
    // A synthesizer that "succeeds" but writes nothing produces silence in
    // voice, which is indistinguishable from TTS being broken. Catch it
    // here where we can still say what happened.
    match tokio::fs::metadata(&output).await {
        Ok(meta) if meta.len() > 0 => {}
        Ok(_) => {
            let _ = tokio::fs::remove_file(&output).await;
            anyhow::bail!("the speech synthesizer produced an empty audio file");
        }
        Err(error) => anyhow::bail!("the speech synthesizer wrote no audio file ({error})"),
    }
    queue_audio_file(manager, guild_id, channel_id, output).await
}

async fn queue_audio_file(
    manager: Arc<songbird::Songbird>,
    guild_id: u64,
    channel_id: u64,
    output: PathBuf,
) -> anyhow::Result<()> {
    let call = manager
        .join(GuildId::new(guild_id), ChannelId::new(channel_id))
        .await?;
    let track = call
        .lock()
        .await
        .enqueue_input(songbird::input::File::new(output.clone()).into())
        .await;

    // `File` is opened lazily by Songbird. Deleting it immediately after
    // enqueueing recreates the exact race fixed in Java's TtsService: a
    // queued track can reach the decoder after its WAV has disappeared.
    // A fixed timeout is not safe either when several announcements are
    // queued. Keep the file until Songbird says this particular track has
    // ended (or failed), then remove it.
    let cleanup = DeleteAudioFile::new(output);
    track.add_event(Event::Track(TrackEvent::End), cleanup.clone())?;
    if let Err(error) = track.add_event(Event::Track(TrackEvent::Error), cleanup) {
        // End cleanup is already installed, so an inability to add the
        // extra error hook must not turn a successfully queued utterance
        // into a reported playback failure.
        tracing::debug!(%error, "Could not attach Discord TTS error cleanup hook");
    }
    tracing::info!(guild_id, channel_id, "Queued Discord TTS audio");
    Ok(())
}

#[derive(Clone)]
struct DeleteAudioFile {
    path: Arc<PathBuf>,
    removed: Arc<std::sync::atomic::AtomicBool>,
}

impl DeleteAudioFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(path),
            removed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    async fn remove(&self) {
        if !self
            .removed
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            if let Err(error) = tokio::fs::remove_file(self.path.as_ref()).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        %error,
                        path = %self.path.display(),
                        "Could not remove completed Discord TTS audio file"
                    );
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl EventHandler for DeleteAudioFile {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        self.remove().await;
        None
    }
}

async fn synthesize_wav(output: &Path, text: &str, voice: &str) -> anyhow::Result<()> {
    // One renderer, shared with the desktop path. These two had drifted:
    // this one was eSpeak-only and simply stripped the engine prefix off
    // the voice id, so a Flite voice became `espeak -v slt`, which fails
    // and writes nothing - the "wrote no audio file" error in the log.
    if let Some((program, args)) = scarlet_core::tts_voices::render_command(voice, text, output) {
        let status = tokio::process::Command::new(&program).args(&args).status().await?;
        anyhow::ensure!(status.success(), "{} exited with {status}", program.display());
        return Ok(());
    }

    // The voice named no installed engine (an id saved before engine
    // prefixes existed, or an engine since uninstalled). Fall back to
    // whatever this machine does have, rather than dropping the
    // announcement.
    #[cfg(target_os = "linux")]
    {
        let executable = find_on_path("espeak-ng")
            .or_else(|| find_on_path("espeak"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Discord TTS needs a speech synthesizer on Linux and none was found on PATH \
                     (eSpeak NG, Flite or Pico; on Arch: `pacman -S espeak-ng`)"
                )
            })?;
        let mut command = tokio::process::Command::new(executable);
        command.arg("-w").arg(output);
        let name = scarlet_core::tts_voices::VoiceId::parse(voice).voice;
        if !name.trim().is_empty() {
            command.arg("-v").arg(name.trim());
        }
        let status = command.arg(text).status().await?;
        anyhow::ensure!(status.success(), "eSpeak exited with {status}");
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let executable = find_on_path("pwsh")
            .or_else(|| find_on_path("powershell"))
            .ok_or_else(|| anyhow::anyhow!("Discord TTS requires PowerShell on Windows"))?;
        let script = "Add-Type -AssemblyName System.Speech; $s=[System.Speech.Synthesis.SpeechSynthesizer]::new(); if($args[2]){$s.SelectVoice($args[2])}; $s.SetOutputToWaveFile($args[0]); $s.Speak($args[1]); $s.Dispose()";
        let status = tokio::process::Command::new(executable)
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .arg(output)
            .arg(text)
            .arg(scarlet_core::tts_voices::VoiceId::parse(voice).voice)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "PowerShell TTS exited with {status}");
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!(
        "Discord TTS is unsupported on this operating system"
    ))
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    scarlet_core::tts_voices::find_executable(executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A misconfiguration should be loud once, not once per announcement,
    /// and should clear when it starts working.
    #[test]
    fn a_repeated_problem_is_only_recorded_once_and_clears_on_success() {
        let status = Arc::new(std::sync::Mutex::new(TtsRouteStatus::default()));
        report(&status, "no voice channel");
        report(&status, "no voice channel");
        assert_eq!(
            status.lock().unwrap_or_else(|e| e.into_inner()).last_problem.as_deref(),
            Some("no voice channel")
        );
        report(&status, "something else");
        assert_eq!(
            status.lock().unwrap_or_else(|e| e.into_inner()).last_problem.as_deref(),
            Some("something else")
        );
        clear(&status);
        assert!(status.lock().unwrap_or_else(|e| e.into_inner()).last_problem.is_none());
    }

    #[test]
    fn path_lookup_finds_a_known_shell() {
        if cfg!(unix) {
            assert!(find_on_path("sh").is_some());
        }
    }

    #[tokio::test]
    async fn installed_engine_produces_a_wave_file() {
        if find_on_path("espeak-ng")
            .or_else(|| find_on_path("espeak"))
            .is_none()
        {
            return;
        }
        let path =
            std::env::temp_dir().join(format!("scarlet-tts-test-{}.wav", uuid::Uuid::new_v4()));
        synthesize_wav(&path, "Celeste voice test", "")
            .await
            .unwrap();
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert!(bytes.len() > 44);
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn completed_track_cleanup_removes_the_wave_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.wav");
        tokio::fs::write(&path, b"RIFF test data").await.unwrap();
        let cleanup = DeleteAudioFile::new(path.clone());

        cleanup.clone().remove().await;
        cleanup.remove().await;

        assert!(!path.exists());
    }
}
