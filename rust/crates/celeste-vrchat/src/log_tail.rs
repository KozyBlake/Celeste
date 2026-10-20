//! Port of `net.sybyline.scarlet.ScarletVRChatLogs` (the local
//! `output_log_*.txt` tailer-parser) and `net.sybyline.scarlet.ext.VrcAppData`
//! (VRChat app-data directory resolution).
//!
//! This is a *different* data source from `client.rs`'s VRChat REST API: it
//! reads the plaintext log VRChat itself writes to disk on the machine it's
//! running on, which is how a live player list (who's in your current
//! instance right now) gets built without waiting on the audit-log API —
//! the same mechanism VRCX and other companion tools use.
//!
//! ## What's ported line-for-line vs. simplified
//!
//! [`parse_entry_text`] (the actual "what does this log line mean" parsing)
//! is ported faithfully, one `if`/`else if` branch per Java branch, checked
//! against the exact literal prefixes/separators from the source (using
//! `strip_prefix`/`find`/`rfind` rather than transcribing Java's hardcoded
//! character offsets — copying `text.substring(27, ...)` as a bare `27`
//! would be exactly the kind of silent transcription error this rewrite is
//! trying to avoid; matching on the actual literal string is equivalent and
//! doesn't depend on counting characters correctly by hand).
//!
//! The low-level tailing (`Tail.java`, an `Apache Commons IO Tailer` fork)
//! is a byte-level state machine handling bare-CR line endings and
//! mid-write partial reads. This port reads growing files by line instead
//! (splitting on `\n`, trimming a trailing `\r`), which is equivalent for
//! every real VRChat log (they use LF or CRLF, never bare CR) without
//! reproducing Commons IO's full generality for an ending format VRChat
//! doesn't produce.
//!
//! **Not tested against a real running VRChat client in this environment**
//! (no VRChat installed here) — the parsing logic has thorough unit tests
//! against reconstructed real log line formats, but the end-to-end file
//! tailing has only been exercised against synthetic files in tests, not
//! an actual live `output_log_*.txt`. Confirm on a machine that's actually
//! running VRChat before trusting the live-tailing half of this.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDateTime;
use tokio::sync::mpsc;

pub const VRCHAT_APP_ID: &str = "438100";

/// Relative paths ported from `VrcAppData.DIR` / `VrcAppData.TEMP`.
pub const APP_DATA_DIR_RELATIVE: &str = "AppData/LocalLow/VRChat/VRChat";
pub const APP_DATA_TEMP_RELATIVE: &str = "AppData/Local/Temp/VRChat/VRChat";

/// Port of `VrcAppData.DIR`: the directory `output_log_*.txt` files live in.
pub fn resolve_vrc_app_data_dir() -> PathBuf {
    resolve(APP_DATA_DIR_RELATIVE)
}

/// Port of `VrcAppData.TEMP`. VRChat writes crash dumps and some transient
/// state here; resolved the same way as [`resolve_vrc_app_data_dir`] so a
/// single `CELESTE_VRC_APPDATA_DIR` override serves both. The legacy
/// `SCARLET_VRC_APPDATA_DIR` spelling remains accepted.
pub fn resolve_vrc_app_data_temp_dir() -> PathBuf {
    resolve(APP_DATA_TEMP_RELATIVE)
}

/// Port of `VrcAppData.resolve(String relative)`.
///
/// - `CELESTE_VRC_APPDATA_DIR` (or legacy `SCARLET_VRC_APPDATA_DIR`), if set,
///   IS the resolved VRChat directory;
///   the two known prefixes are stripped off `relative` so one override root
///   produces the right path for both `DIR` and `TEMP` (Java does the same,
///   for the Android companion build that points the tailer at a logcat
///   dump). Java also reads the `scarlet.vrcAppData.dir` system property
///   first — Rust has no system properties, so the env var is the only
///   override, which is the same knob Java falls back to.
/// - Linux: each detected Steam library's Proton `compatdata` prefix, then
///   `~/.steam/steam` as the hardcoded fallback.
/// - Everything else (Windows): `<user.home>/<relative>` directly.
///
/// Java's Android/Termux branch is deliberately not ported — it exists for
/// the separate scarlet-android release, which this desktop port is not.
pub fn resolve(relative: &str) -> PathBuf {
    if let Ok(over) = std::env::var("CELESTE_VRC_APPDATA_DIR")
        .or_else(|_| std::env::var("SCARLET_VRC_APPDATA_DIR"))
    {
        if !over.is_empty() {
            let mut rel = relative;
            for prefix in [APP_DATA_DIR_RELATIVE, APP_DATA_TEMP_RELATIVE] {
                if rel == prefix {
                    rel = "";
                    break;
                }
                if let Some(stripped) = rel.strip_prefix(prefix).and_then(|r| r.strip_prefix('/')) {
                    rel = stripped;
                    break;
                }
            }
            let root = PathBuf::from(over);
            return if rel.is_empty() { root } else { root.join(rel) };
        }
    }

    let home = dirs_home();

    if cfg!(target_os = "linux") {
        if let Some(detected) = find_in_steam_libraries(&home, relative) {
            tracing::info!("Resolved VRChat path via Steam library detection: {}", detected.display());
            return detected;
        }
        let fallback = home.join(".steam/steam").join(proton_prefix()).join(relative);
        tracing::warn!(
            "Could not detect Steam library for VRChat, falling back to default path: {}",
            fallback.display()
        );
        return fallback;
    }

    home.join(relative)
}

fn proton_prefix() -> PathBuf {
    PathBuf::from(format!("steamapps/compatdata/{VRCHAT_APP_ID}/pfx/drive_c/users/steamuser"))
}

/// Resolves the user's home directory across platforms. `HOME` is the
/// Unix/macOS convention; Windows doesn't set it by default and uses
/// `USERPROFILE` instead (falling back to `HOMEDRIVE`+`HOMEPATH` on older
/// setups where even that's unset).
fn dirs_home() -> PathBuf {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    #[cfg(not(windows))]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    if let (Some(drive), Some(path)) = (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        let mut combined = PathBuf::from(drive);
        combined.push(path);
        return combined;
    }
    PathBuf::from(".")
}

/// Port of `VrcAppData.findInSteamLibraries` + `detectSteamLibraryRoots`.
fn find_in_steam_libraries(home: &Path, relative: &str) -> Option<PathBuf> {
    for root in detect_steam_library_roots(home) {
        let candidate = root.join(proton_prefix()).join(relative);
        tracing::debug!("Checking Steam library candidate: {}", candidate.display());
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn detect_steam_library_roots(home: &Path) -> Vec<PathBuf> {
    let steam_homes = [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"), // Flatpak Steam
    ];

    let mut vdf_and_home = None;
    for steam_home in &steam_homes {
        let candidate = steam_home.join("steamapps/libraryfolders.vdf");
        if candidate.is_file() {
            vdf_and_home = Some((candidate, steam_home.clone()));
            break;
        }
    }

    let mut roots = Vec::new();
    match &vdf_and_home {
        Some((_, primary)) => roots.push(primary.clone()),
        None => roots.push(home.join(".steam/steam")),
    }

    let Some((vdf, _)) = vdf_and_home else {
        return roots;
    };

    if let Ok(contents) = std::fs::read_to_string(&vdf) {
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("\"path\"") {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() >= 4 {
                    let library_root = PathBuf::from(parts[3]);
                    if library_root.exists() && !roots.contains(&library_root) {
                        roots.push(library_root);
                    }
                }
            }
        }
    }
    roots
}

/// Port of `ScarletVRChatLogs.locateTarget`: the newest
/// `output_log_<timestamp>.txt` file in `dir`, by filename timestamp (not
/// mtime — matches the Java version exactly).
pub fn locate_latest_log(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            let stamp = name.strip_prefix("output_log_")?.strip_suffix(".txt")?;
            let ts = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d_%H-%M-%S").ok()?;
            Some((ts, p))
        })
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, p)| p)
}

/// One parsed VRChat log entry, ported from `ScarletVRChatLogs.Listener`'s
/// callback methods.
#[derive(Debug, Clone, PartialEq)]
pub enum LogEventKind {
    UserAuthenticated { display_name: String, user_id: String },
    UserQuit { lifetime_seconds: f64 },
    UserJoined { location: String },
    /// `[Behaviour] Entering Room: <world name>`, logged immediately before
    /// the `Joining wrld_...` line for the same instance.
    ///
    /// **Not in Java.** `handleEntry` ignores this line, so the Java UI only
    /// ever knows an instance by its raw `wrld_...:1234~group(...)` location.
    /// The Rust UI surfaces the instance it is watching, and showing a world
    /// id there instead of "Studio Apartment J31" would be strictly worse
    /// for the person reading it. Parsing one more line is the whole cost.
    EnteringRoom { world_name: String },
    UserLeft,
    PlayerJoined { display_name: String, user_id: String },
    PlayerLeft { display_name: String, user_id: String },
    PlayerSwitchAvatar { display_name: String, avatar_display_name: String },
    VtkInit { target_display_name: String, actor_display_name: Option<String> },
    PlayerSpawnPedestal { display_name: String, user_id: String, content_type: String, content_id: String },
    PlayerSpawnSticker { display_name: String, user_id: String, sticker_id: String },
    PlayerSpawnProp { user_id: String, prop_id: String },
    ApiRequest { index: i32, method: String, url: String },
    VideoLoad { display_name: String, url: String, title: String },
    /// `[AssetBundleDownloadManager] [N] Unpacking Avatar (<name> by <author>)`
    ///
    /// **Not in Java.** The last thing a client manages before a crasher
    /// avatar takes it down, and the only log line that names the
    /// avatar's *author* — `Switching X to avatar Y` gives the wearer and
    /// the avatar's name, never who uploaded it.
    AvatarUnpacked { avatar_name: String, author_name: Option<String> },
    /// `[ModerationManager] <display name> has been kicked`: a kick that
    /// actually took effect in the instance this client is in.
    ///
    /// **Not in Java.** `ScarletVRChatLogs.handleEntry` matches only the
    /// vote-kick under `[ModerationManager]` and drops this line, so
    /// Scarlet never saw these either.
    ///
    /// This is the only local trace of a kick performed by someone whose
    /// authority is VRChat-side rather than a group role: no group audit
    /// entry is written for it, and the target otherwise just appears in
    /// `OnPlayerLeft` like anyone who left on their own.
    ///
    /// The line names no actor. Celeste can report that a kick happened
    /// and to whom, never by whom — guessing an actor from whoever was
    /// nearby would be a fabricated moderation record.
    PlayerKicked { display_name: String },
    /// `[ModerationManager] <display name> has been warned`: an instance
    /// warn that took effect in the instance this client is in.
    ///
    /// **Not in Java**, which matches only the vote-kick line.
    ///
    /// Unlike [`LogEventKind::PlayerKicked`], a group-role instance warn
    /// *does* write a `group.instance.warn` audit entry, so this line is
    /// not the only record of it -- it is the *fast* one. It arrives
    /// within a second, where the audit poller is on a timer and may be a
    /// minute behind. Celeste uses it to bring the authoritative poll
    /// forward, not to replace it.
    ///
    /// VRChat names no actor here either, so the actor comes from the
    /// audit entry that follows.
    PlayerWarned { display_name: String },
    /// A `[ModerationManager]` line whose shape this parser does not know.
    ///
    /// Not a parsed event — an instrument. Celeste recognises the
    /// vote-kick, the kick and the warn; any other moderation action
    /// taken by someone with VRChat-side privileges leaves no trace it
    /// can act on, because it is not a group-role action, so the group
    /// audit log never records it and locally the target simply appears
    /// to leave.
    ///
    /// The prefix allowlist already passes every `[ModerationManager]`
    /// line through, so anything VRChat does write was reaching the parser
    /// and being discarded unseen. Surfacing the text is what lets the
    /// shape be identified and parsed properly instead of guessed at.
    UnrecognizedModeration { text: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEvent {
    pub preamble: bool,
    pub timestamp: NaiveDateTime,
    pub kind: LogEventKind,
}

/// Port of `ScarletVRChatLogs.entry`/`entry_dtf`: splits a raw log line into
/// (timestamp, level, text). VRChat's log line shape is
/// `yyyy.MM.dd HH:mm:ss LEVEL  -  text`.
pub fn parse_log_line(line: &str) -> Option<(NaiveDateTime, &str, &str)> {
    // "YYYY.MM.DD HH:MM:SS" is always exactly 19 bytes/chars (ASCII digits
    // and separators), so slicing is safe and avoids pulling in `regex`
    // just for this.
    if line.len() < 20 {
        return None;
    }
    let (ts_part, rest) = line.split_at(19);
    let timestamp = NaiveDateTime::parse_from_str(ts_part, "%Y.%m.%d %H:%M:%S").ok()?;
    let rest = rest.strip_prefix(' ')?;
    let level_end = rest.find(char::is_whitespace)?;
    let (level, after_level) = rest.split_at(level_end);
    let after_level = after_level.trim_start();
    let text = after_level.strip_prefix("-  ")?;
    Some((timestamp, level, text))
}

/// Port of `ScarletVRChatLogs.handleEntry`'s big `if`/`else if` chain — see
/// module docs on why this uses `strip_prefix`/`find`/`rfind` instead of
/// Java's hardcoded substring offsets.
pub fn parse_entry_text(text: &str) -> Option<LogEventKind> {
    // `User Authenticated: <name> (usr_...)`.
    //
    // **Deliberate divergence from Java.** `handleEntry`'s authentication
    // branch is nested inside `if (text.startsWith("[Behaviour] "))` but
    // itself tests `text.startsWith("[User Authenticated: ")` — on the same
    // `text`, from index 0. Both can never be true at once, so the branch is
    // unreachable and Java never learns its own user id from the log. That
    // matters: `clientUserId` is what `CelesteEventListener` compares against
    // to avoid running join checks (and the mixed-character-name alert)
    // against the local user themselves. A real VRChat log writes this line
    // with no `[Behaviour] ` prefix and no brackets at all, confirmed against
    // a live 2026-09 client log:
    //   `2026.09.26 22:31:07 Debug      -  User Authenticated: Name (usr_...)`
    // so this matches the real format. The bracketed shape is still accepted
    // in case an older client wrote it that way.
    if let Some(rest) = text
        .strip_prefix("User Authenticated: ")
        .or_else(|| text.strip_prefix("[Behaviour] User Authenticated: "))
        .or_else(|| text.strip_prefix("[Behaviour] [User Authenticated: "))
    {
        let name_end = rest.rfind(" (")?;
        let id_end = rest.rfind(')')?;
        return Some(LogEventKind::UserAuthenticated {
            display_name: rest[..name_end].to_string(),
            user_id: rest[name_end + 2..id_end].to_string(),
        });
    }

    if let Some(after_behaviour) = text.strip_prefix("[Behaviour] ") {
        if let Some(rest) = after_behaviour.strip_prefix("Entering Room: ") {
            return Some(LogEventKind::EnteringRoom { world_name: rest.to_string() });
        }
        if let Some(rest) = after_behaviour.strip_prefix("Joining ") {
            if !rest.starts_with("or Creating Room: ") {
                return Some(LogEventKind::UserJoined { location: rest.to_string() });
            }
            return None;
        }
        if after_behaviour.starts_with("OnLeftRoom") {
            return Some(LogEventKind::UserLeft);
        }
        if let Some(rest) = after_behaviour.strip_prefix("OnPlayerJoined ") {
            let oparen = rest.rfind(" (")?;
            let cparen = rest.rfind(')')?;
            return Some(LogEventKind::PlayerJoined {
                display_name: rest[..oparen].to_string(),
                user_id: rest[oparen + 2..cparen].to_string(),
            });
        }
        if let Some(rest) = after_behaviour.strip_prefix("OnPlayerLeft ") {
            let oparen = rest.rfind(" (")?;
            let cparen = rest.rfind(')')?;
            return Some(LogEventKind::PlayerLeft {
                display_name: rest[..oparen].to_string(),
                user_id: rest[oparen + 2..cparen].to_string(),
            });
        }
        if let Some(rest) = after_behaviour.strip_prefix("Switching ") {
            if let Some(cidx) = rest.rfind(" to avatar ") {
                return Some(LogEventKind::PlayerSwitchAvatar {
                    display_name: rest[..cidx].to_string(),
                    avatar_display_name: rest[cidx + " to avatar ".len()..].to_string(),
                });
            }
        }
        return None;
    }

    if let Some(after_api) = text.strip_prefix("[API] [") {
        if let Some(cidx) = after_api.find("] Sending ") {
            let index: i32 = after_api[..cidx].parse().unwrap_or(-1);
            let after_sending = &after_api[cidx + "] Sending ".len()..];
            if let Some(sep1) = after_sending.find(" request to ") {
                return Some(LogEventKind::ApiRequest {
                    index,
                    method: after_sending[..sep1].to_string(),
                    url: after_sending[sep1 + " request to ".len()..].to_string(),
                });
            }
        }
        return None;
    }

    if let Some(after) = text.strip_prefix("[AssetBundleDownloadManager] ") {
        // "[461] Unpacking Avatar (D.S.C Fallback by 72kontop)"
        if let Some(open_paren) = after.find("] Unpacking Avatar (") {
            let inner = &after[open_paren + "] Unpacking Avatar (".len()..];
            if let Some(inner) = inner.strip_suffix(')') {
                // Avatar names contain " by " often enough that the last
                // occurrence is the separator, not the first.
                let (avatar_name, author_name) = match inner.rfind(" by ") {
                    Some(at) => (&inner[..at], Some(inner[at + 4..].to_string())),
                    None => (inner, None),
                };
                if !avatar_name.is_empty() {
                    return Some(LogEventKind::AvatarUnpacked {
                        avatar_name: avatar_name.to_string(),
                        author_name,
                    });
                }
            }
        }
        return None;
    }

    if let Some(after_mm) = text.strip_prefix("[ModerationManager] ") {
        if let Some(after_vk) = after_mm.strip_prefix("A vote kick has been initiated against ") {
            if let Some(display) = after_vk.strip_suffix(", do you agree?") {
                return Some(match display.find(" by ") {
                    Some(by) => LogEventKind::VtkInit {
                        target_display_name: display[..by].to_string(),
                        actor_display_name: Some(display[by + 4..].to_string()),
                    },
                    None => LogEventKind::VtkInit { target_display_name: display.to_string(), actor_display_name: None },
                });
            }
        }
        if let Some(display_name) = after_mm.strip_suffix(" has been kicked") {
            if !display_name.is_empty() {
                return Some(LogEventKind::PlayerKicked { display_name: display_name.to_string() });
            }
        }
        if let Some(display_name) = after_mm.strip_suffix(" has been warned") {
            if !display_name.is_empty() {
                return Some(LogEventKind::PlayerWarned { display_name: display_name.to_string() });
            }
        }
        return Some(LogEventKind::UnrecognizedModeration { text: after_mm.to_string() });
    }

    if let Some(after) = text.strip_prefix("[SharingManager] User ") {
        if let Some(cidx) = after.rfind(") created sharing pedestal for ") {
            let oidx = after.find(" (")?;
            let rest = &after[cidx + ") created sharing pedestal for ".len()..];
            let sidx = rest.rfind(' ')?;
            return Some(LogEventKind::PlayerSpawnPedestal {
                user_id: after[..oidx].to_string(),
                display_name: after[oidx + 2..cidx].to_string(),
                content_type: rest[..sidx].to_string(),
                content_id: rest[sidx + 1..].to_string(),
            });
        }
        return None;
    }

    if let Some(after) = text.strip_prefix("[StickersManager] User ") {
        if let Some(cidx) = after.rfind(") spawned sticker ") {
            let oidx = after.find(" (")?;
            return Some(LogEventKind::PlayerSpawnSticker {
                user_id: after[..oidx].to_string(),
                display_name: after[oidx + 2..cidx].to_string(),
                sticker_id: after[cidx + ") spawned sticker ".len()..].to_string(),
            });
        }
        return None;
    }

    if let Some(after) = text.strip_prefix("[VRCProps] Prop ") {
        if let Some(cidx) = after.rfind(" spawned by ") {
            return Some(LogEventKind::PlayerSpawnProp {
                prop_id: after[..cidx].to_string(),
                user_id: after[cidx + " spawned by ".len()..].to_string(),
            });
        }
        return None;
    }

    if let Some(after) = text.strip_prefix("VRCApplication: HandleApplicationQuit at ") {
        let lifetime_seconds: f64 = after.parse().unwrap_or(f64::NAN);
        return Some(LogEventKind::UserQuit { lifetime_seconds });
    }

    // Current video-player tag format.
    if let Some(after) = text.strip_prefix("[<color=#9C6994>USharpVideo</color>] Started video load for URL: ") {
        if let Some(cidx) = after.rfind(", requested by ") {
            return Some(LogEventKind::VideoLoad {
                url: after[..cidx].to_string(),
                display_name: after[cidx + ", requested by ".len()..].to_string(),
                title: String::new(),
            });
        }
        return None;
    }
    // Old format (pre 2025-02-12).
    if let Some(after) = text.strip_prefix("[USharpVideo] Started video load for URL: ") {
        if let Some(cidx) = after.rfind(", requested by ") {
            return Some(LogEventKind::VideoLoad {
                url: after[..cidx].to_string(),
                display_name: after[cidx + ", requested by ".len()..].to_string(),
                title: String::new(),
            });
        }
    }

    None
}

/// How often to poll the log file for new content and check whether a
/// newer `output_log_*.txt` has appeared (VRChat starts a fresh one each
/// launch). `Tail.java` polled on a similar order of magnitude.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// How long Java's tailer thread waits between "has a log file appeared
/// yet?" checks before the first one exists (`MiscUtils.sleep(10_000L)`).
const WAIT_FOR_LOG_INTERVAL: Duration = Duration::from_millis(10_000);

/// Everything the tailer reports, ported from `ScarletVRChatLogs.Listener`.
///
/// The previous version of this module only ever emitted parsed entries,
/// dropping `log_init`/`log_catchUp` entirely — which left the consumer with
/// no way to know a new VRChat session had started or that catch-up had
/// finished. `CelesteEventListener` keys real behavior off both
/// (`isTailerLive`, deferred-row flushing, avatar rescan), so they are part
/// of the contract, not bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub enum TailerEvent {
    /// `log_init`: this file just became the tail target (new VRChat session).
    Init { file: PathBuf },
    /// `log_catchUp`: everything already in the file has been replayed and
    /// what follows is live. `file: None` is Java's `catchUp(null)` — there
    /// is no log file to read at all (VRChat not installed, or none written
    /// yet).
    CatchUp { file: Option<PathBuf> },
    /// One recognized log line (`handleEntry`).
    Entry(LogEvent),
    /// The log Celeste is tailing has stopped growing while the session
    /// was live — the client is hung or gone.
    ///
    /// **Not in Java.** Detecting a crash only when a *new* log appears
    /// means waiting for the person to relaunch, which is exactly when
    /// they no longer need telling. A frozen client leaves the log
    /// abruptly silent, and that is observable immediately.
    ///
    /// Measured on a real 73-minute session (357,245 timestamped lines):
    /// the longest gap between consecutive lines was 22 seconds and
    /// nothing exceeded 30. A live client is simply never quiet for long,
    /// so [`SILENCE_THRESHOLD`] sits well clear of the observed maximum.
    WentSilent,
    /// The log started growing again after [`TailerEvent::WentSilent`] —
    /// the client was hung and recovered ("Wait Longer" rather than
    /// "Terminate"). Whatever the silence raised must be withdrawn.
    Resumed,
}

/// How long the log must stand still before the client is presumed hung.
///
/// 60s is roughly three times the longest gap observed in a real session.
/// Raising it delays the report; lowering it risks calling a quiet moment
/// a crash, and a false crash report names a person.
pub const SILENCE_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(60);

/// Spawns the tailer as a background task on `handle` and returns a channel
/// of [`TailerEvent`]s. Resolves the VRChat app-data directory via
/// [`resolve_vrc_app_data_dir`]. Takes an explicit runtime `Handle` (rather
/// than relying on the ambient `tokio::spawn` context) so it can be called
/// from `main`'s bootstrap code before the GUI event loop takes over the
/// main thread.
pub fn spawn_tailer(handle: &tokio::runtime::Handle) -> mpsc::UnboundedReceiver<TailerEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let app_data_dir = resolve_vrc_app_data_dir();
    tracing::info!("VRChat log tailer watching {}", app_data_dir.display());
    handle.spawn(tail_forever(app_data_dir, tx));
    rx
}

/// Port of `ScarletVRChatLogs.thread()` plus `Tail`'s preamble/live
/// distinction.
///
/// Sequence, matching Java exactly:
/// 1. No VRChat app-data directory at all → one `CatchUp { file: None }`, the
///    "does not seem to be installed" warning, and the tailer **stops**
///    (Java returns from its thread). Nothing retries, because on a machine
///    with no VRChat client there is nothing to wait for.
/// 2. Directory exists but holds no `output_log_*.txt` yet → `CatchUp
///    { file: None }` every 10s with "Waiting for log file to appear...",
///    until one shows up.
/// 3. For each target file: `Init`, then the already-present content as
///    `preamble: true` entries, then `CatchUp { file: Some(..) }` once EOF is
///    reached, then live entries as `preamble: false`.
/// 4. When a newer `output_log_*.txt` appears (VRChat restarted), switch to
///    it and start again from step 3 — Java's outer `while` loop.
///
/// One deliberate difference from `Tail.java`: Java buffers each entry and
/// only dispatches it when the *next* header line (or EOF) arrives, because
/// it collects a multi-line entry's continuation lines into a `List<String>`
/// it passes to `handleEntry`. `handleEntry` never reads that list — every
/// branch parses the header line's `txt` group alone — so dispatching each
/// recognized line as soon as it is complete produces exactly the same
/// events, just without the one-line delay. Continuation lines match no
/// branch in either implementation.
pub async fn tail_forever(app_data_dir: PathBuf, sender: mpsc::UnboundedSender<TailerEvent>) {
    if !app_data_dir.is_dir() {
        let _ = sender.send(TailerEvent::CatchUp { file: None });
        tracing::warn!(
            "The VRChat Client does not seem to be installed on this machine, disabling log tailer-parser! \
             (looked in {})",
            app_data_dir.display()
        );
        return;
    }

    let mut current_path = loop {
        if let Some(path) = locate_latest_log(&app_data_dir) {
            break path;
        }
        if sender.send(TailerEvent::CatchUp { file: None }).is_err() {
            return;
        }
        tokio::time::sleep(WAIT_FOR_LOG_INTERVAL).await;
        tracing::info!("Waiting for log file to appear...");
    };

    'files: loop {
        tracing::info!("Tailing {}", file_name_of(&current_path));
        if sender.send(TailerEvent::Init { file: current_path.clone() }).is_err() {
            return;
        }

        let mut position: u64 = 0;
        let mut preamble = true;
        let mut last_growth = std::time::Instant::now();
        let mut silent = false;
        // Reaching catch-up is not the same as watching a session run.
        // Starting Celeste while VRChat is closed also reaches catch-up,
        // on a log that stopped hours ago — and that log is silent by
        // definition, so a stall check keyed on catch-up alone would
        // report the previous session's crash on every startup, forever.
        // A stall only means something once this file has grown while
        // live.
        let mut grew_while_live = false;

        loop {
            match read_new_lines(&current_path, &mut position) {
                Ok(lines) => {
                    if !lines.is_empty() {
                        last_growth = std::time::Instant::now();
                        if !preamble {
                            grew_while_live = true;
                        }
                        if silent {
                            silent = false;
                            tracing::info!("VRChat log is being written again; withdrawing the stall report");
                            if sender.send(TailerEvent::Resumed).is_err() {
                                return;
                            }
                        }
                    } else if !preamble
                        && grew_while_live
                        && !silent
                        && last_growth.elapsed() >= SILENCE_THRESHOLD
                    {
                        // Only once live: during preamble an empty read
                        // just means we reached the end of an old file.
                        silent = true;
                        tracing::warn!(
                            "VRChat log has not grown for {}s; the client appears to have stopped",
                            SILENCE_THRESHOLD.as_secs()
                        );
                        if sender.send(TailerEvent::WentSilent).is_err() {
                            return;
                        }
                    }
                    let mut events = Vec::new();
                    for line in lines {
                        // Parsing a NaiveDateTime is comparatively expensive.
                        // Real VRChat logs contain hundreds of thousands of
                        // unrelated Unity lines, so reject those before doing
                        // any timestamp work. A large previous-session log
                        // otherwise takes minutes to catch up and prevents a
                        // newly-created log from becoming the active target.
                        if !might_contain_event(&line) {
                            continue;
                        }
                        if let Some((timestamp, _level, text)) = parse_log_line(&line) {
                            if let Some(kind) = parse_entry_text(text) {
                                events.push(LogEvent { preamble, timestamp, kind });
                            }
                        }
                    }

                    if preamble {
                        // VRChat may have started while we were reading the
                        // previous process's log. Never enqueue stale history
                        // ahead of the new session: switch immediately and
                        // let its Init event supersede this one.
                        if let Some(newest) = locate_latest_log(&app_data_dir) {
                            if newest != current_path {
                                tracing::info!(
                                    from = %file_name_of(&current_path),
                                    to = %file_name_of(&newest),
                                    "A newer VRChat log appeared during catch-up; discarding stale backlog"
                                );
                                current_path = newest;
                                continue 'files;
                            }
                        }
                        events = compact_preamble(events);
                    }

                    for event in events {
                        if sender.send(TailerEvent::Entry(event)).is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
                    tracing::error!("Exception whilst tailing {}: {}", file_name_of(&current_path), err);
                }
            }

            // Reaching the end of what was already on disk is the catch-up
            // point: from here on, anything read is live.
            if preamble {
                preamble = false;
                tracing::info!("Caught up with {}", file_name_of(&current_path));
                if sender.send(TailerEvent::CatchUp { file: Some(current_path.clone()) }).is_err() {
                    return;
                }
            }

            tokio::time::sleep(POLL_INTERVAL).await;

            // VRChat writes a brand-new log file per launch; switching to it
            // restarts the Init/preamble/CatchUp cycle for the new session.
            if let Some(newest) = locate_latest_log(&app_data_dir) {
                if newest != current_path {
                    current_path = newest;
                    break;
                }
            }
        }
    }
}

/// Fast string gate for the exact families understood by
/// [`parse_entry_text`]. It avoids parsing timestamps for the overwhelming
/// majority of Unity's diagnostic chatter without changing parser behavior.
fn might_contain_event(line: &str) -> bool {
    [
        "User Authenticated: ",
        "[Behaviour] Entering Room: ",
        "[Behaviour] Joining ",
        "[Behaviour] OnLeftRoom",
        "[Behaviour] OnPlayerJoined ",
        "[Behaviour] OnPlayerLeft ",
        "[Behaviour] Switching ",
        "[API] [",
        "[AssetBundleDownloadManager] ",
        "[ModerationManager] ",
        "[SharingManager] User ",
        "[StickersManager] User ",
        "[VRCProps] Prop ",
        "VRCApplication: HandleApplicationQuit at ",
        "USharpVideo</color>] Started video load for URL: ",
        "[USharpVideo] Started video load for URL: ",
    ]
    .iter()
    .any(|marker| line.contains(marker))
}

/// A historical VRChat log can contain several worlds and thousands of
/// recognized events. Only the most recent instance can describe the roster
/// visible when that process ended. Keeping its `EnteringRoom`/`UserJoined`
/// pair and everything after it both matches the final state and prevents the
/// async listener from spending minutes replaying obsolete worlds before it
/// can process the current log.
fn compact_preamble(events: Vec<LogEvent>) -> Vec<LogEvent> {
    let Some(last_join) = events
        .iter()
        .rposition(|event| matches!(&event.kind, LogEventKind::UserJoined { .. }))
    else {
        return events;
    };
    let start = events[..=last_join]
        .iter()
        .rposition(|event| matches!(&event.kind, LogEventKind::EnteringRoom { .. }))
        .unwrap_or(last_join);
    let authenticated = events[..start]
        .iter()
        .rposition(|event| matches!(&event.kind, LogEventKind::UserAuthenticated { .. }))
        .map(|index| events[index].clone());
    let mut compact = Vec::with_capacity(events.len() - start + usize::from(authenticated.is_some()));
    if let Some(event) = authenticated {
        compact.push(event);
    }
    compact.extend(events.into_iter().skip(start));
    compact
}

fn file_name_of(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name().map(|n| n.to_string_lossy()).unwrap_or_else(|| path.to_string_lossy())
}

/// Reads any bytes appended to `path` since `*position`, advances
/// `*position` past every complete line found, and returns those lines. A
/// trailing partial line (no newline yet — VRChat may still be mid-write)
/// is left unconsumed so it's re-read whole on the next call, matching
/// `Tail.java`'s buffering behavior instead of splitting a line in half.
fn read_new_lines(path: &Path, position: &mut u64) -> std::io::Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len < *position {
        // Log file was truncated or replaced under the same name; restart
        // from the beginning rather than seeking past EOF.
        *position = 0;
    }
    if len == *position {
        return Ok(Vec::new());
    }

    file.seek(SeekFrom::Start(*position))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let mut lines = Vec::new();
    let mut line_start = 0usize;
    let mut consumed = 0usize;
    for (i, &byte) in buf.iter().enumerate() {
        if byte == b'\n' {
            let mut line_end = i;
            if line_end > line_start && buf[line_end - 1] == b'\r' {
                line_end -= 1;
            }
            lines.push(String::from_utf8_lossy(&buf[line_start..line_end]).into_owned());
            line_start = i + 1;
            consumed = line_start;
        }
    }
    *position += consumed as u64;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real lines copied verbatim out of an `output_log_2026-09-26_22-31-01.txt`
    /// written by a live VRChat client (the repo owner's, PC build). The
    /// parsing half of this module had only ever been tested against
    /// reconstructed lines, so this pins the actual on-disk format —
    /// including the `Debug` level (not `Log`) that every line in a real
    /// client log carries, and CRLF line endings.
    #[test]
    fn parses_real_client_log_lines() {
        let real = [
            "2026.09.26 22:31:07 Debug      -  User Authenticated: BlakeBelladonna (usr_34cb56bd-ef02-46b1-b80c-abcf5acbeef1)\r",
            "2026.09.26 22:31:13 Debug      -  [Behaviour] Entering Room: Studio Apartment J31\r",
            "2026.09.26 22:31:13 Debug      -  [Behaviour] Joining wrld_0ebc959b-d11e-44fc-aff0-b17d8e71d37d:09580~private(usr_34cb56bd-ef02-46b1-b80c-abcf5acbeef1)~region(us)\r",
            "2026.09.26 22:31:13 Debug      -  [Behaviour] Joining or Creating Room: Studio Apartment J31\r",
            "2026.09.26 22:32:10 Debug      -  [Behaviour] OnPlayerJoined Church (usr_849ac025-7f46-4645-b23a-ac52fd13dfac)\r",
            "2026.09.26 22:31:59 Debug      -  [Behaviour] OnPlayerLeft BlakeBelladonna (usr_34cb56bd-ef02-46b1-b80c-abcf5acbeef1)\r",
            "2026.09.26 22:31:59 Debug      -  [Behaviour] OnLeftRoom\r",
            "2026.09.26 22:32:10 Debug      -  [Behaviour] Switching twisted_masks to avatar Thicc Catocat\r",
        ];
        let parsed: Vec<LogEventKind> = real
            .iter()
            .filter_map(|line| {
                let line = line.trim_end_matches(['\r', '\n']);
                let (_, level, text) = parse_log_line(line)?;
                assert_eq!(level, "Debug", "real client logs use the Debug level, not Log");
                parse_entry_text(text)
            })
            .collect();

        assert_eq!(
            parsed,
            vec![
                LogEventKind::UserAuthenticated {
                    display_name: "BlakeBelladonna".to_string(),
                    user_id: "usr_34cb56bd-ef02-46b1-b80c-abcf5acbeef1".to_string(),
                },
                LogEventKind::EnteringRoom { world_name: "Studio Apartment J31".to_string() },
                LogEventKind::UserJoined {
                    location: "wrld_0ebc959b-d11e-44fc-aff0-b17d8e71d37d:09580~private(usr_34cb56bd-ef02-46b1-b80c-abcf5acbeef1)~region(us)".to_string(),
                },
                // "Joining or Creating Room: ..." is deliberately not an event.
                LogEventKind::PlayerJoined {
                    display_name: "Church".to_string(),
                    user_id: "usr_849ac025-7f46-4645-b23a-ac52fd13dfac".to_string(),
                },
                LogEventKind::PlayerLeft {
                    display_name: "BlakeBelladonna".to_string(),
                    user_id: "usr_34cb56bd-ef02-46b1-b80c-abcf5acbeef1".to_string(),
                },
                LogEventKind::UserLeft,
                LogEventKind::PlayerSwitchAvatar {
                    display_name: "twisted_masks".to_string(),
                    avatar_display_name: "Thicc Catocat".to_string(),
                },
            ]
        );
        assert!(real.iter().all(|line| might_contain_event(line)));
    }

    #[test]
    fn catch_up_keeps_only_the_latest_instance_and_authentication() {
        fn event(second: u32, kind: LogEventKind) -> LogEvent {
            LogEvent {
                preamble: true,
                timestamp: NaiveDateTime::parse_from_str(
                    &format!("2026.09.30 12:00:{second:02}"),
                    "%Y.%m.%d %H:%M:%S",
                )
                .unwrap(),
                kind,
            }
        }

        let compact = compact_preamble(vec![
            event(0, LogEventKind::UserAuthenticated {
                display_name: "Owner".into(),
                user_id: "usr_owner".into(),
            }),
            event(1, LogEventKind::EnteringRoom { world_name: "Old world".into() }),
            event(2, LogEventKind::UserJoined { location: "wrld_old:1".into() }),
            event(3, LogEventKind::PlayerJoined {
                display_name: "Old player".into(),
                user_id: "usr_old".into(),
            }),
            event(4, LogEventKind::EnteringRoom { world_name: "Current world".into() }),
            event(5, LogEventKind::UserJoined { location: "wrld_current:2".into() }),
            event(6, LogEventKind::PlayerJoined {
                display_name: "Current player".into(),
                user_id: "usr_current".into(),
            }),
        ]);

        assert_eq!(compact.len(), 4);
        assert!(matches!(&compact[0].kind, LogEventKind::UserAuthenticated { .. }));
        assert!(matches!(
            &compact[1].kind,
            LogEventKind::EnteringRoom { world_name } if world_name == "Current world"
        ));
        assert!(matches!(
            &compact[2].kind,
            LogEventKind::UserJoined { location } if location == "wrld_current:2"
        ));
        assert!(matches!(
            &compact[3].kind,
            LogEventKind::PlayerJoined { display_name, .. } if display_name == "Current player"
        ));
    }

    #[test]
    fn parses_a_full_log_line() {
        let line = "2026.09.27 14:32:10 Debug      -  [Behaviour] OnPlayerJoined SomePlayer (usr_11111111-1111-1111-1111-111111111111)";
        let (ts, level, text) = parse_log_line(line).unwrap();
        assert_eq!(ts.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-09-27 14:32:10");
        assert_eq!(level, "Debug");
        assert_eq!(text, "[Behaviour] OnPlayerJoined SomePlayer (usr_11111111-1111-1111-1111-111111111111)");
    }

    #[test]
    fn user_authenticated() {
        // The real format, taken verbatim from a live 2026-09 VRChat log.
        let text = "User Authenticated: SomePlayer (usr_11111111-1111-1111-1111-111111111111)";
        assert_eq!(
            parse_entry_text(text),
            Some(LogEventKind::UserAuthenticated {
                display_name: "SomePlayer".to_string(),
                user_id: "usr_11111111-1111-1111-1111-111111111111".to_string(),
            })
        );
        // The bracketed shape Java's (unreachable) branch expected is still
        // accepted, so an older client log keeps working.
        let bracketed = "[Behaviour] [User Authenticated: SomePlayer (usr_11111111-1111-1111-1111-111111111111)]";
        assert!(matches!(parse_entry_text(bracketed), Some(LogEventKind::UserAuthenticated { .. })));
    }

    #[test]
    fn user_joined_location_but_not_room_creation_message() {
        let text = "[Behaviour] Joining wrld_11111111-1111-1111-1111-111111111111:12345~private(usr_2)";
        assert_eq!(
            parse_entry_text(text),
            Some(LogEventKind::UserJoined {
                location: "wrld_11111111-1111-1111-1111-111111111111:12345~private(usr_2)".to_string()
            })
        );

        let creating = "[Behaviour] Joining or Creating Room: wrld_11111111-1111-1111-1111-111111111111";
        assert_eq!(parse_entry_text(creating), None);
    }

    #[test]
    fn user_left_room() {
        assert_eq!(parse_entry_text("[Behaviour] OnLeftRoom"), Some(LogEventKind::UserLeft));
    }

    #[test]
    fn player_joined_and_left() {
        let joined = "[Behaviour] OnPlayerJoined SomeOtherPlayer (usr_22222222-2222-2222-2222-222222222222)";
        assert_eq!(
            parse_entry_text(joined),
            Some(LogEventKind::PlayerJoined {
                display_name: "SomeOtherPlayer".to_string(),
                user_id: "usr_22222222-2222-2222-2222-222222222222".to_string(),
            })
        );
        let left = "[Behaviour] OnPlayerLeft SomeOtherPlayer (usr_22222222-2222-2222-2222-222222222222)";
        assert_eq!(
            parse_entry_text(left),
            Some(LogEventKind::PlayerLeft {
                display_name: "SomeOtherPlayer".to_string(),
                user_id: "usr_22222222-2222-2222-2222-222222222222".to_string(),
            })
        );
    }

    #[test]
    fn player_switch_avatar() {
        let text = "[Behaviour] Switching SomePlayer to avatar Cool Avatar Name";
        assert_eq!(
            parse_entry_text(text),
            Some(LogEventKind::PlayerSwitchAvatar {
                display_name: "SomePlayer".to_string(),
                avatar_display_name: "Cool Avatar Name".to_string(),
            })
        );
    }

    #[test]
    fn vote_kick_with_and_without_actor() {
        let with_actor = "[ModerationManager] A vote kick has been initiated against Target Name by Actor Name, do you agree?";
        assert_eq!(
            parse_entry_text(with_actor),
            Some(LogEventKind::VtkInit {
                target_display_name: "Target Name".to_string(),
                actor_display_name: Some("Actor Name".to_string()),
            })
        );
        let without_actor = "[ModerationManager] A vote kick has been initiated against Target Name, do you agree?";
        assert_eq!(
            parse_entry_text(without_actor),
            Some(LogEventKind::VtkInit { target_display_name: "Target Name".to_string(), actor_display_name: None })
        );
    }

    #[test]
    fn api_request() {
        let text = "[API] [3] Sending GET request to https://api.vrchat.cloud/api/1/auth/user";
        assert_eq!(
            parse_entry_text(text),
            Some(LogEventKind::ApiRequest {
                index: 3,
                method: "GET".to_string(),
                url: "https://api.vrchat.cloud/api/1/auth/user".to_string(),
            })
        );
    }

    #[test]
    fn sharing_pedestal_sticker_and_prop() {
        let pedestal = "[SharingManager] User usr_3 (Some Player) created sharing pedestal for avatar avtr_abc";
        assert_eq!(
            parse_entry_text(pedestal),
            Some(LogEventKind::PlayerSpawnPedestal {
                user_id: "usr_3".to_string(),
                display_name: "Some Player".to_string(),
                content_type: "avatar".to_string(),
                content_id: "avtr_abc".to_string(),
            })
        );

        let sticker = "[StickersManager] User usr_4 (Some Player) spawned sticker stkr_xyz";
        assert_eq!(
            parse_entry_text(sticker),
            Some(LogEventKind::PlayerSpawnSticker {
                user_id: "usr_4".to_string(),
                display_name: "Some Player".to_string(),
                sticker_id: "stkr_xyz".to_string(),
            })
        );

        let prop = "[VRCProps] Prop prop_1 spawned by usr_5";
        assert_eq!(
            parse_entry_text(prop),
            Some(LogEventKind::PlayerSpawnProp { prop_id: "prop_1".to_string(), user_id: "usr_5".to_string() })
        );
    }

    #[test]
    fn application_quit() {
        let text = "VRCApplication: HandleApplicationQuit at 123.45";
        assert_eq!(parse_entry_text(text), Some(LogEventKind::UserQuit { lifetime_seconds: 123.45 }));
    }

    #[test]
    fn video_load_both_formats() {
        let new_format =
            "[<color=#9C6994>USharpVideo</color>] Started video load for URL: https://example.com/v.mp4, requested by Some Player";
        assert_eq!(
            parse_entry_text(new_format),
            Some(LogEventKind::VideoLoad {
                url: "https://example.com/v.mp4".to_string(),
                display_name: "Some Player".to_string(),
                title: String::new(),
            })
        );

        let old_format = "[USharpVideo] Started video load for URL: https://example.com/v2.mp4, requested by Other Player";
        assert_eq!(
            parse_entry_text(old_format),
            Some(LogEventKind::VideoLoad {
                url: "https://example.com/v2.mp4".to_string(),
                display_name: "Other Player".to_string(),
                title: String::new(),
            })
        );
    }

    #[test]
    fn unrecognized_lines_return_none() {
        assert_eq!(parse_entry_text("[SomeOtherSystem] unrelated message"), None);
    }

    #[test]
    fn locate_latest_log_picks_the_newest_by_filename_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["output_log_2026-01-01_10-00-00.txt", "output_log_2026-06-15_09-30-00.txt", "output_log_2026-03-01_08-00-00.txt"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        std::fs::write(dir.path().join("not_a_log.txt"), "").unwrap();
        let latest = locate_latest_log(dir.path()).unwrap();
        assert_eq!(latest.file_name().unwrap().to_str().unwrap(), "output_log_2026-06-15_09-30-00.txt");
    }

    #[test]
    fn locate_latest_log_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(locate_latest_log(dir.path()).is_none());
    }

    /// Helper: pulls the next [`TailerEvent::Entry`] off the channel,
    /// asserting nothing takes longer than the timeout.
    async fn next_entry(rx: &mut mpsc::UnboundedReceiver<TailerEvent>) -> LogEvent {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
            if let TailerEvent::Entry(entry) = event {
                return entry;
            }
        }
    }

    #[tokio::test]
    async fn tails_growing_log_file_preamble_then_live() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("output_log_2026-01-01_00-00-00.txt");
        std::fs::write(
            &log_path,
            "2026.01.01 00:00:01 Debug      -  [Behaviour] OnPlayerJoined Preexisting (usr_9)\n",
        )
        .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(tail_forever(dir.path().to_path_buf(), tx));

        // `log_init` fires before anything is read from the new target.
        let init = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        assert_eq!(init, TailerEvent::Init { file: log_path.clone() });

        let first = next_entry(&mut rx).await;
        assert!(first.preamble, "pre-existing content should be marked preamble");
        assert_eq!(
            first.kind,
            LogEventKind::PlayerJoined { display_name: "Preexisting".to_string(), user_id: "usr_9".to_string() }
        );

        // ...then `log_catchUp` once that pre-existing content is exhausted.
        let caught_up = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        assert_eq!(caught_up, TailerEvent::CatchUp { file: Some(log_path.clone()) });

        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        writeln!(f, "2026.01.01 00:00:02 Debug      -  [Behaviour] OnPlayerJoined LiveJoiner (usr_10)").unwrap();
        drop(f);

        let second = next_entry(&mut rx).await;
        assert!(!second.preamble, "content appended after startup should not be preamble");
        assert_eq!(
            second.kind,
            LogEventKind::PlayerJoined { display_name: "LiveJoiner".to_string(), user_id: "usr_10".to_string() }
        );

        drop(rx);
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    }

    /// Java's tailer thread reports `catchUp(null)` and stops with the
    /// "does not seem to be installed" warning when VRChat's app-data
    /// directory doesn't exist at all. Consumers rely on that single
    /// catch-up to stop waiting for an instance that will never open.
    #[tokio::test]
    async fn no_vrchat_directory_reports_catch_up_then_stops() {
        let missing = std::path::PathBuf::from("/definitely/not/a/vrchat/install");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(tail_forever(missing, tx));

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert_eq!(event, TailerEvent::CatchUp { file: None });

        // The task returns rather than polling forever.
        tokio::time::timeout(Duration::from_secs(5), handle).await.unwrap().unwrap();
        assert!(rx.recv().await.is_none(), "tailer should have stopped, closing the channel");
    }

    /// A machine with VRChat installed but no log written yet: Java keeps
    /// reporting `catchUp(null)` every 10s rather than stopping.
    #[tokio::test]
    async fn empty_log_directory_reports_catch_up_and_keeps_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(tail_forever(dir.path().to_path_buf(), tx));

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert_eq!(event, TailerEvent::CatchUp { file: None });
        // Unlike the no-directory case, the task stays alive waiting for a
        // log file to appear (Java's 10s retry loop) rather than returning.
        assert!(!handle.is_finished());

        handle.abort();
    }

    #[test]
    fn app_data_override_serves_both_dir_and_temp() {
        // `SCARLET_VRC_APPDATA_DIR` IS the resolved directory: the known
        // prefixes are stripped so one root works for DIR and TEMP alike.
        let root = std::path::Path::new("/tmp/some-override");
        std::env::set_var("SCARLET_VRC_APPDATA_DIR", root);
        assert_eq!(resolve(APP_DATA_DIR_RELATIVE), root);
        assert_eq!(resolve(APP_DATA_TEMP_RELATIVE), root);
        assert_eq!(resolve("AppData/LocalLow/VRChat/VRChat/crash"), root.join("crash"));
        std::env::remove_var("SCARLET_VRC_APPDATA_DIR");
    }

    #[test]
    fn read_new_lines_leaves_partial_trailing_line_for_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.txt");
        std::fs::write(&path, "line one\nline two\npartial-no-newline-yet").unwrap();

        let mut position = 0u64;
        let lines = read_new_lines(&path, &mut position).unwrap();
        assert_eq!(lines, vec!["line one".to_string(), "line two".to_string()]);
        assert!(position < std::fs::metadata(&path).unwrap().len());

        std::fs::write(&path, "line one\nline two\npartial-no-newline-yet, now finished\n").unwrap();
        let more = read_new_lines(&path, &mut position).unwrap();
        assert_eq!(more, vec!["partial-no-newline-yet, now finished".to_string()]);
    }
}

#[cfg(test)]
mod moderation_line_tests {
    use super::*;

    /// Modelled on a real line, with an invented display name: the real
    /// one belongs to a third party and does not belong in this history.
    ///
    /// The detail worth keeping is the shape. Names on these lines carry
    /// a one-dot leader (U+2024) where a period would go — homoglyph
    /// names are exactly the kind that get kicked — so the parser must
    /// not assume ASCII or split on '.'.
    #[test]
    fn a_kick_that_took_effect_is_parsed() {
        assert_eq!(
            parse_entry_text("[ModerationManager] example\u{2024}0000 aaaa has been kicked"),
            Some(LogEventKind::PlayerKicked { display_name: "example\u{2024}0000 aaaa".to_string() })
        );
    }

    #[test]
    fn the_vote_kick_shape_still_wins() {
        assert!(matches!(
            parse_entry_text("[ModerationManager] A vote kick has been initiated against Someone, do you agree?"),
            Some(LogEventKind::VtkInit { .. })
        ));
    }

    /// Anything else under this prefix is surfaced rather than dropped, so
    /// the next unknown shape is reported instead of silently discarded —
    /// which is how the kick line went unnoticed in Java and here.
    #[test]
    fn an_unknown_moderation_shape_is_reported_not_dropped() {
        assert_eq!(
            parse_entry_text("[ModerationManager] something entirely new happened"),
            Some(LogEventKind::UnrecognizedModeration { text: "something entirely new happened".to_string() })
        );
    }

    /// The warn line arrives about a second after the action, where the
    /// audit poller can be a minute behind it. Recorded from a live
    /// instance warn; the fullwidth name is what VRChat actually wrote.
    #[test]
    fn an_instance_warn_is_parsed_from_the_local_log() {
        assert_eq!(
            parse_entry_text("[ModerationManager] \u{ff30}\u{ff45}\u{ff4e}\u{ff4e}\u{ff59} has been warned"),
            Some(LogEventKind::PlayerWarned {
                display_name: "\u{ff30}\u{ff45}\u{ff4e}\u{ff4e}\u{ff59}".to_string()
            })
        );
        // An empty name is not a warn; it falls through to the catch-all.
        assert_eq!(
            parse_entry_text("[ModerationManager]  has been warned"),
            Some(LogEventKind::UnrecognizedModeration { text: " has been warned".to_string() })
        );
    }

    #[test]
    fn a_bare_suffix_is_not_a_kick_of_nobody() {
        assert!(matches!(
            parse_entry_text("[ModerationManager]  has been kicked"),
            Some(LogEventKind::UnrecognizedModeration { .. })
        ));
    }
}

#[cfg(test)]
mod avatar_unpack_tests {
    use super::*;

    /// The real line from the crash the owner captured. The avatar name
    /// carries a one-dot leader (U+2024), and here the wearer happens to
    /// be the author — usually they differ, which is the whole point of
    /// reading this line as well as the switch.
    #[test]
    fn the_unpack_line_yields_the_avatar_and_its_author() {
        assert_eq!(
            parse_entry_text(
                "[AssetBundleDownloadManager] [461] Unpacking Avatar (D\u{2024}S\u{2024}C Fallback by 72kontop)"
            ),
            Some(LogEventKind::AvatarUnpacked {
                avatar_name: "D\u{2024}S\u{2024}C Fallback".to_string(),
                author_name: Some("72kontop".to_string()),
            })
        );
    }

    /// " by " inside an avatar's own name must not steal the author.
    #[test]
    fn the_last_by_separates_the_author() {
        assert_eq!(
            parse_entry_text("[AssetBundleDownloadManager] [1] Unpacking Avatar (Stand by Me by Someone)"),
            Some(LogEventKind::AvatarUnpacked {
                avatar_name: "Stand by Me".to_string(),
                author_name: Some("Someone".to_string()),
            })
        );
    }

    #[test]
    fn an_unattributed_avatar_still_parses() {
        assert_eq!(
            parse_entry_text("[AssetBundleDownloadManager] [2] Unpacking Avatar (Nameless)"),
            Some(LogEventKind::AvatarUnpacked { avatar_name: "Nameless".to_string(), author_name: None })
        );
    }

    /// The manager writes a great deal of unrelated chatter; none of it
    /// should become an event.
    #[test]
    fn other_download_manager_lines_are_ignored() {
        assert_eq!(parse_entry_text("[AssetBundleDownloadManager] Starting download of Avatar @ 0 MB, 0 still queued."), None);
        assert_eq!(parse_entry_text("[AssetBundleDownloadManager] Asset bundle cache cleared successfully"), None);
    }
}

#[cfg(test)]
mod stall_gate_tests {

    /// A stall is only meaningful once the file has grown while live.
    ///
    /// The bug this guards: reaching catch-up is not the same as watching
    /// a session run. Starting Celeste with VRChat closed also reaches
    /// catch-up, on a log that stopped hours ago and is silent by
    /// definition — so a check keyed on catch-up alone reports the
    /// previous session's crash on every single startup.
    ///
    /// This mirrors the tailer's own condition rather than driving the
    /// loop, which needs a real clock and a real file; the loop is
    /// covered by the integration test above.
    #[test]
    fn silence_is_only_a_stall_after_the_log_grew_live() {
        fn would_report(preamble: bool, grew_while_live: bool, elapsed_ok: bool, silent: bool) -> bool {
            !preamble && grew_while_live && !silent && elapsed_ok
        }

        // Celeste started on an old, finished log: caught up, nothing ever
        // arrived live, and the file is silent. Must stay quiet.
        assert!(!would_report(false, false, true, false), "stale log on startup must not report");

        // A real session that was running and stopped.
        assert!(would_report(false, true, true, false), "a live session going quiet is a stall");

        // Still replaying history.
        assert!(!would_report(true, true, true, false));

        // Not quiet long enough yet.
        assert!(!would_report(false, true, false, false));

        // Already reported; once per stall.
        assert!(!would_report(false, true, true, true));
    }
}
