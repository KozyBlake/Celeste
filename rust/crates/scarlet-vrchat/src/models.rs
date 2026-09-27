//! A small, hand-picked subset of the VRChat API's data model.
//!
//! Java gets this for free from the generated `vrchatapi-java` client
//! (an OpenAPI-generated library covering the whole API surface). Rust has
//! no equivalent generated client in this workspace, and hand-writing an
//! equivalent for every endpoint isn't attempted here -- these are the
//! shapes `scarlet-vrchat`'s currently-ported endpoints actually need,
//! extended as more endpoints are ported. `#[serde(default)]`/`Option`
//! throughout so an unrecognized or missing field never breaks
//! deserialization, matching Gson's default leniency in the Java version.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CurrentUser {
    pub id: String,
    pub username: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "currentAvatarImageUrl")]
    pub current_avatar_image_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "userIcon")]
    pub user_icon: Option<String>,
    #[serde(rename = "currentAvatarImageUrl")]
    pub current_avatar_image_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A hand-picked subset of `io.github.vrchatapi.model.GroupAuditLogEntry`'s
/// fields -- the ones `ScarletData.AuditEntryMetadata` and the audit-log
/// polling loop actually read. `data` carries whatever event-specific
/// payload VRChat attaches (join/leave/role-change/... shape varies by
/// `event_type`), same role as `getData()` in the Java model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupAuditLogEntry {
    pub id: String,
    #[serde(rename = "groupId")]
    pub group_id: String,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "actorId")]
    pub actor_id: Option<String>,
    #[serde(rename = "actorDisplayName")]
    pub actor_display_name: Option<String>,
    #[serde(rename = "targetId")]
    pub target_id: Option<String>,
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub description: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PaginatedGroupAuditLogEntryList {
    pub results: Vec<GroupAuditLogEntry>,
    #[serde(rename = "totalCount")]
    pub total_count: i64,
    #[serde(rename = "hasNext")]
    pub has_next: bool,
}
