//! Port of `net.sybyline.scarlet.log` (`ScarletLogger`, `ScarletLoggerFactory`,
//! `ScarletLoggerService`).
//!
//! The Java version is a hand-written SLF4J provider: it renames logger
//! targets (`net.sybyline.scarlet.*` -> `KozyBlake/Scarlet/*`, JDA loggers get
//! a `JDA/`/`JDA-I/` prefix), formats `%F %T [LEVEL] [name] message` lines,
//! and fans them out to stdout/stderr *and* a per-run log file on a
//! dedicated background thread, flushed by a shutdown hook.
//!
//! In Rust we don't have an SLF4J-style provider SPI to implement (there's no
//! `ScarletLoggerService` equivalent needed — `tracing` is used directly by
//! every crate instead of routed through a facade), so this module ports the
//! *behavior* rather than the class shapes: [`init`] installs a
//! `tracing_subscriber` that writes the same timestamp/level/name format to
//! stdout/stderr and to `<data-dir>/logs/celeste_log_<timestamp>.txt`, using
//! `tracing_appender`'s non-blocking writer as the equivalent of the Java
//! background logging thread. [`normalize_target`] reproduces
//! `ScarletLoggerFactory::getLogger`'s name rewriting for parity with the old
//! log file naming scheme.

use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Logger names from before the rename, still rewritten to the current
/// root so a mixed run's log lines group together.
const LEGACY_LOG_ROOT: &str = "Scarlet";
const LEGACY_LOG_ROOT_2: &str = "Celeste";
const LOG_ROOT: &str = "KozyBlake/Celeste";
const LEGACY_PACKAGE_PREFIX: &str = "net.sybyline.scarlet.";

/// What gets logged when `CELESTE_LOG` says nothing.
///
/// This used to be a bare `debug`, which is a *global* level: every
/// dependency logged at debug too. Celeste holds a Discord gateway
/// connection open and polls the VRChat API continuously, and at debug
/// level serenity records every gateway dispatch while hyper, h2 and
/// rustls record every request and TLS handshake. On a machine left
/// running for a day that produced multi-gigabyte log files in which
/// Celeste's own lines were a rounding error.
///
/// Celeste's own crates stay at debug, because that is what makes a bug
/// report useful. Everything else reports what went wrong, not what it
/// did. `CELESTE_LOG=debug` still turns the firehose back on.
const DEFAULT_LOG_FILTER: &str = concat!(
    "info",
    ",celeste=debug",
    ",celeste_core=debug",
    ",celeste_dave=debug",
    ",celeste_discord=debug",
    ",celeste_events=debug",
    ",celeste_gif=debug",
    ",celeste_ui=debug",
    ",celeste_vrchat=debug",
    ",serenity=warn",
    ",songbird=warn",
    ",tungstenite=warn",
    ",tokio_tungstenite=warn",
    ",hyper=warn",
    ",hyper_util=warn",
    ",h2=warn",
    ",reqwest=warn",
    ",rustls=warn",
    // The graphics stack logs per *frame*, not per event, so it outweighs
    // everything else combined. `wgpu_core` reports each acquired and
    // released swapchain texture at debug and - the reason a global
    // `info` is not enough on its own - `Device::maintain` at info, once
    // every frame.
    ",wgpu=warn",
    ",wgpu_core=warn",
    ",wgpu_hal=warn",
    ",egui_wgpu=warn",
    ",naga=warn",
);

/// How many of this user's log files to keep. Celeste writes one per run
/// and is expected to stay open for days, so without a cap the folder
/// grows for the life of the install.
const LOG_FILES_KEPT: usize = 10;

/// Port of `ScarletLoggerFactory.getLogger`'s name normalization.
///
/// Kept for parity with the Java log output / grep habits even though Rust
/// module paths (`celeste_discord::commands::...`) don't naturally collide
/// with the Java package names this was written to disambiguate.
pub fn normalize_target(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("net.dv8tion.jda.api.") {
        format!("JDA/{}", rest.replace('.', "/"))
    } else if let Some(rest) = name.strip_prefix("net.dv8tion.jda.internal.") {
        format!("JDA-I/{}", rest.replace('.', "/"))
    } else if name == LEGACY_LOG_ROOT || name == LEGACY_LOG_ROOT_2 {
        LOG_ROOT.to_string()
    } else if let Some(rest) = name.strip_prefix(&format!("{LEGACY_LOG_ROOT}/")) {
        format!("{LOG_ROOT}/{rest}")
    } else if let Some(rest) = name.strip_prefix(LEGACY_PACKAGE_PREFIX) {
        format!("{LOG_ROOT}/{}", rest.replace('.', "/"))
    } else {
        name.to_string()
    }
}

/// Returns the path this run's log file will be (or was) written to, mirroring
/// `ScarletLogger.getActiveLogFile()`. `data_dir` is the equivalent of
/// `Scarlet.dir`.
pub fn log_file_path(data_dir: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    data_dir.join("logs").join(format!("celeste_log_{stamp}.txt"))
}

/// Initializes global tracing: stdout/stderr plus the per-run log file.
///
/// The returned [`WorkerGuard`] must be kept alive for the lifetime of the
/// process (it flushes the non-blocking file writer on drop) - this is the
/// Rust equivalent of the Java shutdown hook that flushes `logFileStream`.
pub fn init(data_dir: &Path) -> anyhow::Result<WorkerGuard> {
    let log_file = log_file_path(data_dir);
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    // Before this run adds another file, drop the oldest ones. Failing to
    // prune is never a reason not to start.
    if let Some(parent) = log_file.parent() {
        prune_old_logs(parent, LOG_FILES_KEPT);
    }

    let filter_directive = std::env::var("CELESTE_LOG")
        .ok()
        .or_else(|| std::env::var("SCARLET_LOG").ok())
        .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_string());
    let filter = EnvFilter::try_new(filter_directive)
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true);
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_target(true);

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(guard)
}

/// Keeps the newest `keep` `celeste_log_*.txt` files and deletes the rest.
///
/// Names carry a sortable `%Y-%m-%d_%H-%M-%S` stamp, so ordering by name
/// orders by age without stat-ing every file. Legacy `scarlet_log_*`
/// files count too: they are the same thing under the old name.
fn prune_old_logs(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_log_file(path))
        .collect();
    if logs.len() <= keep {
        return;
    }
    logs.sort();
    let doomed = logs.len() - keep;
    for path in logs.into_iter().take(doomed) {
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::debug!(path = %path.display(), "Removed an old Celeste log"),
            Err(error) => {
                tracing::debug!(path = %path.display(), %error, "Could not remove an old log")
            }
        }
    }
}

fn is_log_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".txt")
        && ["celeste_log_", "scarlet_log_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_filter_does_not_put_every_dependency_at_debug() {
        // The regression this guards: the default was a bare "debug",
        // which is a *global* level. Celeste holds a Discord gateway open
        // and polls the VRChat API constantly, so serenity, hyper, h2 and
        // rustls each logged every event, request and handshake. A day's
        // run produced a multi-gigabyte file Windows would not open.
        EnvFilter::try_new(DEFAULT_LOG_FILTER).expect("the default filter must parse");
        let directives: Vec<&str> = DEFAULT_LOG_FILTER.split(',').collect();

        let global = directives
            .iter()
            .find(|directive| !directive.contains('='))
            .expect("the filter needs a global level");
        assert_eq!(
            *global, "info",
            "the global level must not be debug or trace: that is every dependency"
        );

        for noisy in [
            "serenity",
            "songbird",
            "hyper",
            "h2",
            "reqwest",
            "rustls",
            "tungstenite",
            // Per-frame rather than per-event, and the observed cause of a
            // 2 GB log: wgpu_core writes several lines for every frame
            // presented, one of them at info rather than debug.
            "wgpu",
            "wgpu_core",
            "wgpu_hal",
            "naga",
        ] {
            let directive = directives
                .iter()
                .find(|directive| directive.starts_with(&format!("{noisy}=")))
                .unwrap_or_else(|| panic!("{noisy} writes a lot and is not capped"));
            assert!(
                directive.ends_with("=warn") || directive.ends_with("=error"),
                "{directive} should report problems, not activity"
            );
        }

        // Celeste's own crates stay at debug - that is what makes a bug
        // report from a tester worth reading.
        for own in ["celeste_ui=debug", "celeste_discord=debug", "celeste_events=debug"] {
            assert!(directives.contains(&own), "{own} missing from the default filter");
        }
    }

    #[test]
    fn only_the_newest_log_files_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for hour in 0..15 {
            std::fs::write(
                dir.path().join(format!("celeste_log_2026-01-01_{hour:02}-00-00.txt")),
                "x",
            )
            .unwrap();
        }
        // Something that is not one of ours must survive untouched.
        std::fs::write(dir.path().join("notes.txt"), "keep me").unwrap();

        prune_old_logs(dir.path(), 10);

        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();

        assert!(left.contains(&"notes.txt".to_string()), "pruned an unrelated file");
        let logs: Vec<&String> = left.iter().filter(|name| name.starts_with("celeste_log_")).collect();
        assert_eq!(logs.len(), 10, "kept {} logs, expected 10", logs.len());
        // The five oldest went; the newest stayed.
        assert!(!left.iter().any(|name| name.contains("_04-00-00")));
        assert!(left.iter().any(|name| name.contains("_05-00-00")));
        assert!(left.iter().any(|name| name.contains("_14-00-00")));
    }

    #[test]
    fn pruning_a_folder_with_few_logs_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for hour in 0..3 {
            std::fs::write(
                dir.path().join(format!("celeste_log_2026-01-01_{hour:02}-00-00.txt")),
                "x",
            )
            .unwrap();
        }
        prune_old_logs(dir.path(), 10);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
        // A folder that does not exist is not an error either.
        prune_old_logs(&dir.path().join("nope"), 10);
    }

    #[test]
    fn normalizes_jda_and_legacy_targets() {
        assert_eq!(normalize_target("net.dv8tion.jda.api.JDA"), "JDA/JDA");
        assert_eq!(
            normalize_target("net.dv8tion.jda.internal.JDAImpl"),
            "JDA-I/JDAImpl"
        );
        assert_eq!(normalize_target("Celeste"), "KozyBlake/Celeste");
        // A logger name from before the rename maps to the same root.
        assert_eq!(normalize_target("Scarlet"), "KozyBlake/Celeste");
        // A Java class name still maps under the current root, so log
        // lines from ported and not-yet-ported subsystems group together.
        assert_eq!(
            normalize_target("net.sybyline.scarlet.ScarletSettings"),
            "KozyBlake/Celeste/ScarletSettings"
        );
        assert_eq!(normalize_target("some.other.Thing"), "some.other.Thing");
    }
}
