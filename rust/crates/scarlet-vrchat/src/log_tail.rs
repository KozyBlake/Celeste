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

/// Port of `VrcAppData.resolve`: locates VRChat's `AppData/LocalLow/VRChat/VRChat`
/// equivalent directory (where `output_log_*.txt` files live).
///
/// - `SCARLET_VRC_APPDATA_DIR` env var, if set, is used directly (matches
///   the Java override, minus the Android-companion-app path stripping
///   logic, which isn't relevant to this desktop port).
/// - Linux: tries each detected Steam library's Proton `compatdata` path
///   for VRChat's app id, falling back to `~/.steam/steam/...`.
/// - Everything else (Windows, macOS): `<home>/AppData/LocalLow/VRChat/VRChat`
///   directly, matching the Java version's non-Linux branch (VRChat has no
///   native macOS client, but this mirrors the Java fallback rather than
///   guessing further).
pub fn resolve_vrc_app_data_dir() -> PathBuf {
    if let Ok(over) = std::env::var("SCARLET_VRC_APPDATA_DIR") {
        if !over.is_empty() {
            return PathBuf::from(over);
        }
    }

    let home = dirs_home();

    if cfg!(target_os = "linux") {
        if let Some(detected) = find_in_steam_libraries(&home) {
            return detected;
        }
        return home.join(".steam/steam").join(proton_prefix()).join("AppData/LocalLow/VRChat/VRChat");
    }

    home.join("AppData/LocalLow/VRChat/VRChat")
}

fn proton_prefix() -> PathBuf {
    PathBuf::from(format!("steamapps/compatdata/{VRCHAT_APP_ID}/pfx/drive_c/users/steamuser"))
}

/// Resolves the user's home directory across platforms. `HOME` is the
/// Unix/macOS convention; Windows doesn't set it by default and uses
/// `USERPROFILE` instead (falling back to `HOMEDRIVE`+`HOMEPATH` on older
/// setups where even that's unset).
fn dirs_home() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
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
fn find_in_steam_libraries(home: &Path) -> Option<PathBuf> {
    for root in detect_steam_library_roots(home) {
        let candidate = root.join(proton_prefix()).join("AppData/LocalLow/VRChat/VRChat");
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
    if let Some(after_behaviour) = text.strip_prefix("[Behaviour] ") {
        if let Some(rest) = after_behaviour.strip_prefix("[User Authenticated: ") {
            let name_end = rest.rfind(" (")?;
            let id_end = rest.rfind(')')?;
            return Some(LogEventKind::UserAuthenticated {
                display_name: rest[..name_end].to_string(),
                user_id: rest[name_end + 2..id_end].to_string(),
            });
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
        return None;
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

/// Spawns the tailer as a background task on `handle` and returns a channel
/// of parsed log events. Resolves the VRChat app-data directory via
/// [`resolve_vrc_app_data_dir`]. Takes an explicit runtime `Handle` (rather
/// than relying on the ambient `tokio::spawn` context) so it can be called
/// from `main`'s bootstrap code before the GUI event loop takes over the
/// main thread.
pub fn spawn_tailer(handle: &tokio::runtime::Handle) -> mpsc::UnboundedReceiver<LogEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let app_data_dir = resolve_vrc_app_data_dir();
    tracing::info!("VRChat log tailer watching {}", app_data_dir.display());
    handle.spawn(tail_forever(app_data_dir, tx));
    rx
}

/// Runs forever, tailing whichever `output_log_*.txt` in `app_data_dir` is
/// currently newest, sending a [`LogEvent`] for every recognized line.
/// Content that already existed the first time a given log file is opened
/// is sent with `preamble: true` (silent catch-up, matching
/// `ScarletVRChatLogs`'s own preamble/live distinction); everything
/// appended afterwards is `preamble: false`. Returns once `sender`'s
/// receiver is dropped.
pub async fn tail_forever(app_data_dir: PathBuf, sender: mpsc::UnboundedSender<LogEvent>) {
    let mut current_path: Option<PathBuf> = None;
    let mut position: u64 = 0;
    let mut preamble = true;

    loop {
        match locate_latest_log(&app_data_dir) {
            None => {
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            Some(path) if current_path.as_ref() != Some(&path) => {
                current_path = Some(path);
                position = 0;
                preamble = true;
            }
            Some(_) => {}
        }

        let path = current_path.clone().expect("just resolved above");
        if let Ok(lines) = read_new_lines(&path, &mut position) {
            let this_batch_is_preamble = preamble;
            for line in lines {
                if let Some((timestamp, _level, text)) = parse_log_line(&line) {
                    if let Some(kind) = parse_entry_text(text) {
                        let event = LogEvent { preamble: this_batch_is_preamble, timestamp, kind };
                        if sender.send(event).is_err() {
                            return;
                        }
                    }
                }
            }
            // The first read (whatever it contained) caught up to the
            // then-current end of file; everything from here on is live.
            preamble = false;
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
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
        let text = "[Behaviour] [User Authenticated: SomePlayer (usr_11111111-1111-1111-1111-111111111111)]";
        assert_eq!(
            parse_entry_text(text),
            Some(LogEventKind::UserAuthenticated {
                display_name: "SomePlayer".to_string(),
                user_id: "usr_11111111-1111-1111-1111-111111111111".to_string(),
            })
        );
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

        let first = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        assert!(first.preamble, "pre-existing content should be marked preamble");
        assert_eq!(
            first.kind,
            LogEventKind::PlayerJoined { display_name: "Preexisting".to_string(), user_id: "usr_9".to_string() }
        );

        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        writeln!(f, "2026.01.01 00:00:02 Debug      -  [Behaviour] OnPlayerJoined LiveJoiner (usr_10)").unwrap();
        drop(f);

        let second = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        assert!(!second.preamble, "content appended after startup should not be preamble");
        assert_eq!(
            second.kind,
            LogEventKind::PlayerJoined { display_name: "LiveJoiner".to_string(), user_id: "usr_10".to_string() }
        );

        drop(rx);
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
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
