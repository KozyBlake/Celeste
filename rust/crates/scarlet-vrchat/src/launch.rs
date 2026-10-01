//! Cross-platform port of `ext/VrcLaunch` for Celeste's automatic
//! follow-into-new-instance feature.

use std::process::Command;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const DEBUG_LEVELS: &str = "--log-debug-levels=API;All;Always;AssetBundleDownloadManager;ContentCreator;Errors;NetworkData;NetworkProcessing;NetworkTransport;Warnings";
const RELAUNCH_SUPPRESS: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Shared by the fast detector and later audit event, matching Java's one
/// dedupe map for both observation paths.
pub fn claim_recent_launch(location: &str) -> bool {
    static RECENT: OnceLock<Mutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    let now = std::time::Instant::now();
    let mut recent = RECENT.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    recent.retain(|_, launched| now.duration_since(*launched) <= RELAUNCH_SUPPRESS);
    if recent.get(location).is_some_and(|launched| now.duration_since(*launched) < RELAUNCH_SUPPRESS) {
        return false;
    }
    recent.insert(location.to_string(), now);
    true
}

fn encode_query(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(*byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

pub fn build_launch_uri(location: &str, secure_name: Option<&str>) -> String {
    // Keep Java's established ref value; VRChat may use it as an external
    // launch allow-list/telemetry identifier rather than visible branding.
    let mut uri = format!("vrchat://launch?ref=KozyBlakeCeleste&id={location}");
    if let Some(name) = secure_name.map(str::trim).filter(|name| !name.is_empty()) {
        uri.push_str("&shortName=");
        uri.push_str(&encode_query(name));
    }
    uri
}

fn vrchat_pids() -> Vec<String> {
    let output = if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq VRChat.exe", "/FO", "CSV", "/NH"])
            .output()
    } else {
        Command::new("pgrep").args(["-f", "VRChat.exe"]).output()
    };
    let Ok(output) = output else { return Vec::new() };
    let text = String::from_utf8_lossy(&output.stdout);
    if cfg!(windows) {
        text.lines()
            .filter(|line| line.trim_start().starts_with("\"VRChat.exe\""))
            .filter_map(|line| line.split("\",\"").nth(1))
            .map(|pid| pid.trim_matches('"').trim().to_string())
            .filter(|pid| pid.chars().all(|ch| ch.is_ascii_digit()))
            .collect()
    } else {
        text.lines()
            .map(str::trim)
            .filter(|pid| !pid.is_empty() && pid.chars().all(|ch| ch.is_ascii_digit()))
            .map(str::to_string)
            .collect()
    }
}

fn quit_running_vrchat() {
    let pids = vrchat_pids();
    if pids.is_empty() {
        return;
    }
    tracing::info!(?pids, "VRChat is running; closing it before automatic instance follow");
    if cfg!(windows) {
        let _ = Command::new("taskkill").args(["/IM", "VRChat.exe"]).status();
    } else {
        let mut command = Command::new("kill");
        command.arg("-TERM").args(&pids);
        let _ = command.status();
    }
    for _ in 0..20 {
        if vrchat_pids().is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let survivors = vrchat_pids();
    if survivors.is_empty() {
        return;
    }
    tracing::warn!(?survivors, "VRChat did not exit gracefully; forcing it closed for automatic follow");
    if cfg!(windows) {
        let _ = Command::new("taskkill").args(["/F", "/IM", "VRChat.exe"]).status();
    } else {
        let mut command = Command::new("kill");
        command.arg("-KILL").args(survivors);
        let _ = command.status();
    }
}

/// Cold-launches desktop-mode VRChat into `location`. This intentionally
/// mirrors Java's quit-and-relaunch behavior: VRChat exposes no prompt-free
/// way for an external tool to move an already-running client in place.
pub fn launch_desktop(user_id: Option<&str>, location: &str, secure_name: Option<&str>) -> std::io::Result<()> {
    quit_running_vrchat();
    let uri = build_launch_uri(location, secure_name);
    let mut args = vec![
        "--no-vr".to_string(),
        "--enable-sdk-log-levels".to_string(),
        "--enable-udon-debug-logging".to_string(),
        "--enable-verbose-logging".to_string(),
        DEBUG_LEVELS.to_string(),
        uri.clone(),
    ];
    if let Some(user_id) = user_id.filter(|id| !id.is_empty()) {
        args.push(format!("--profile={user_id}"));
    }

    // Steam's direct applaunch form preserves every debug/deep-link arg on
    // Linux and is also usable when Steam is available in PATH on Windows.
    if Command::new(if cfg!(windows) { "steam.exe" } else { "steam" })
        .arg("-applaunch")
        .arg(crate::log_tail::VRCHAT_APP_ID)
        .args(&args)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // Protocol-handler fallback. It retains the secure location token even
    // where Steam isn't in PATH, though OS protocol handlers cannot carry
    // the extra debug flags.
    if cfg!(windows) {
        Command::new("cmd").args(["/C", "start", "", &uri]).spawn().map(|_| ())
    } else {
        Command::new("xdg-open").arg(uri).spawn().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_uri_contains_and_encodes_the_secure_join_name() {
        assert_eq!(
            build_launch_uri("wrld_1:123~group(grp_1)", Some("secure name/+")),
            "vrchat://launch?ref=KozyBlakeCeleste&id=wrld_1:123~group(grp_1)&shortName=secure%20name%2F%2B"
        );
        assert_eq!(
            build_launch_uri("wrld_1:public", None),
            "vrchat://launch?ref=KozyBlakeCeleste&id=wrld_1:public"
        );
    }
}
