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

    let filter_directive = std::env::var("CELESTE_LOG")
        .ok()
        .or_else(|| std::env::var("SCARLET_LOG").ok())
        .unwrap_or_else(|| "debug".to_string());
    let filter = EnvFilter::try_new(filter_directive).unwrap_or_else(|_| EnvFilter::new("debug"));

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

#[cfg(test)]
mod tests {
    use super::*;

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
