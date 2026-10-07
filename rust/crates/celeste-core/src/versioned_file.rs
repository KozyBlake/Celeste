//! Port of `net.sybyline.scarlet.util.VersionedFile`.
//!
//! VRChat addresses an uploaded file as an id plus a version, and sometimes
//! a qualifier (`security`, `standard`), in a handful of different
//! spellings: `file_<uuid>/3`, `file_<uuid>:3`, `file_<uuid>/3/security`.
//! This parses any of them out of a larger string — an asset-bundle URL, or
//! the `[API] ... analysis/file_.../3` line VRChat writes to its own log,
//! which is where Celeste gets exact avatar performance ratings from.

use std::sync::OnceLock;

pub const DEFAULT_FILE_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionedFile {
    pub id: String,
    pub version: i32,
    /// Empty when absent, matching Java (which normalises null/blank to "").
    pub qualifier: String,
}

fn pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    // Java: file_\w{8}-\w{4}-\w{4}-\w{4}-\w{12} then [/:] version, then an
    // optional [/:] qualifier.
    PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(?P<id>file_\w{8}-\w{4}-\w{4}-\w{4}-\w{12})[/:](?P<version>-?\d+)([/:](?P<qualifier>[\-\?\&/\w]+))?",
        )
        .expect("static pattern")
    })
}

impl VersionedFile {
    /// Port of `VersionedFile.parse`: finds the first versioned-file
    /// reference anywhere in `value`.
    pub fn parse(value: &str) -> Option<VersionedFile> {
        let captures = pattern().captures(value)?;
        Some(VersionedFile {
            id: captures.name("id")?.as_str().to_string(),
            // Java's `parseIntElse(.., 1)`.
            version: captures
                .name("version")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(DEFAULT_FILE_VERSION),
            qualifier: captures
                .name("qualifier")
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default(),
        })
    }

    pub fn of(id: impl Into<String>) -> VersionedFile {
        VersionedFile {
            id: id.into(),
            version: DEFAULT_FILE_VERSION,
            qualifier: String::new(),
        }
    }

    /// Port of `toString(char)`.
    pub fn to_string_with(&self, delimiter: char) -> String {
        if self.qualifier.is_empty() {
            format!("{}{delimiter}{}", self.id, self.version)
        } else {
            format!(
                "{}{delimiter}{}{delimiter}{}",
                self.id, self.version, self.qualifier
            )
        }
    }

    /// Port of `replaceInto`.
    pub fn replace_into(&self, template: &str) -> String {
        template
            .replace("{fileId}", &self.id)
            .replace("{fileVersion}", &self.version.to_string())
            .replace("{fileQualifier}", &self.qualifier)
    }
}

impl std::fmt::Display for VersionedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string_with('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_spelling_vrchat_uses() {
        let id = "file_aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        assert_eq!(
            VersionedFile::parse(&format!("{id}/3")),
            Some(VersionedFile {
                id: id.to_string(),
                version: 3,
                qualifier: String::new()
            })
        );
        assert_eq!(
            VersionedFile::parse(&format!("{id}:7")),
            Some(VersionedFile {
                id: id.to_string(),
                version: 7,
                qualifier: String::new()
            })
        );
        assert_eq!(
            VersionedFile::parse(&format!("{id}/3/security")),
            Some(VersionedFile {
                id: id.to_string(),
                version: 3,
                qualifier: "security".to_string()
            })
        );
    }

    /// The real shape it has to find: the analysis request VRChat logs.
    #[test]
    fn parses_out_of_a_log_line_url() {
        let url =
            "https://api.vrchat.cloud/api/1/analysis/file_aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/4";
        let parsed = VersionedFile::parse(url).unwrap();
        assert_eq!(parsed.id, "file_aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(parsed.version, 4);
    }

    /// And out of an avatar's asset-bundle URL, which is where the
    /// API-fallback path gets it.
    #[test]
    fn parses_out_of_an_asset_bundle_url() {
        let url =
            "https://api.vrchat.cloud/api/1/file/file_aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/12/file";
        let parsed = VersionedFile::parse(url).unwrap();
        assert_eq!(parsed.version, 12);
        // `file` is picked up as the qualifier, exactly as Java's pattern
        // does — the analysis call only special-cases "security"/"standard",
        // so anything else behaves like no qualifier there.
        assert_eq!(parsed.qualifier, "file");
    }

    #[test]
    fn a_string_with_no_file_reference_is_none() {
        assert_eq!(VersionedFile::parse("no file here"), None);
        assert_eq!(VersionedFile::parse(""), None);
    }

    #[test]
    fn renders_with_the_delimiter_java_uses_for_cache_keys() {
        let file = VersionedFile {
            id: "file_x".to_string(),
            version: 2,
            qualifier: "security".to_string(),
        };
        assert_eq!(file.to_string_with('#'), "file_x#2#security");
        assert_eq!(file.to_string(), "file_x/2/security");
        assert_eq!(VersionedFile::of("file_y").to_string(), "file_y/1");
    }
}
