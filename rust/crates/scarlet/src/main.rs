//! Rust port of `net.sybyline.scarlet.Main` / `Scarlet.main`.
//! Status: stub. See ../../PORTING.md for the real entry-point wiring still
//! to be ported (data-dir resolution, settings load, executors, subsystem
//! startup order, restart/update exit-code handling).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let data_dir = std::env::var_os("SCARLET_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let _log_guard = scarlet_core::log::init(&data_dir)?;

    tracing::info!("App: {} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION);
    tracing::warn!("scarlet (Rust) is an in-progress port; see rust/PORTING.md for status");

    Ok(())
}
