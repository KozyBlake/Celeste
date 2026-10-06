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

/// Resolves the GUI startup folder, offering a safe copy-based migration
/// when Celeste discovers an existing Java Scarlet installation. Headless
/// callers intentionally continue using `resolve_data_dir` directly.
pub fn choose_startup_data_dir(resolved: &ResolvedDataDir) -> PathBuf {
    let default_dir = scarlet_core::paths::default_data_dir();
    match &resolved.source {
        DataDirSource::Discovered => choose_detected_migration(&resolved.path, &default_dir),
        DataDirSource::FirstRun => choose_first_run(&default_dir),
        _ => resolved.path.clone(),
    }
}

fn choose_detected_migration(detected: &Path, default_dir: &Path) -> PathBuf {
    let result = rfd::MessageDialog::new()
        .set_title("Celeste - Scarlet data found")
        .set_description(format!(
            "Celeste found an existing Scarlet data folder:\n\n{}\n\n\
             Import its settings, moderation tags, watched lists and history into:\n\n{}\n\n\
             The Scarlet folder will not be changed. Encrypted Java credentials cannot be imported.\n\n\
             Yes: import this folder\nNo: choose a different Scarlet folder\nCancel: start Celeste without importing",
            detected.display(),
            default_dir.display(),
        ))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();

    match result {
        rfd::MessageDialogResult::Yes => import_legacy_folder(detected, default_dir),
        rfd::MessageDialogResult::No => choose_legacy_folder(default_dir).map_or_else(
            || default_dir.to_path_buf(),
            |source| import_legacy_folder(&source, default_dir),
        ),
        _ => default_dir.to_path_buf(),
    }
}

fn choose_first_run(default_dir: &Path) -> PathBuf {
    let import = rfd::MessageDialog::new()
        .set_title("Celeste - import Scarlet data")
        .set_description(
            "Do you have an existing Scarlet data folder to import?\n\n\
             Celeste can transfer settings, moderation tags, watched lists and history while leaving Scarlet untouched.\n\n\
             Java-encrypted credentials cannot be imported and will be requested during setup.",
        )
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if import == rfd::MessageDialogResult::Yes {
        if let Some(source) = choose_legacy_folder(default_dir) {
            return import_legacy_folder(&source, default_dir);
        }
    }
    ask_where_to_store_data(default_dir)
}

fn choose_legacy_folder(default_dir: &Path) -> Option<PathBuf> {
    let selected = rfd::FileDialog::new()
        .set_title("Choose the Scarlet data folder")
        .set_directory(default_dir.parent().unwrap_or(default_dir))
        .pick_folder()?;
    if scarlet_core::paths::settings_file_usable(&selected) {
        return Some(selected);
    }
    rfd::MessageDialog::new()
        .set_title("Celeste - not a Scarlet data folder")
        .set_description(format!(
            "{} does not contain a readable, non-empty settings.json. Nothing was imported.",
            selected.display(),
        ))
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    None
}

fn import_legacy_folder(source: &Path, destination: &Path) -> PathBuf {
    if source == destination {
        return destination.to_path_buf();
    }
    match scarlet_core::paths::copy_data_dir(source, destination) {
        Ok(copied) => {
            rfd::MessageDialog::new()
                .set_title("Celeste - Scarlet data imported")
                .set_description(format!(
                    "Imported {copied} file(s) into:\n{}\n\nThe original folder remains untouched. Celeste setup will now request any credentials that could not be transferred.",
                    destination.display(),
                ))
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            destination.to_path_buf()
        }
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("Celeste - import failed")
                .set_description(format!(
                    "Celeste could not copy the Scarlet data: {error}\n\nThe original folder was not changed. Celeste will use it directly for this run so the data is still available.",
                ))
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            source.to_path_buf()
        }
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

    let use_default = rfd::MessageDialog::new()
        .set_title("Celeste - data folder")
        .set_description(&message)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();

    if use_default == rfd::MessageDialogResult::Yes {
        // The platform default needs no pointer file: it is what resolution
        // finds on its own.
        return default_dir.to_path_buf();
    }

    let picked = rfd::FileDialog::new()
        .set_title("Choose a folder for Celeste's data")
        .set_directory(default_dir.parent().unwrap_or(default_dir))
        .pick_folder();

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
