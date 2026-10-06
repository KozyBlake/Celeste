//! The first-run "where should Celeste keep its data?" question.
//!
//! Java never asks: its data directory is derived from the platform and
//! that's that. Asking is a deliberate addition — the repo owner runs test
//! builds from several directories, and wants to decide where a given
//! install writes rather than have it dump `settings.json`, `secure-prefs.json`,
//! `logs/` and `backups/` wherever the binary was launched from.
//!
//! It only ever runs when there is no data directory yet, no `SCARLET_HOME`,
//! nothing remembered from a previous run, and no existing config found by
//! discovery — so an established install is never interrupted, and a
//! scripted/headless run never blocks on a dialog.
//!
//! This uses a native dialog rather than an egui screen because it has to
//! happen *before* the settings store is opened (opening it is what creates
//! the files), and therefore before the main window's event loop exists.

use std::path::{Path, PathBuf};

use scarlet_core::paths::{DataDirSource, ResolvedDataDir};

/// Resolves the GUI startup folder. Legacy migration is offered inside the
/// first-run setup window after the app opens; only storage location belongs
/// in this pre-window native prompt.
pub fn choose_startup_data_dir(resolved: &ResolvedDataDir) -> PathBuf {
    let default_dir = scarlet_core::paths::default_data_dir();
    match &resolved.source {
        // Discovery found Java Scarlet, not an established Celeste folder.
        // Start Celeste in its own location so the setup wizard can offer an
        // additive import without ever treating Java settings as live.
        DataDirSource::Discovered => default_dir,
        DataDirSource::FirstRun => ask_where_to_store_data(&default_dir),
        _ => resolved.path.clone(),
    }
}

/// Asks where to store data, remembers the answer for future runs, and
/// returns the directory to use. Falls back to `default_dir` if the person
/// dismisses the dialog.
pub fn ask_where_to_store_data(default_dir: &Path) -> PathBuf {
    let message = format!(
        "Celeste hasn't been set up on this machine yet.\n\n\
         By default it keeps its settings, logs and backups in:\n{}\n\n\
         Use that folder?\n\n\
         (Choose No to pick a different one. You can move it later by \
         setting CELESTE_HOME, or from Settings.)",
        default_dir.display()
    );

    let use_default = match crate::dialogs::message(
        "Celeste - data folder",
        &message,
        rfd::MessageButtons::YesNo,
        rfd::MessageLevel::Info,
    ) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(%error, "Could not open the first-run data-folder question");
            return default_dir.to_path_buf();
        }
    };

    if use_default == rfd::MessageDialogResult::Yes {
        // The platform default needs no pointer file: it is what resolution
        // finds on its own.
        return default_dir.to_path_buf();
    }

    let picked = match crate::dialogs::pick_folder(
        "Choose a folder for Celeste's data",
        Some(default_dir.parent().unwrap_or(default_dir)),
    ) {
        Ok(picked) => picked,
        Err(error) => {
            tracing::error!(%error, "Could not open the first-run data-folder picker");
            None
        }
    };

    match picked {
        Some(chosen) => {
            if let Err(err) = scarlet_core::paths::remember_data_dir(&chosen) {
                // Not fatal: this run still uses the chosen folder, the next
                // one just won't know about it.
                tracing::warn!("Could not remember the chosen data folder: {err}");
            }
            tracing::info!("Data folder chosen by the user: {}", chosen.display());
            chosen
        }
        None => {
            tracing::info!("Data folder choice cancelled; using the default");
            default_dir.to_path_buf()
        }
    }
}
