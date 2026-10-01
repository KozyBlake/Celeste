//! Generates `SETTINGS.md` from the live settings registry.
//!
//! The hand-written `SETTINGS.md` this replaces had drifted: it still
//! documented settings that no longer exist, still told people their data
//! lived in `KozyBlake\Scarlet`, and had no way of noticing either. A
//! reference for 100+ settings maintained by hand is a reference that is
//! wrong, so this builds it from [`SettingsRegistry::all`] and the same
//! [`super::help::describe`] copy the desktop shows.
//!
//! [`render`] is the whole generator. The test at the bottom is a snapshot
//! test: it fails when the committed file no longer matches what the code
//! would produce, and rewrites it when run with `UPDATE_SETTINGS_MD=1`:
//!
//! ```sh
//! UPDATE_SETTINGS_MD=1 cargo test -p scarlet-core settings_md_is_current
//! ```
//!
//! So adding a setting without documenting it is a test failure, not a
//! thing someone notices eighteen months later.

use super::file_valued::{SettingCategory, SettingKind, SettingDescriptor};
use std::sync::Arc;

/// Markdown for one setting's type and permitted values.
fn kind_cell(kind: &SettingKind) -> String {
    match kind {
        SettingKind::Basic => "text".to_string(),
        SettingKind::Boolean { default } => format!("boolean (default `{default}`)"),
        SettingKind::IntegerRange { default, min, max } => {
            format!("integer `{min}`–`{max}` (default `{default}`)")
        }
        SettingKind::Enum { default, variants } => {
            let list = variants.iter().map(|v| format!("`{v}`")).collect::<Vec<_>>().join(", ");
            format!("one of {list} (default `{default}`)")
        }
        SettingKind::StringChoice { .. } => "one of a list discovered at runtime".to_string(),
        SettingKind::StringPattern { pattern, .. } => match pattern {
            Some(p) => format!("text matching `{p}`"),
            None => "text".to_string(),
        },
        SettingKind::StringArrayPattern { pattern, .. } => match pattern {
            Some(p) => format!("list of text, each matching `{p}`"),
            None => "list of text".to_string(),
        },
        SettingKind::Void { .. } => "action (no stored value)".to_string(),
    }
}

/// Escapes the pipes that would otherwise break out of a Markdown table
/// cell. Several patterns are alternations and contain them.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

pub fn render(entries: &[Arc<dyn SettingDescriptor>]) -> String {
    let mut out = String::new();
    out.push_str(
        "# Celeste settings reference\n\
         \n\
         **This file is generated.** It is built from the settings registry by\n\
         `crates/scarlet-core/src/settings/doc.rs`, and a test fails if it falls out\n\
         of date. To regenerate after adding or changing a setting:\n\
         \n\
         ```sh\n\
         cd rust && UPDATE_SETTINGS_MD=1 cargo test -p scarlet-core settings_md_is_current\n\
         ```\n\
         \n\
         Edit the description in `settings/help.rs`, not this file.\n\
         \n\
         ## Where settings live\n\
         \n\
         | | |\n\
         |---|---|\n\
         | Linux | `$XDG_DATA_HOME/KozyBlake/Celeste/settings.json`, or `~/.local/share/KozyBlake/Celeste/settings.json` |\n\
         | Windows | `%LOCALAPPDATA%\\KozyBlake\\Celeste\\settings.json` |\n\
         \n\
         Celeste also discovers the legacy `KozyBlake/Scarlet` locations and adopts an\n\
         existing install rather than starting empty. `CELESTE_HOME` (or the legacy\n\
         `SCARLET_HOME`) overrides the folder for one run, and **Settings → System**\n\
         moves it permanently.\n\
         \n\
         Everything below is editable in the desktop app under its category heading,\n\
         or from Discord with `/settings edit`.\n",
    );

    for category in SettingCategory::ALL {
        let mut in_category: Vec<&Arc<dyn SettingDescriptor>> =
            entries.iter().filter(|e| e.category() == category).collect();
        if in_category.is_empty() {
            continue;
        }
        in_category.sort_by(|a, b| a.id().cmp(b.id()));

        out.push_str(&format!("\n## {}\n\n", category.label()));
        out.push_str("| Setting | Id | Type | What it does |\n|---|---|---|---|\n");
        for entry in in_category {
            let description = super::help::describe(entry.id()).unwrap_or("");
            out.push_str(&format!(
                "| {} | `{}` | {} | {} |\n",
                cell(entry.name()),
                entry.id(),
                cell(&kind_cell(entry.kind())),
                cell(description),
            ));
        }
    }

    out.push_str(&format!("\n---\n\n{} settings in {} categories.\n", entries.len(), SettingCategory::ALL.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repo-root `SETTINGS.md`, relative to this crate. Test-only: the
    /// build itself never reads outside `rust/`.
    fn settings_md() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../SETTINGS.md")
    }

    #[test]
    fn settings_md_is_current() {
        let dir = std::env::temp_dir().join(format!(
            "celeste-settings-doc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = super::super::Settings::open(&dir);
        let generated = render(&settings.registry.all());
        let _ = std::fs::remove_dir_all(&dir);

        let path = settings_md();
        if std::env::var("UPDATE_SETTINGS_MD").is_ok() {
            std::fs::write(&path, &generated).expect("write SETTINGS.md");
            return;
        }

        let Ok(committed) = std::fs::read_to_string(&path) else {
            // Building the crate outside the repo (vendored, packaged) is
            // not a failure; there is simply nothing to compare against.
            return;
        };

        assert_eq!(
            committed, generated,
            "SETTINGS.md is out of date with the settings registry.\n\
             Regenerate it:\n  \
             cd rust && UPDATE_SETTINGS_MD=1 cargo test -p scarlet-core settings_md_is_current"
        );
    }

    #[test]
    fn every_category_renders_a_table() {
        let dir = std::env::temp_dir().join(format!("celeste-doc-cat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = super::super::Settings::open(&dir);
        let markdown = render(&settings.registry.all());
        let _ = std::fs::remove_dir_all(&dir);
        for category in SettingCategory::ALL {
            assert!(
                markdown.contains(&format!("\n## {}\n", category.label())),
                "category {} produced no section",
                category.label()
            );
        }
    }

    /// A pattern containing `|` used to break the table it was rendered
    /// into, silently turning one row into malformed Markdown.
    #[test]
    fn pipes_in_a_pattern_do_not_break_the_table() {
        assert_eq!(cell("(a|b)"), "(a\\|b)");
    }
}
