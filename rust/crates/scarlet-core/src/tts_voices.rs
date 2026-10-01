//! Discovering the speech voices installed on this machine.
//!
//! Port of `LinuxCommandTtsProvider`'s engine discovery. The Rust side only
//! ever asked eSpeak — and worse, it `return`ed on the first engine that
//! answered, so on a machine with eSpeak NG installed no other synthesizer
//! was ever consulted. Java registers four engines and keeps the voices
//! from all of them.
//!
//! Voice ids are `engine/voice`, as Java's are (`engine.key + "/" +
//! voiceName`). The prefix is not decoration: it is how the speak path
//! knows which binary to run, and how a person choosing a voice can tell
//! that `flite/slt` and `espeak-ng/en-us` come from different synthesizers
//! that sound nothing alike.
//!
//! Discovery runs once per process and is cached — each engine costs a
//! subprocess, and this is read every time the settings list is drawn.

/// A voice id split into the engine that provides it and the name that
/// engine knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceId {
    pub engine: String,
    pub voice: String,
}

impl VoiceId {
    /// Parses `engine/voice`.
    ///
    /// A bare name with no prefix is a voice id saved before engine
    /// prefixes existed. Those must keep working, so it is reported with an
    /// empty engine and [`resolve_legacy`] maps it onto whichever engine
    /// actually has it.
    pub fn parse(id: &str) -> Self {
        match id.split_once('/') {
            Some((engine, voice)) => VoiceId {
                engine: engine.to_string(),
                voice: voice.to_string(),
            },
            None => VoiceId { engine: String::new(), voice: id.to_string() },
        }
    }

    pub fn as_id(&self) -> String {
        if self.engine.is_empty() {
            self.voice.clone()
        } else {
            format!("{}/{}", self.engine, self.voice)
        }
    }
}

/// Maps an un-prefixed voice name onto the first installed voice with that
/// name, so a `tts_voice_name` saved before prefixes existed still selects
/// the voice the user picked rather than silently falling back to a
/// default.
pub fn resolve_legacy(saved: &str, installed: &[String]) -> Option<String> {
    if saved.is_empty() {
        return None;
    }
    if installed.iter().any(|id| id == saved) {
        return Some(saved.to_string());
    }
    installed
        .iter()
        .find(|id| VoiceId::parse(id).voice.eq_ignore_ascii_case(saved))
        .cloned()
}

/// Every installed voice, as `engine/voice`. Cached for the process.
pub fn installed() -> Vec<String> {
    static VOICES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    VOICES.get_or_init(detect).clone()
}

#[cfg(target_os = "linux")]
fn detect() -> Vec<String> {
    let mut voices = Vec::new();
    // Java's registration order (`initEngines`), kept so the first entry --
    // which becomes the default when nothing is chosen -- is the same one
    // Java would have picked.
    push_flite(&mut voices, "flite");
    push_pico(&mut voices);
    push_espeak(&mut voices, "espeak-ng");
    push_espeak(&mut voices, "espeak");
    voices
}

/// `espeak --voices`, second column, exactly as Java's
/// `awk 'NR>1 {print $2}'` does.
#[cfg(target_os = "linux")]
fn push_espeak(out: &mut Vec<String>, command: &str) {
    let Some(lines) = run(command, &["--voices"]) else { return };
    let mut names: Vec<String> = lines
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|token| is_simple_voice_token(token))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    // Java falls back to a single "default" voice when an engine is present
    // but lists none, rather than dropping the engine entirely.
    if names.is_empty() {
        names.push("default".to_string());
    }
    add(out, command, &names);
}

/// `flite -lv`, which prints a prose line with the names embedded, so Java
/// splits on whitespace and colons and filters the prose words out.
#[cfg(target_os = "linux")]
fn push_flite(out: &mut Vec<String>, command: &str) {
    let Some(text) = run(command, &["-lv"]) else { return };
    const NOISE: [&str; 5] = ["voices", "available", "voice", "list", "internal"];
    let mut names: Vec<String> = text
        .replace(':', " ")
        .split_whitespace()
        .filter(|token| !NOISE.contains(&token.to_ascii_lowercase().as_str()))
        .filter(|token| is_simple_voice_token(token))
        .map(str::to_string)
        .collect();
    names.dedup();
    if names.is_empty() {
        names.push("default".to_string());
    }
    add(out, command, &names);
}

/// Pico ships a fixed set and has no way to enumerate them, so Java hard-codes
/// the list (`registerPico`).
#[cfg(target_os = "linux")]
fn push_pico(out: &mut Vec<String>) {
    if which("pico2wave").is_none() {
        return;
    }
    let names: Vec<String> = ["en-US", "en-GB", "de-DE", "es-ES", "fr-FR", "it-IT"]
        .iter()
        .map(|name| name.to_string())
        .collect();
    add(out, "pico2wave", &names);
}

#[cfg(target_os = "macos")]
fn detect() -> Vec<String> {
    let Some(text) = run("say", &["-v", "?"]) else { return Vec::new() };
    let names: Vec<String> = text
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    let mut out = Vec::new();
    add(&mut out, "say", &names);
    out
}

#[cfg(target_os = "windows")]
fn detect() -> Vec<String> {
    const SCRIPT: &str = "Add-Type -AssemblyName System.Speech; \
         [System.Speech.Synthesis.SpeechSynthesizer]::new().GetInstalledVoices() | \
         ForEach-Object { $_.VoiceInfo.Name }";
    for shell in ["pwsh", "powershell"] {
        if let Some(text) = run(shell, &["-NoProfile", "-NonInteractive", "-Command", SCRIPT]) {
            let names: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            if !names.is_empty() {
                let mut out = Vec::new();
                add(&mut out, "sapi", &names);
                return out;
            }
        }
    }
    Vec::new()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect() -> Vec<String> {
    Vec::new()
}

/// Appends `engine/name` for each name, skipping duplicates — Java's
/// `addVoices`, which keys on the full id so two engines may both offer a
/// voice called `default`.
fn add(out: &mut Vec<String>, engine: &str, names: &[String]) {
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let id = format!("{engine}/{name}");
        if !out.contains(&id) {
            out.push(id);
        }
    }
}

/// Java's `isSimpleVoiceToken`: a bare identifier, which filters out the
/// prose and punctuation that `--voices` and `-lv` print around the names.
fn is_simple_voice_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '@' | '-'))
}

#[allow(dead_code)]
fn run(command: &str, args: &[&str]) -> Option<String> {
    which(command)?;
    let output = std::process::Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[allow(dead_code)]
fn which(command: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_id_round_trips_through_its_engine_and_name() {
        let id = VoiceId::parse("espeak-ng/en-us");
        assert_eq!(id.engine, "espeak-ng");
        assert_eq!(id.voice, "en-us");
        assert_eq!(id.as_id(), "espeak-ng/en-us");
    }

    /// A voice saved before engine prefixes existed must keep selecting the
    /// voice the user picked, not silently fall back to a default.
    #[test]
    fn a_legacy_unprefixed_voice_still_resolves() {
        let installed = vec![
            "flite/slt".to_string(),
            "espeak-ng/en-us".to_string(),
            "espeak/en-us".to_string(),
        ];
        assert_eq!(
            resolve_legacy("en-us", &installed).as_deref(),
            Some("espeak-ng/en-us"),
            "picks the first engine offering that name"
        );
        assert_eq!(
            resolve_legacy("espeak/en-us", &installed).as_deref(),
            Some("espeak/en-us"),
            "an already-prefixed id is kept as-is"
        );
        assert_eq!(resolve_legacy("nope", &installed), None);
        assert_eq!(resolve_legacy("", &installed), None);
    }

    /// The bug: the old enumerator returned on the first engine that
    /// answered, so only one synthesizer's voices were ever offered.
    #[test]
    fn voices_from_every_engine_are_kept() {
        let mut out = Vec::new();
        add(&mut out, "flite", &["slt".to_string(), "kal".to_string()]);
        add(&mut out, "espeak-ng", &["en-us".to_string()]);
        add(&mut out, "pico2wave", &["en-US".to_string()]);
        assert_eq!(out.len(), 4);
        assert!(out.contains(&"flite/slt".to_string()));
        assert!(out.contains(&"espeak-ng/en-us".to_string()));
        assert!(out.contains(&"pico2wave/en-US".to_string()));
    }

    /// Two engines may each offer a voice called the same thing; keying on
    /// the full id keeps both, as Java does.
    #[test]
    fn the_same_name_from_two_engines_is_two_voices() {
        let mut out = Vec::new();
        add(&mut out, "espeak-ng", &["default".to_string()]);
        add(&mut out, "flite", &["default".to_string()]);
        add(&mut out, "flite", &["default".to_string()]);
        assert_eq!(out, vec!["espeak-ng/default", "flite/default"]);
    }

    #[test]
    fn prose_and_punctuation_are_not_voice_names() {
        assert!(is_simple_voice_token("en-us"));
        assert!(is_simple_voice_token("cmu_us_slt"));
        assert!(!is_simple_voice_token("(default)"));
        assert!(!is_simple_voice_token(""));
    }
}
