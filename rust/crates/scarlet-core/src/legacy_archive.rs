//! Safe, cross-platform extraction of legacy Scarlet data archives.
//!
//! Archives are untrusted input. Only the small set of files understood by
//! [`crate::legacy_import`] is extracted, archive paths are normalized before
//! use, and count/size limits prevent an accidental archive bomb from filling
//! the user's data drive. No platform archive command is invoked.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const MAX_ENTRIES: usize = 50_000;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

const ROOT_FILES: &[&str] = &[
    "settings.json",
    "staff_list.json",
    "secret_staff_list.json",
    "moderation_tags.json",
    "watched_groups.json",
    "watched_users.json",
    "watched_avatars.json",
    "good_pronoun.json",
    "bad_pronoun.json",
    "global.json",
    "discord_perms.json",
];
const RECORD_DIRS: &[&str] = &["usr", "gaud", "ex"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Zip,
    SevenZip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    parts: Vec<String>,
    size: u64,
}

/// Extensions shown by both native file pickers.
pub const PICKER_EXTENSIONS: &[&str] = &[
    "zip", "7z", "tar", "tar.gz", "tgz", "tar.bz2", "tbz2", "tar.xz", "txz",
];

/// Extract the recognized Scarlet data in `archive` to a private staging
/// directory beneath the Celeste data folder. The caller queues that folder
/// for import and asks [`cleanup_staging`] to remove it after the restart.
pub fn prepare(archive: &Path, data_dir: &Path) -> Result<PathBuf> {
    let kind = detect_kind(archive)?;
    anyhow::ensure!(
        archive.is_file(),
        "{} is not a readable archive file",
        archive.display()
    );
    let entries = list_entries(archive, kind)?;
    let root = find_root(&entries)?;
    let selected = select_entries(&entries, &root)?;

    let staging_parent = data_dir.join(".legacy-import-staging");
    std::fs::create_dir_all(&staging_parent)?;
    let staging = staging_parent.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&staging)?;

    let result = extract_entries(archive, kind, &staging, &selected);
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    anyhow::ensure!(
        crate::paths::settings_file_usable(&staging),
        "the archive's Scarlet folder does not contain a readable, non-empty settings.json"
    );
    Ok(staging)
}

/// Remove only a directory that Celeste itself created for archive import.
/// This deliberately refuses arbitrary paths.
pub fn cleanup_staging(path: &Path, data_dir: &Path) {
    let Some(parent) = path.parent() else { return };
    if parent == data_dir.join(".legacy-import-staging") {
        let _ = std::fs::remove_dir_all(path);
        if std::fs::read_dir(parent)
            .ok()
            .is_some_and(|mut entries| entries.next().is_none())
        {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn detect_kind(path: &Path) -> Result<Kind> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Kind::TarGz
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        Kind::TarBz2
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        Kind::TarXz
    } else if name.ends_with(".tar") {
        Kind::Tar
    } else if name.ends_with(".zip") {
        Kind::Zip
    } else if name.ends_with(".7z") {
        Kind::SevenZip
    } else {
        bail!("unsupported archive type; choose ZIP, 7Z, TAR, TAR.GZ, TAR.BZ2, or TAR.XZ")
    };
    Ok(kind)
}

fn normalize(name: &str) -> Option<Vec<String>> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.contains('\0') {
        return None;
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return None;
        }
        parts.push(part.to_string());
    }
    (!parts.is_empty()).then_some(parts)
}

fn recognized(relative: &[String]) -> bool {
    (relative.len() == 1 && ROOT_FILES.contains(&relative[0].as_str()))
        || (relative.len() == 2
            && RECORD_DIRS.contains(&relative[0].as_str())
            && !relative[1].is_empty())
}

fn find_root(entries: &[Entry]) -> Result<Vec<String>> {
    let mut candidates = entries
        .iter()
        .filter(|entry| {
            entry
                .parts
                .last()
                .is_some_and(|part| part == "settings.json")
        })
        .map(|entry| {
            let root = entry.parts[..entry.parts.len() - 1].to_vec();
            let score = entries
                .iter()
                .filter(|candidate| candidate.parts.starts_with(&root))
                .filter(|candidate| recognized(&candidate.parts[root.len()..]))
                .count();
            (score, root)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(score_a, root_a), (score_b, root_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| root_a.len().cmp(&root_b.len()))
    });
    candidates
        .into_iter()
        .next()
        .map(|(_, root)| root)
        .context("the archive does not contain a Scarlet settings.json")
}

fn select_entries(entries: &[Entry], root: &[String]) -> Result<HashMap<String, PathBuf>> {
    let mut selected = HashMap::new();
    let mut destinations = HashSet::new();
    let mut total = 0u64;
    for entry in entries {
        if !entry.parts.starts_with(root) {
            continue;
        }
        let relative = &entry.parts[root.len()..];
        if !recognized(relative) {
            continue;
        }
        anyhow::ensure!(
            entry.size <= MAX_FILE_BYTES,
            "{} is too large to import safely",
            entry.name
        );
        total = total
            .checked_add(entry.size)
            .context("archive size overflow")?;
        anyhow::ensure!(
            total <= MAX_TOTAL_BYTES,
            "recognized archive data exceeds the 1 GiB safety limit"
        );
        let destination = relative.iter().collect::<PathBuf>();
        anyhow::ensure!(
            destinations.insert(destination.clone()),
            "archive contains duplicate data path {}",
            destination.display()
        );
        selected.insert(entry.name.clone(), destination);
    }
    anyhow::ensure!(
        selected
            .values()
            .any(|path| path == Path::new("settings.json")),
        "the archive does not contain a usable Scarlet root"
    );
    Ok(selected)
}

fn list_entries(path: &Path, kind: Kind) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    match kind {
        Kind::Zip => {
            let mut archive = zip::ZipArchive::new(File::open(path)?)?;
            anyhow::ensure!(
                archive.len() <= MAX_ENTRIES,
                "archive contains too many entries"
            );
            for index in 0..archive.len() {
                let file = archive.by_index(index)?;
                if file.is_file() {
                    push_entry(&mut entries, file.name(), file.size())?;
                }
            }
        }
        Kind::SevenZip => {
            sevenz_rust::decompress_file_with_extract_fn(
                path,
                std::env::temp_dir(),
                |entry, _reader, _| {
                    if !entry.is_directory() {
                        push_entry(&mut entries, entry.name(), entry.size())
                            .map_err(sevenz_rust::Error::io)?;
                    }
                    Ok(true)
                },
            )
            .map_err(|error| anyhow::anyhow!("could not read 7Z archive: {error}"))?;
        }
        _ => {
            let reader = tar_reader(path, kind)?;
            let mut archive = tar::Archive::new(reader);
            for item in archive.entries()? {
                let item = item?;
                if item.header().entry_type().is_file() {
                    let name = item.path()?.to_string_lossy().to_string();
                    push_entry(&mut entries, &name, item.size())?;
                }
            }
        }
    }
    anyhow::ensure!(
        entries.len() <= MAX_ENTRIES,
        "archive contains too many entries"
    );
    Ok(entries)
}

fn push_entry(entries: &mut Vec<Entry>, name: &str, size: u64) -> std::io::Result<()> {
    if entries.len() >= MAX_ENTRIES {
        return Err(std::io::Error::other("archive contains too many entries"));
    }
    if let Some(parts) = normalize(name) {
        entries.push(Entry {
            name: name.to_string(),
            parts,
            size,
        });
    }
    Ok(())
}

fn extract_entries(
    path: &Path,
    kind: Kind,
    destination: &Path,
    selected: &HashMap<String, PathBuf>,
) -> Result<()> {
    match kind {
        Kind::Zip => {
            let mut archive = zip::ZipArchive::new(File::open(path)?)?;
            for index in 0..archive.len() {
                let mut source = archive.by_index(index)?;
                if let Some(relative) = selected.get(source.name()) {
                    let size = source.size();
                    write_limited(&mut source, &destination.join(relative), size)?;
                }
            }
        }
        Kind::SevenZip => {
            sevenz_rust::decompress_file_with_extract_fn(path, destination, |entry, reader, _| {
                if let Some(relative) = selected.get(entry.name()) {
                    write_limited(reader, &destination.join(relative), entry.size())
                        .map_err(sevenz_rust::Error::io)?;
                }
                Ok(true)
            })
            .map_err(|error| anyhow::anyhow!("could not extract 7Z archive: {error}"))?;
        }
        _ => {
            let reader = tar_reader(path, kind)?;
            let mut archive = tar::Archive::new(reader);
            for item in archive.entries()? {
                let mut item = item?;
                if !item.header().entry_type().is_file() {
                    continue;
                }
                let name = item.path()?.to_string_lossy().to_string();
                if let Some(relative) = selected.get(&name) {
                    let size = item.size();
                    write_limited(&mut item, &destination.join(relative), size)?;
                }
            }
        }
    }
    Ok(())
}

fn write_limited(reader: &mut dyn Read, path: &Path, expected: u64) -> std::io::Result<()> {
    if expected > MAX_FILE_BYTES {
        return Err(std::io::Error::other(
            "archive entry exceeds the per-file safety limit",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    let copied = std::io::copy(&mut reader.take(MAX_FILE_BYTES + 1), &mut file)?;
    file.flush()?;
    if copied > MAX_FILE_BYTES {
        let _ = std::fs::remove_file(path);
        return Err(std::io::Error::other(
            "archive entry expanded beyond the safety limit",
        ));
    }
    Ok(())
}

fn tar_reader(path: &Path, kind: Kind) -> Result<Box<dyn Read>> {
    let file = File::open(path)?;
    let reader: Box<dyn Read> = match kind {
        Kind::Tar => Box::new(file),
        Kind::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        Kind::TarBz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        Kind::TarXz => Box::new(lzma_rust2::XzReader::new_mem_limit(file, true, 256 * 1024)),
        _ => unreachable!(),
    };
    Ok(reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizer_rejects_traversal_and_windows_prefixes() {
        assert!(normalize("../settings.json").is_none());
        assert!(normalize("C:/settings.json").is_none());
        assert_eq!(
            normalize("Scarlet\\usr\\usr_1.json").unwrap(),
            ["Scarlet", "usr", "usr_1.json"]
        );
    }

    #[test]
    fn nested_scarlet_root_is_selected() {
        let entries = vec![
            Entry {
                name: "export/Scarlet/settings.json".into(),
                parts: normalize("export/Scarlet/settings.json").unwrap(),
                size: 2,
            },
            Entry {
                name: "export/Scarlet/watched_groups.json".into(),
                parts: normalize("export/Scarlet/watched_groups.json").unwrap(),
                size: 2,
            },
        ];
        assert_eq!(find_root(&entries).unwrap(), ["export", "Scarlet"]);
    }

    #[test]
    fn unsupported_formats_have_a_clear_error() {
        let error = detect_kind(Path::new("scarlet.rar"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported archive type"));
    }

    #[test]
    fn tar_archive_extracts_only_recognized_data_from_a_nested_root() {
        let workspace = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let nested = source.path().join("export").join("Scarlet");
        std::fs::create_dir_all(nested.join("usr")).unwrap();
        std::fs::write(nested.join("settings.json"), r#"{"legacy":true}"#).unwrap();
        std::fs::write(nested.join("watched_groups.json"), "[]").unwrap();
        std::fs::write(nested.join("usr").join("usr_1.json"), r#"{"id":"usr_1"}"#).unwrap();
        std::fs::write(nested.join("not-imported.txt"), "private unrelated data").unwrap();

        let archive_path = workspace.path().join("scarlet.tar");
        let file = File::create(&archive_path).unwrap();
        let mut builder = tar::Builder::new(file);
        builder
            .append_dir_all("export", source.path().join("export"))
            .unwrap();
        builder.finish().unwrap();

        let destination = workspace.path().join("celeste");
        let staged = prepare(&archive_path, &destination).unwrap();
        assert!(staged.join("settings.json").is_file());
        assert!(staged.join("watched_groups.json").is_file());
        assert!(staged.join("usr").join("usr_1.json").is_file());
        assert!(!staged.join("not-imported.txt").exists());

        cleanup_staging(&staged, &destination);
        assert!(!staged.exists());
    }

    #[test]
    fn zip_archive_is_supported_without_allowing_path_traversal() {
        let workspace = tempfile::tempdir().unwrap();
        let archive_path = workspace.path().join("scarlet.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive_path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("Scarlet/settings.json", options).unwrap();
        writer.write_all(br#"{"legacy":true}"#).unwrap();
        writer
            .start_file("Scarlet/staff_list.json", options)
            .unwrap();
        writer.write_all(br#"["usr_old"]"#).unwrap();
        writer
            .start_file("Scarlet/../../escaped.json", options)
            .unwrap();
        writer.write_all(b"not allowed").unwrap();
        writer.finish().unwrap();

        let destination = workspace.path().join("celeste");
        let staged = prepare(&archive_path, &destination).unwrap();
        assert!(staged.join("settings.json").is_file());
        assert!(staged.join("staff_list.json").is_file());
        assert!(!workspace.path().join("escaped.json").exists());
    }
}
