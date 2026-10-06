//! Cross-platform port of `ext/VrcLaunch` for Celeste's automatic
//! follow-into-new-instance feature.

use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use sysinfo::{ProcessesToUpdate, System};

const DEBUG_LEVELS: &str = "--log-debug-levels=API;All;Always;AssetBundleDownloadManager;ContentCreator;Errors;NetworkData;NetworkProcessing;NetworkTransport;Warnings";
const RELAUNCH_SUPPRESS: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Shared by the fast detector and later audit event, matching Java's one
/// dedupe map for both observation paths.
pub fn claim_recent_launch(location: &str) -> bool {
    static RECENT: OnceLock<Mutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    let now = std::time::Instant::now();
    let mut recent = RECENT.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
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

/// Running VRChat processes associated with one numeric local profile.
/// Profile zero may be an older manually launched process with no explicit
/// profile argument, but only when it is the sole VRChat process; guessing in
/// a multi-client setup would risk terminating the wrong account.
pub fn profile_pids(profile: u32) -> Vec<u32> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let mut all = Vec::new();
    let mut exact = Vec::new();
    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        if name != "vrchat.exe" && name != "vrchat" {
            continue;
        }
        let pid = pid.as_u32();
        all.push(pid);
        let args = process.cmd().iter().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>();
        let inline = format!("--profile={profile}");
        let paired = args.windows(2).any(|pair| pair[0] == "--profile" && pair[1] == profile.to_string());
        if args.iter().any(|arg| arg.as_ref() == inline) || paired {
            exact.push(pid);
        }
    }
    if exact.is_empty() && profile == 0 && all.len() == 1 {
        all
    } else {
        exact
    }
}

pub fn profile_is_running(profile: u32) -> bool {
    !profile_pids(profile).is_empty()
}

fn profile_argument(profile: u32) -> String {
    format!("--profile={profile}")
}

/// Stop only the process belonging to `profile`. Never uses image-wide
/// `taskkill`; that old behavior closed every moderator client on the host.
pub fn stop_profile(profile: u32) -> io::Result<()> {
    let pids = profile_pids(profile);
    if pids.is_empty() {
        return Ok(());
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    for pid in pids {
        let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) else { continue };
        if !process.kill() {
            return Err(io::Error::other(format!("could not stop VRChat profile {profile} (PID {pid})")));
        }
    }
    Ok(())
}

/// How long to wait for a launch request to actually produce a client.
/// A cold VRChat start goes through Steam and EAC, so this is generous.
const LAUNCH_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const LAUNCH_CONFIRM_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Cold-launches desktop-mode VRChat into `location`. This intentionally
/// mirrors Java's quit-and-relaunch behavior: VRChat exposes no prompt-free
/// way for an external tool to move an already-running client in place.
pub fn launch_desktop(profile: u32, location: &str, secure_name: Option<&str>) -> std::io::Result<()> {
    if profile_is_running(profile) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("VRChat profile {profile} is already running"),
        ));
    }
    let uri = build_launch_uri(location, secure_name);
    let mut args = vec![
        "--no-vr".to_string(),
        "--enable-sdk-log-levels".to_string(),
        "--enable-udon-debug-logging".to_string(),
        "--enable-verbose-logging".to_string(),
        DEBUG_LEVELS.to_string(),
        uri.clone(),
    ];
    args.push(profile_argument(profile));

    // Steam's direct applaunch form preserves every debug/deep-link arg on
    // Linux and is also usable when Steam is available in PATH on Windows.
    for executable in steam_candidates() {
        let Ok(mut child) = Command::new(&executable)
            .arg("-applaunch")
            .arg(crate::log_tail::VRCHAT_APP_ID)
            .args(&args)
            .spawn()
        else {
            continue;
        };
        tracing::info!(profile, location, launcher = %executable.display(), "Asked Steam to start the managed VRChat profile");
        // Spawning Steam is not the same as starting VRChat. `steam
        // -applaunch` hands the request over and exits reporting success
        // whatever happens next, so without this the supervisor would
        // record a launch that never produced a client, then retry and
        // burn its restart budget on a silent failure.
        //
        // The case that makes this more than belt-and-braces: Steam serves
        // a second request for an app id it is already running by focusing
        // the existing copy rather than starting another. If that is what
        // happens here, every slot after the first fails, and it fails
        // quietly. Confirming the process means the operator is told.
        let deadline = std::time::Instant::now() + LAUNCH_CONFIRM_TIMEOUT;
        while std::time::Instant::now() < deadline {
            // Reap the launcher so a long-running Celeste does not collect
            // zombies; `steam -applaunch` exits almost immediately.
            let _ = child.try_wait();
            if profile_is_running(profile) {
                tracing::info!(profile, location, "Managed VRChat profile is running");
                return Ok(());
            }
            std::thread::sleep(LAUNCH_CONFIRM_POLL);
        }
        let _ = child.try_wait();
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "Steam accepted the launch but no VRChat process with --profile={profile} appeared within {}s. \
                 If another VRChat client is already running, Steam may have focused it instead of starting a second copy.",
                LAUNCH_CONFIRM_TIMEOUT.as_secs()
            ),
        ));
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Steam could not be started; managed clients require the normal Steam/EAC launch path",
    ))
}

/// Move one managed profile by stopping only that profile and waiting for it
/// to leave the process table before launching it through Steam again.
pub fn relaunch_desktop(profile: u32, location: &str, secure_name: Option<&str>) -> io::Result<()> {
    stop_profile(profile)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while profile_is_running(profile) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if profile_is_running(profile) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("VRChat profile {profile} did not stop within 10 seconds"),
        ));
    }
    launch_desktop(profile, location, secure_name)
}

fn steam_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(windows) {
        for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(root) = std::env::var_os(variable) {
                candidates.push(std::path::PathBuf::from(root).join("Steam/steam.exe"));
            }
        }
        candidates.push("steam.exe".into());
    } else {
        candidates.push("steam".into());
    }
    candidates
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

    #[test]
    fn steam_launch_uses_numeric_profiles() {
        assert_eq!(profile_argument(3), "--profile=3");
    }
}
