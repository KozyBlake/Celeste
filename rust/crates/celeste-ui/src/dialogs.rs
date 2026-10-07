//! Runtime-safe native dialogs.
//!
//! Celeste builds `rfd` with the XDG portal backend on Linux. That backend
//! is asynchronous and needs a Tokio reactor even when rfd's synchronous API
//! is called. Calling `FileDialog::pick_*` from egui's main thread therefore
//! panics inside zbus. Drive the async API in a small current-thread runtime
//! instead, and contain a backend panic so a broken portal cannot take down
//! the entire application.

use std::future::Future;
use std::path::{Path, PathBuf};

pub fn pick_folder(
    title: &str,
    directory: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    run(dialog.pick_folder()).map(|picked| picked.map(|folder| folder.path().to_path_buf()))
}

pub fn pick_archive(
    title: &str,
    directory: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .add_filter("Supported archives", celeste_core::legacy_archive::PICKER_EXTENSIONS)
        .add_filter("ZIP archives", &["zip"])
        .add_filter("7-Zip archives", &["7z"])
        .add_filter("TAR archives", &["tar", "tar.gz", "tgz", "tar.bz2", "tbz2", "tar.xz", "txz"]);
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    run(dialog.pick_file()).map(|picked| picked.map(|file| file.path().to_path_buf()))
}

pub fn message(
    title: &str,
    description: impl Into<String>,
    buttons: rfd::MessageButtons,
    level: rfd::MessageLevel,
) -> Result<rfd::MessageDialogResult, String> {
    let dialog = rfd::AsyncMessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(buttons)
        .set_level(level);
    run(dialog.show())
}

pub fn error_message(title: &str, description: impl Into<String>) -> Result<(), String> {
    message(
        title,
        description,
        rfd::MessageButtons::Ok,
        rfd::MessageLevel::Error,
    )
    .map(|_| ())
}

fn run<F, T>(future: F) -> Result<T, String>
where
    F: Future<Output = T>,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not start the native-dialog runtime: {error}"))?;
        Ok::<T, String>(runtime.block_on(future))
    }))
    .map_err(|_| {
        "the operating system's native dialog service failed; Celeste stayed open".to_string()
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_runtime_drives_async_work() {
        assert_eq!(run(async { 42 }).unwrap(), 42);
    }

    #[test]
    fn dialog_runtime_contains_panics() {
        let result = run(async { panic!("simulated portal failure") });
        assert!(result.is_err());
    }
}
