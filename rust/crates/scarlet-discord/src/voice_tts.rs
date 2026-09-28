//! Queues Celeste TTS announcements into Discord voice through Songbird.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serenity::model::id::{ChannelId, GuildId};

#[derive(Clone)]
pub struct DiscordVoiceTts {
    sender: tokio::sync::mpsc::UnboundedSender<(String, String)>,
    /// Why the last announcement didn't reach voice, if it didn't. Read by
    /// the UI so "TTS isn't coming through on Discord" has an answer in the
    /// app rather than only in the log.
    status: Arc<std::sync::Mutex<TtsRouteStatus>>,
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
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
        let status = Arc::new(std::sync::Mutex::new(TtsRouteStatus::default()));
        let worker_status = status.clone();
        runtime.spawn(async move {
            while let Some((text, voice)) = receiver.recv().await {
                // Every one of these used to be a bare `continue`, so a
                // missing voice channel looked exactly like a working
                // config that simply never spoke. Say why, once per
                // distinct reason, and remember it for the status bar.
                let guild_id = config.string("guildSf").and_then(|id| id.parse::<u64>().ok());
                let Some(guild_id) = guild_id else {
                    report(
                        &worker_status,
                        "No Discord server is configured, so TTS has nowhere to speak \
                         (run the setup wizard, or set `guildSf`).",
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
                match synthesize_and_queue(manager.clone(), guild_id, channel_id, text, voice).await
                {
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
        let _ = self.sender.send((text.to_string(), voice.to_string()));
    }
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
    let call = manager
        .join(GuildId::new(guild_id), ChannelId::new(channel_id))
        .await?;
    call.lock()
        .await
        .enqueue_input(songbird::input::File::new(output.clone()).into())
        .await;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10 * 60)).await;
        let _ = tokio::fs::remove_file(output).await;
    });
    Ok(())
}

async fn synthesize_wav(output: &Path, text: &str, voice: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let executable = find_on_path("espeak-ng")
            .or_else(|| find_on_path("espeak"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Discord TTS needs eSpeak NG on Linux and neither `espeak-ng` nor \
                     `espeak` is on PATH (on Arch: `pacman -S espeak-ng`)"
                )
            })?;
        let mut command = tokio::process::Command::new(executable);
        command.arg("-w").arg(output);
        if !voice.trim().is_empty() {
            command
                .arg("-v")
                .arg(voice.rsplit(['/', ':']).next().unwrap_or(voice));
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
            .arg(voice)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "PowerShell TTS exited with {status}");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let executable = find_on_path("say")
            .ok_or_else(|| anyhow::anyhow!("Discord TTS requires `say` on macOS"))?;
        let mut command = tokio::process::Command::new(executable);
        command
            .args(["--data-format=LEI16@22050", "-o"])
            .arg(output);
        if !voice.trim().is_empty() {
            command.args(["-v", voice]);
        }
        let status = command.arg(text).status().await?;
        anyhow::ensure!(status.success(), "say exited with {status}");
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!(
        "Discord TTS is unsupported on this operating system"
    ))
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(executable))
            .find(|candidate| candidate.is_file())
    })
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
            status.lock().unwrap().last_problem.as_deref(),
            Some("no voice channel")
        );
        report(&status, "something else");
        assert_eq!(
            status.lock().unwrap().last_problem.as_deref(),
            Some("something else")
        );
        clear(&status);
        assert!(status.lock().unwrap().last_problem.is_none());
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
}
