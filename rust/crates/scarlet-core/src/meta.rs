//! Port of `net.sybyline.scarlet.ScarletMeta` and `ScarletAnnouncement`:
//! the POJOs for the `meta.json` file hosted at
//! `https://raw.githubusercontent.com/<fork>/<repo>/main/meta.json`
//! (see [`crate::identity::META_URL`]).
//!
//! `latest_release`/`latest_build` drive the update-check flow. `announcement`
//! is an optional broadcast notice: when present (and not expired) it pops a
//! dialog on every running Celeste instance on the next hourly poll.
//!
//! Both types derive `Deserialize`/`Serialize` with `#[serde(default)]` on
//! every optional field, matching Gson's leniency about unknown/missing JSON
//! keys (older builds silently ignore fields they don't know about; a build
//! polling an older `meta.json` just sees `None`/empty defaults instead of a
//! deserialize error).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScarletMeta {
    #[serde(default)]
    pub latest_release: Option<String>,
    #[serde(default)]
    pub latest_build: Option<String>,
    /// Optional broadcast announcement; `None` when nothing to broadcast.
    #[serde(default)]
    pub announcement: Option<ScarletAnnouncement>,
}

/// Maintainers edit this sub-object on the repo's `main` branch to push an
/// out-of-band notice to every running Celeste instance. `id` is the dedup
/// key: once a user has acknowledged a given id, they won't be re-prompted
/// for that announcement on the next poll; bump the id to broadcast a new
/// one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScarletAnnouncement {
    /// Stable identifier for this announcement; used to dedup re-prompts.
    pub id: String,
    /// One of "info" (default), "warning", "urgent" - drives the dialog icon.
    #[serde(default)]
    pub severity: Option<String>,
    /// Short headline shown as the dialog title; falls back to "Celeste announcement".
    #[serde(default)]
    pub title: Option<String>,
    /// Body text shown in the dialog. If empty, no popup is shown.
    #[serde(default)]
    pub message: Option<String>,
    /// Optional web URL; Celeste only opens http/https announcement links.
    #[serde(default)]
    pub url: Option<String>,
    /// Optional ISO-8601 timestamp. If present and in the past, the
    /// announcement is ignored.
    #[serde(default)]
    pub expires: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_meta_json_shape_from_the_repo() {
        let json = r#"{
            "latest_release": "0.4.20",
            "latest_build": "0.4.20",
            "announcement": {
                "id": "20260926-000000-0420-tagging",
                "severity": "warning",
                "title": "0.4.20 fix",
                "message": "details",
                "expires": "2026-10-26T00:00:00Z"
            }
        }"#;
        let meta: ScarletMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.latest_release.as_deref(), Some("0.4.20"));
        let ann = meta.announcement.unwrap();
        assert_eq!(ann.id, "20260926-000000-0420-tagging");
        assert_eq!(ann.severity.as_deref(), Some("warning"));
        assert!(ann.url.is_none());
    }

    #[test]
    fn tolerates_missing_announcement_and_unknown_fields() {
        let json = r#"{"latest_release": "0.4.20", "latest_build": "0.4.20", "some_future_field": 42}"#;
        let meta: ScarletMeta = serde_json::from_str(json).unwrap();
        assert!(meta.announcement.is_none());
    }
}
