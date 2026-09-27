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

/// Hand-picked subset of `io.github.vrchatapi.model.Group`, field names and
/// shapes confirmed against the community-maintained OpenAPI spec
/// (github.com/vrchatapi/specification, `components/schemas/Group.yaml`)
/// rather than guessed, since VRChat's API isn't officially documented.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "shortCode")]
    pub short_code: Option<String>,
    pub discriminator: Option<String>,
    #[serde(rename = "ownerId")]
    pub owner_id: Option<String>,
    #[serde(rename = "memberCount")]
    pub member_count: i64,
    #[serde(rename = "onlineMemberCount")]
    pub online_member_count: i64,
    #[serde(rename = "iconUrl")]
    pub icon_url: Option<String>,
    #[serde(rename = "bannerUrl")]
    pub banner_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Only present when `getGroup` is called with `includeRoles=true`.
    pub roles: Option<Vec<GroupRole>>,
}

/// Confirmed against `components/schemas/GroupRole.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupRole {
    pub id: String,
    #[serde(rename = "groupId")]
    pub group_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(rename = "isSelfAssignable", default)]
    pub is_self_assignable: bool,
    #[serde(rename = "isManagementRole", default)]
    pub is_management_role: bool,
    #[serde(rename = "isAddedOnJoin", default)]
    pub is_added_on_join: bool,
    pub order: Option<i64>,
}

/// Confirmed against `components/schemas/GroupMember.yaml`. VRChat's own
/// schema notes several fields as "only missing when explicitly fetching
/// own user" (`Option` throughout handles that, matching the general
/// lenient-deserialization approach this module already takes).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupMember {
    pub id: Option<String>,
    #[serde(rename = "groupId")]
    pub group_id: Option<String>,
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    #[serde(rename = "roleIds", default)]
    pub role_ids: Vec<String>,
    #[serde(rename = "managerNotes")]
    pub manager_notes: Option<String>,
    #[serde(rename = "membershipStatus")]
    pub membership_status: Option<String>,
    #[serde(rename = "isRepresenting", default)]
    pub is_representing: bool,
    #[serde(rename = "joinedAt")]
    pub joined_at: Option<String>,
    #[serde(rename = "bannedAt")]
    pub banned_at: Option<String>,
    pub visibility: Option<String>,
}

/// Confirmed against `components/responses/groups/GroupMemberSearchResponse.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GroupMemberSearchResult {
    pub results: Vec<GroupMember>,
    pub total: i64,
}

/// Hand-picked subset of `World`, confirmed against
/// `components/schemas/World.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct World {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "authorId")]
    pub author_id: Option<String>,
    #[serde(rename = "authorName")]
    pub author_name: Option<String>,
    pub capacity: Option<i64>,
    #[serde(rename = "recommendedCapacity")]
    pub recommended_capacity: Option<i64>,
    #[serde(rename = "imageUrl")]
    pub image_url: Option<String>,
    #[serde(rename = "thumbnailImageUrl")]
    pub thumbnail_image_url: Option<String>,
    #[serde(rename = "releaseStatus")]
    pub release_status: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub visits: Option<i64>,
    pub popularity: Option<i64>,
    pub heat: Option<i64>,
    pub favorites: Option<i64>,
}

/// Hand-picked subset of `Avatar`, confirmed against
/// `components/schemas/Avatar.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Avatar {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "authorId")]
    pub author_id: Option<String>,
    #[serde(rename = "authorName")]
    pub author_name: Option<String>,
    #[serde(rename = "imageUrl")]
    pub image_url: Option<String>,
    #[serde(rename = "thumbnailImageUrl")]
    pub thumbnail_image_url: Option<String>,
    #[serde(rename = "releaseStatus")]
    pub release_status: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Confirmed against `components/schemas/GroupAnnouncement.yaml`. Every
/// field is optional in the schema itself -- `GET .../announcement`
/// documents returning an **empty object** when no announcement has been
/// made, rather than `null` or a 404, so callers check `title`/`text` (or
/// just `id`) to tell "no announcement" from a real one, not `Option`-ness
/// of the whole struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupAnnouncement {
    pub id: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    #[serde(rename = "authorId")]
    pub author_id: Option<String>,
    #[serde(rename = "imageUrl")]
    pub image_url: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

impl GroupAnnouncement {
    /// The API returns an empty object (all fields absent/null) rather
    /// than a 404 when there's no current announcement.
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.title.is_none() && self.text.is_none()
    }
}

/// Confirmed against `components/schemas/GroupPost.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupPost {
    pub id: Option<String>,
    #[serde(rename = "groupId")]
    pub group_id: Option<String>,
    #[serde(rename = "authorId")]
    pub author_id: Option<String>,
    #[serde(rename = "editorId")]
    pub editor_id: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    #[serde(rename = "imageId")]
    pub image_id: Option<String>,
    #[serde(rename = "imageUrl")]
    pub image_url: Option<String>,
    #[serde(rename = "roleIds", default)]
    pub role_ids: Vec<String>,
    pub visibility: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

/// Confirmed against `components/responses/groups/GroupPostsResponse.yaml`
/// -- `GET .../posts` wraps its results in `{posts, total}` rather than
/// returning a bare array (unlike bans/invites/join-requests).
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct GroupPostsResponse {
    pub posts: Vec<GroupPost>,
    #[allow(dead_code)]
    pub total: i64,
}

/// Confirmed against `components/schemas/GroupTransferable.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupTransferable {
    pub requirements: Option<GroupTransferableRequirements>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupTransferableRequirements {
    #[serde(rename = "groupNotMonetized", default)]
    pub group_not_monetized: bool,
    #[serde(rename = "hasVRCPlus", default)]
    pub has_vrc_plus: bool,
    #[serde(rename = "hasVerifiedEmail", default)]
    pub has_verified_email: bool,
    #[serde(rename = "targetCanOwnMoreGroups", default)]
    pub target_can_own_more_groups: bool,
    #[serde(rename = "targetIsGroupMember", default)]
    pub target_is_group_member: bool,
}

/// Hand-picked subset of `Instance`. Not confirmed field-by-field against
/// the spec (unlike the models above) -- kept intentionally small and
/// `Option`-heavy since this is the shape `get_instance`/`close_instance`
/// return, not yet cross-checked the way the group/world/avatar models were.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Instance {
    pub id: Option<String>,
    #[serde(rename = "worldId")]
    pub world_id: Option<String>,
    #[serde(rename = "instanceId")]
    pub instance_id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "userCount")]
    pub user_count: Option<i64>,
    pub capacity: Option<i64>,
    #[serde(rename = "ownerId")]
    pub owner_id: Option<String>,
    pub closed: Option<serde_json::Value>,
    pub full: Option<bool>,
}
