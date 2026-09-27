//! Port target: `ScarletVRChat`'s `ApiClient` setup and `login()`/`refresh()`/
//! `auditQuery*` methods (3,367 lines total; only the auth flow and the
//! group-audit-log endpoints are ported so far -- see PORTING.md).
//!
//! ## Scope decision: no generated API client
//!
//! Java gets full VRChat API coverage for free from `vrchatapi-java`, an
//! OpenAPI-generated client. There's no equivalent in this Rust workspace
//! (generating one is a separate, large undertaking -- the spec covers
//! hundreds of endpoints), so this hand-writes the handful of endpoints
//! Scarlet's ported features actually need, growing incrementally rather
//! than attempting full coverage up front.
//!
//! ## Not ported yet: persistent encrypted cookie storage
//!
//! `ScarletVRChatCookieJar` persists the session cookie into the encrypted
//! settings store so a restart doesn't require re-authenticating. This
//! client only keeps cookies in memory via `reqwest`'s built-in cookie
//! jar (`cookie_store(true)`), which is correct for the lifetime of one
//! run but means every restart re-logs-in. Wiring persistence through
//! `scarlet_core::secure_store` is a real, tracked gap (see PORTING.md),
//! not a silent omission -- it needs a serializable cookie jar (e.g. the
//! `reqwest_cookie_store` crate) that this skeleton doesn't pull in yet.

use reqwest::{Client, StatusCode};
use serde_json::Value;

use crate::basic_auth::basic_auth_header;
use crate::models::{CurrentUser, PaginatedGroupAuditLogEntryList};

/// Encrypted-registry keys for the primary VRChat account, matching
/// `ScarletVRChat`'s own domain (`new ScarletVRChat(this, "global", ...)`
/// in `Scarlet.java`) so a data directory carried over from a Java install
/// round-trips. Shared here (rather than duplicated as string literals in
/// `scarlet-ui`'s setup wizard and this module's unattended-login helper)
/// so the two never drift apart.
pub const USERNAME_KEY: &str = "global:username";
pub const PASSWORD_KEY: &str = "global:password";

/// Port of `ScarletVRChat`'s `vrchat_group_id` settings key (the "which
/// group does Scarlet moderate" config, asked for via
/// `getStringOrRequireInput("vrchat_group_id", ...)` in its constructor).
/// Stored plain (not encrypted) in `settings.json`, matching Java's use of
/// the plain `getString`/`setString` for this key rather than
/// `RegistryStringEncrypted` -- a group id isn't a secret.
pub const GROUP_ID_KEY: &str = "vrchat_group_id";

/// VRChat `GroupPermissions` string values actually used so far, confirmed
/// against `components/schemas/GroupPermissions.yaml` (kebab-case strings,
/// not the Java enum's snake_case variant names — `group_all`'s real wire
/// value is the wildcard `"*"`, not `"group-all"`, which would have been an
/// easy wrong guess).
pub const PERMISSION_ALL: &str = "*";
pub const PERMISSION_GROUP_BANS_MANAGE: &str = "group-bans-manage";
pub const PERMISSION_GROUP_MEMBERS_VIEWALL: &str = "group-members-viewall";
pub const PERMISSION_GROUP_INVITES_MANAGE: &str = "group-invites-manage";
pub const PERMISSION_GROUP_ANNOUNCEMENT_MANAGE: &str = "group-announcement-manage";

#[derive(Debug, thiserror::Error)]
pub enum VrChatError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("VRChat API returned HTTP {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("unexpected response shape: {0}")]
    UnexpectedShape(String),
}

#[derive(Debug, Clone)]
pub enum LoginOutcome {
    LoggedIn(CurrentUser),
    RequiresTwoFactor(Vec<String>),
}

pub struct VrChatClient {
    http: Client,
    base_url: String,
}

impl VrChatClient {
    /// Builds a client against the real VRChat API
    /// (`https://api.vrchat.com/api/1`, matching `Scarlet.API_BASE_1`).
    pub fn new() -> Result<Self, VrChatError> {
        Self::with_base_url(&scarlet_core::identity::api_base(scarlet_core::identity::API_HOST_1))
    }

    /// Builds a client against an arbitrary base URL — used to point at a
    /// local mock server in tests.
    pub fn with_base_url(base_url: &str) -> Result<Self, VrChatError> {
        let http = Client::builder()
            .cookie_store(true)
            .user_agent(scarlet_core::identity::user_agent())
            .build()?;
        Ok(VrChatClient { http, base_url: base_url.trim_end_matches('/').to_string() })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Port of the Basic-Auth branch of `ScarletVRChat.login()`: calls
    /// `GET /auth/user` with VRChat's non-standard Basic Auth encoding
    /// (see `basic_auth`), and distinguishes "logged in" from "needs 2FA"
    /// by inspecting the response shape (a `requiresTwoFactorAuth` array vs
    /// a full user object), same as the Java version's `data.has("id")`
    /// check.
    pub async fn login_with_credentials(&self, username: &str, password: &str) -> Result<LoginOutcome, VrChatError> {
        let response = self
            .http
            .get(self.url("/auth/user"))
            .header("Authorization", basic_auth_header(username, password))
            .send()
            .await?;
        self.parse_login_response(response).await
    }

    /// Re-checks the current (cookie-based) session, matching
    /// `AuthenticationApi.getCurrentUser` used after `verifyAuthToken`.
    pub async fn get_current_user(&self) -> Result<CurrentUser, VrChatError> {
        let response = self.http.get(self.url("/auth/user")).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))
    }

    async fn parse_login_response(&self, response: reqwest::Response) -> Result<LoginOutcome, VrChatError> {
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let value: Value = serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        if let Some(methods) = value.get("requiresTwoFactorAuth").and_then(|v| v.as_array()) {
            let methods = methods.iter().filter_map(|m| m.as_str().map(str::to_string)).collect();
            return Ok(LoginOutcome::RequiresTwoFactor(methods));
        }
        let user: CurrentUser =
            serde_json::from_value(value).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        Ok(LoginOutcome::LoggedIn(user))
    }

    /// Port of the TOTP branch of `login()`/`loginAlt()`: `POST
    /// /auth/twofactorauth/totp/verify`, returning whether the code was
    /// accepted (`getVerified()` in Java).
    pub async fn verify_totp(&self, code: &str) -> Result<bool, VrChatError> {
        let response = self
            .http
            .post(self.url("/auth/twofactorauth/totp/verify"))
            .json(&serde_json::json!({ "code": code }))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let value: Value = serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        Ok(value.get("verified").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Port of `getGroupAuditLogsRaw`/one page of `auditQuery`: `GET
    /// /groups/{groupId}/auditLogs`.
    pub async fn get_group_audit_logs(
        &self,
        group_id: &str,
        n: u32,
        offset: u32,
    ) -> Result<PaginatedGroupAuditLogEntryList, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/auditLogs")))
            .query(&[("n", n.to_string()), ("offset", offset.to_string())])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getUser(String userId)`: `GET /users/{userId}`.
    pub async fn get_user(&self, user_id: &str) -> Result<crate::models::User, VrChatError> {
        let response = self.http.get(self.url(&format!("/users/{user_id}"))).send().await?;
        Self::parse_response(response).await
    }

    // ---- groups (moderation surface) ----
    //
    // Every endpoint below has its exact path, method, and request/response
    // shape confirmed against the community-maintained OpenAPI spec
    // (github.com/vrchatapi/specification) rather than guessed -- VRChat's
    // API has no official documentation, and getting these wrong silently
    // (a mismatched field name deserializing to `None`/default instead of
    // erroring) would be a much worse failure mode than not having the
    // endpoint at all.

    /// Port of `getGroup(String groupId, Boolean includeRoles)`: `GET
    /// /groups/{groupId}[?includeRoles=true]`.
    pub async fn get_group(&self, group_id: &str, include_roles: bool) -> Result<crate::models::Group, VrChatError> {
        let mut request = self.http.get(self.url(&format!("/groups/{group_id}")));
        if include_roles {
            request = request.query(&[("includeRoles", "true")]);
        }
        Self::parse_response(request.send().await?).await
    }

    /// Port of `getGroupRoles(String groupId)`: `GET /groups/{groupId}/roles`.
    pub async fn get_group_roles(&self, group_id: &str) -> Result<Vec<crate::models::GroupRole>, VrChatError> {
        let response = self.http.get(self.url(&format!("/groups/{group_id}/roles"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupBans(String groupId, int limit)`: `GET
    /// /groups/{groupId}/bans`.
    pub async fn get_group_bans(&self, group_id: &str, n: u32, offset: u32) -> Result<Vec<crate::models::GroupMember>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/bans")))
            .query(&[("n", n.to_string()), ("offset", offset.to_string())])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `banFromGroup(String targetUserId)`: `POST
    /// /groups/{groupId}/bans` with `{"userId": ...}`.
    pub async fn ban_from_group(&self, group_id: &str, user_id: &str) -> Result<crate::models::GroupMember, VrChatError> {
        let response = self
            .http
            .post(self.url(&format!("/groups/{group_id}/bans")))
            .json(&serde_json::json!({ "userId": user_id }))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `unbanFromGroup(String targetUserId)`: `DELETE
    /// /groups/{groupId}/bans/{userId}`.
    pub async fn unban_from_group(&self, group_id: &str, user_id: &str) -> Result<crate::models::GroupMember, VrChatError> {
        let response = self.http.delete(self.url(&format!("/groups/{group_id}/bans/{user_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupMembership(String groupId, String targetUserId)`:
    /// `GET /groups/{groupId}/members/{userId}`. Returns `Ok(None)` (not an
    /// error) when the response body is JSON `null`, matching the spec's
    /// documented "returns a GroupMember, or null when not a member".
    pub async fn get_group_member(&self, group_id: &str, user_id: &str) -> Result<Option<crate::models::GroupMember>, VrChatError> {
        let response = self.http.get(self.url(&format!("/groups/{group_id}/members/{user_id}"))).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let value: Value = serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value).map(Some).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))
    }

    /// Port of `searchGroupMembers(String groupId, String query, int limit)`:
    /// `GET /groups/{groupId}/members/search?query=...`. VRChat requires
    /// `query` to be at least 3 characters (a spec-documented constraint,
    /// not enforced client-side here -- a too-short query is a normal `400`
    /// from the API, surfaced as [`VrChatError::Api`] like any other).
    pub async fn search_group_members(
        &self,
        group_id: &str,
        query: &str,
        n: u32,
        offset: u32,
    ) -> Result<crate::models::GroupMemberSearchResult, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/members/search")))
            .query(&[("query", query), ("n", &n.to_string()), ("offset", &offset.to_string())])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of the kick half of group member management: `DELETE
    /// /groups/{groupId}/members/{userId}`.
    pub async fn kick_group_member(&self, group_id: &str, user_id: &str) -> Result<(), VrChatError> {
        let response = self.http.delete(self.url(&format!("/groups/{group_id}/members/{user_id}"))).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    /// Port of `addGroupRole(String groupId, String targetUserId, String
    /// groupRoleId)`: `PUT /groups/{groupId}/members/{userId}/roles/{roleId}`,
    /// returning the member's full role id list afterward.
    pub async fn add_group_member_role(&self, group_id: &str, user_id: &str, role_id: &str) -> Result<Vec<String>, VrChatError> {
        let response =
            self.http.put(self.url(&format!("/groups/{group_id}/members/{user_id}/roles/{role_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `removeGroupRole(String groupId, String targetUserId, String
    /// groupRoleId)`: `DELETE /groups/{groupId}/members/{userId}/roles/{roleId}`.
    pub async fn remove_group_member_role(&self, group_id: &str, user_id: &str, role_id: &str) -> Result<Vec<String>, VrChatError> {
        let response =
            self.http.delete(self.url(&format!("/groups/{group_id}/members/{user_id}/roles/{role_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupAnnouncement`: `GET /groups/{groupId}/announcement`.
    /// Returns an "empty" `GroupAnnouncement` (see its `is_empty`) rather
    /// than an error when the group has no current announcement, matching
    /// the API's own documented behavior.
    pub async fn get_group_announcement(&self, group_id: &str) -> Result<crate::models::GroupAnnouncement, VrChatError> {
        let response = self.http.get(self.url(&format!("/groups/{group_id}/announcement"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `createGroupAnnouncement`: `POST /groups/{groupId}/announcement`.
    /// Per VRChat's own API docs this **replaces** any existing announcement
    /// (there's only ever at most one).
    pub async fn create_group_announcement(
        &self,
        group_id: &str,
        title: &str,
        text: &str,
        send_notification: bool,
    ) -> Result<crate::models::GroupAnnouncement, VrChatError> {
        let response = self
            .http
            .post(self.url(&format!("/groups/{group_id}/announcement")))
            .json(&serde_json::json!({ "title": title, "text": text, "sendNotification": send_notification }))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `deleteGroupAnnouncement`: `DELETE /groups/{groupId}/announcement`.
    pub async fn delete_group_announcement(&self, group_id: &str) -> Result<(), VrChatError> {
        let response = self.http.delete(self.url(&format!("/groups/{group_id}/announcement"))).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    // ---- worlds / avatars / instances ----

    /// Port of `getWorld(String worldId)`: `GET /worlds/{worldId}`.
    pub async fn get_world(&self, world_id: &str) -> Result<crate::models::World, VrChatError> {
        let response = self.http.get(self.url(&format!("/worlds/{world_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getAvatar(String avatarId)`: `GET /avatars/{avatarId}`.
    pub async fn get_avatar(&self, avatar_id: &str) -> Result<crate::models::Avatar, VrChatError> {
        let response = self.http.get(self.url(&format!("/avatars/{avatar_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getInstance(String worldId, String instanceId)`: `GET
    /// /instances/{worldId}:{instanceId}`.
    pub async fn get_instance(&self, world_id: &str, instance_id: &str) -> Result<crate::models::Instance, VrChatError> {
        let response = self.http.get(self.url(&format!("/instances/{world_id}:{instance_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `closeInstance(String worldId, String instanceId, ...)`:
    /// `DELETE /instances/{worldId}:{instanceId}`. Java also accepts
    /// optional `hardClose`/`closedAt` parameters; not ported here (not
    /// confirmed against the spec yet) -- a plain close is still a real,
    /// working close, just without those two knobs.
    pub async fn close_instance(&self, world_id: &str, instance_id: &str) -> Result<crate::models::Instance, VrChatError> {
        let response = self.http.delete(self.url(&format!("/instances/{world_id}:{instance_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `checkUserHasVRChatPermission(GroupMember, GroupPermissions)`
    /// plus the group-owner fallback from `checkSelfUserHasVRChatPermission`
    /// (`this.currentUserId.equals(this.groupOwnerId)`). **Not ported**: the
    /// `allGroupPermissions` live-event permission cache Java checks first —
    /// that's a separate, not-yet-ported subsystem fed by gateway events;
    /// this always does a fresh `get_group`/`get_group_member` fetch
    /// instead, which is correct, just uncached (an extra round trip per
    /// check rather than a cache hit).
    pub async fn user_has_group_permission(&self, group_id: &str, user_id: &str, permission: &str) -> Result<bool, VrChatError> {
        let group = self.get_group(group_id, true).await?;
        if group.owner_id.as_deref() == Some(user_id) {
            return Ok(true);
        }
        let Some(member) = self.get_group_member(group_id, user_id).await? else {
            return Ok(false);
        };
        let Some(roles) = &group.roles else {
            return Ok(false);
        };
        Ok(roles
            .iter()
            .filter(|role| member.role_ids.contains(&role.id))
            .any(|role| role.permissions.iter().any(|p| p == PERMISSION_ALL || p == permission)))
    }

    async fn parse_response<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T, VrChatError> {
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))
    }
}

/// Port of `ScarletVRChat`'s unattended re-login path: attempts a login
/// using whatever credentials are already stored under [`USERNAME_KEY`]/
/// [`PASSWORD_KEY`], for use at app startup when the interactive setup
/// wizard didn't run this session (it already completed in an earlier one).
///
/// Returns `None` (after logging why) if nothing is stored, the login
/// fails, or VRChat asks for two-factor. **Known gap**: Java's
/// `ScarletVRChat` also stores a TOTP secret (`RegistryStringEncrypted
/// totpsecret`) so an unattended login can generate a valid code itself;
/// this doesn't yet (no TOTP-code-generation dependency pulled in), so an
/// account requiring 2FA can only log in through the interactive wizard,
/// which then hands its already-authenticated client onward instead of
/// calling this at all.
pub async fn login_with_stored_credentials(settings: &scarlet_core::settings::Settings) -> Option<VrChatClient> {
    use scarlet_core::settings::registry::registry_string_encrypted;

    let username = registry_string_encrypted(settings.encrypted.clone(), USERNAME_KEY).get_or_null()?;
    let password = registry_string_encrypted(settings.encrypted.clone(), PASSWORD_KEY).get_or_null()?;

    let client = match VrChatClient::new() {
        Ok(client) => client,
        Err(err) => {
            tracing::error!("Could not build VRChat client for unattended login: {}", err);
            return None;
        }
    };
    match client.login_with_credentials(&username, &password).await {
        Ok(LoginOutcome::LoggedIn(user)) => {
            tracing::info!("VRChat unattended login succeeded as {}", user.display_name.as_deref().unwrap_or(&user.id));
            Some(client)
        }
        Ok(LoginOutcome::RequiresTwoFactor(methods)) => {
            tracing::warn!(
                "VRChat unattended login requires two-factor ({:?}); run the setup wizard interactively to log in",
                methods
            );
            None
        }
        Err(err) => {
            tracing::error!("VRChat unattended login failed: {}", err);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn login_with_stored_credentials_returns_none_when_nothing_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        // No network call should happen at all -- if this test hangs or
        // times out, that's a regression (it means a real request is being
        // attempted against the credential-less path).
        assert!(login_with_stored_credentials(&settings).await.is_none());
    }

    #[tokio::test]
    async fn login_with_credentials_sends_vrchat_style_basic_auth() {
        let server = MockServer::start().await;
        let expected_auth = basic_auth_header("someone", "p&ss");
        Mock::given(method("GET"))
            .and(path("/auth/user"))
            .and(header("Authorization", expected_auth.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_123",
                "username": "someone",
                "displayName": "Someone"
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let outcome = client.login_with_credentials("someone", "p&ss").await.unwrap();
        match outcome {
            LoginOutcome::LoggedIn(user) => assert_eq!(user.id, "usr_123"),
            other => panic!("expected LoggedIn, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn login_detects_two_factor_requirement() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "requiresTwoFactorAuth": ["totp", "otp"]
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let outcome = client.login_with_credentials("someone", "pass").await.unwrap();
        match outcome {
            LoginOutcome::RequiresTwoFactor(methods) => {
                assert_eq!(methods, vec!["totp".to_string(), "otp".to_string()]);
            }
            other => panic!("expected RequiresTwoFactor, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn verify_totp_reports_acceptance() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/twofactorauth/totp/verify"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "verified": true })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(client.verify_totp("123456").await.unwrap());
    }

    #[tokio::test]
    async fn get_group_audit_logs_parses_pagination() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/auditLogs"))
            .and(query_param("n", "50"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{
                    "id": "evt_1",
                    "groupId": "grp_1",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "actorId": "usr_1",
                    "actorDisplayName": "Actor",
                    "targetId": "usr_2",
                    "eventType": "group.instance.kick",
                    "description": "kicked"
                }],
                "totalCount": 1,
                "hasNext": false
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let page = client.get_group_audit_logs("grp_1", 50, 0).await.unwrap();
        assert_eq!(page.total_count, 1);
        assert!(!page.has_next);
        assert_eq!(page.results[0].event_type, "group.instance.kick");
    }

    #[tokio::test]
    async fn api_error_status_is_surfaced() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/user"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid credentials"))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let err = client.login_with_credentials("someone", "wrong").await.unwrap_err();
        match err {
            VrChatError::Api { status, .. } => assert_eq!(status, StatusCode::UNAUTHORIZED),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_group_parses_with_and_without_roles() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1"))
            .and(query_param("includeRoles", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "grp_1",
                "name": "Test Group",
                "memberCount": 5,
                "onlineMemberCount": 2,
                "roles": [{"id": "grol_1", "name": "Member", "permissions": []}]
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let group = client.get_group("grp_1", true).await.unwrap();
        assert_eq!(group.name, "Test Group");
        assert_eq!(group.member_count, 5);
        assert_eq!(group.roles.unwrap()[0].name, "Member");
    }

    #[tokio::test]
    async fn ban_and_unban_round_trip_through_the_bans_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/groups/grp_1/bans"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_1", "groupId": "grp_1", "userId": "usr_2", "roleIds": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/bans/usr_2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_1", "groupId": "grp_1", "userId": "usr_2", "roleIds": []
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let banned = client.ban_from_group("grp_1", "usr_2").await.unwrap();
        assert_eq!(banned.user_id.as_deref(), Some("usr_2"));
        let unbanned = client.unban_from_group("grp_1", "usr_2").await.unwrap();
        assert_eq!(unbanned.user_id.as_deref(), Some("usr_2"));
    }

    #[tokio::test]
    async fn get_group_member_returns_none_for_a_json_null_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/usr_2"))
            .respond_with(ResponseTemplate::new(200).set_body_string("null"))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(client.get_group_member("grp_1", "usr_2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_group_member_parses_a_real_member() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/usr_2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_1", "groupId": "grp_1", "userId": "usr_2", "roleIds": ["grol_1"]
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let member = client.get_group_member("grp_1", "usr_2").await.unwrap().unwrap();
        assert_eq!(member.role_ids, vec!["grol_1".to_string()]);
    }

    #[tokio::test]
    async fn search_group_members_parses_results_and_total() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/search"))
            .and(query_param("query", "ali"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{"id": "gmem_1", "userId": "usr_2", "roleIds": []}],
                "total": 1
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let result = client.search_group_members("grp_1", "ali", 10, 0).await.unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.results[0].user_id.as_deref(), Some("usr_2"));
    }

    #[tokio::test]
    async fn add_and_remove_group_member_role_return_the_role_id_list() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/groups/grp_1/members/usr_2/roles/grol_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(["grol_1", "grol_2"])))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/members/usr_2/roles/grol_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(["grol_2"])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let after_add = client.add_group_member_role("grp_1", "usr_2", "grol_1").await.unwrap();
        assert_eq!(after_add, vec!["grol_1".to_string(), "grol_2".to_string()]);
        let after_remove = client.remove_group_member_role("grp_1", "usr_2", "grol_1").await.unwrap();
        assert_eq!(after_remove, vec!["grol_2".to_string()]);
    }

    #[tokio::test]
    async fn kick_group_member_succeeds_on_a_2xx_response() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/members/usr_2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": {"message": "ok"}})))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        client.kick_group_member("grp_1", "usr_2").await.unwrap();
    }

    #[tokio::test]
    async fn get_world_parses_a_real_world() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/worlds/wrld_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "wrld_1", "name": "Test World", "capacity": 16
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let world = client.get_world("wrld_1").await.unwrap();
        assert_eq!(world.name, "Test World");
        assert_eq!(world.capacity, Some(16));
    }

    #[tokio::test]
    async fn get_avatar_parses_a_real_avatar() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/avatars/avtr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "avtr_1", "name": "Test Avatar"
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let avatar = client.get_avatar("avtr_1").await.unwrap();
        assert_eq!(avatar.name, "Test Avatar");
    }

    #[tokio::test]
    async fn get_and_close_instance_hit_the_colon_delimited_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/instances/wrld_1:12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "worldId": "wrld_1", "instanceId": "12345", "userCount": 3
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/instances/wrld_1:12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "worldId": "wrld_1", "instanceId": "12345", "closed": true
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let instance = client.get_instance("wrld_1", "12345").await.unwrap();
        assert_eq!(instance.user_count, Some(3));
        let closed = client.close_instance("wrld_1", "12345").await.unwrap();
        assert_eq!(closed.instance_id.as_deref(), Some("12345"));
    }

    async fn mock_group_with_roles(server: &MockServer, owner_id: &str) {
        Mock::given(method("GET"))
            .and(path("/groups/grp_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "grp_1", "name": "Test Group", "memberCount": 1, "onlineMemberCount": 0,
                "ownerId": owner_id,
                "roles": [
                    {"id": "grol_mod", "name": "Moderator", "permissions": ["group-bans-manage"]},
                    {"id": "grol_member", "name": "Member", "permissions": []}
                ]
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn group_owner_always_has_permission() {
        let server = MockServer::start().await;
        mock_group_with_roles(&server, "usr_owner").await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(client.user_has_group_permission("grp_1", "usr_owner", PERMISSION_GROUP_BANS_MANAGE).await.unwrap());
    }

    #[tokio::test]
    async fn member_with_the_matching_role_has_permission() {
        let server = MockServer::start().await;
        mock_group_with_roles(&server, "usr_owner").await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/usr_mod"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_1", "groupId": "grp_1", "userId": "usr_mod", "roleIds": ["grol_mod"]
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(client.user_has_group_permission("grp_1", "usr_mod", PERMISSION_GROUP_BANS_MANAGE).await.unwrap());
    }

    #[tokio::test]
    async fn member_without_the_matching_role_lacks_permission() {
        let server = MockServer::start().await;
        mock_group_with_roles(&server, "usr_owner").await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/usr_plain"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_2", "groupId": "grp_1", "userId": "usr_plain", "roleIds": ["grol_member"]
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(!client.user_has_group_permission("grp_1", "usr_plain", PERMISSION_GROUP_BANS_MANAGE).await.unwrap());
    }

    #[tokio::test]
    async fn non_member_lacks_permission() {
        let server = MockServer::start().await;
        mock_group_with_roles(&server, "usr_owner").await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/usr_stranger"))
            .respond_with(ResponseTemplate::new(200).set_body_string("null"))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(!client.user_has_group_permission("grp_1", "usr_stranger", PERMISSION_GROUP_BANS_MANAGE).await.unwrap());
    }

    #[tokio::test]
    async fn get_group_announcement_returns_empty_object_as_is_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/announcement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let announcement = client.get_group_announcement("grp_1").await.unwrap();
        assert!(announcement.is_empty());
    }

    #[tokio::test]
    async fn create_and_delete_group_announcement_round_trip() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/groups/grp_1/announcement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gann_1", "title": "Hello", "text": "World"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/announcement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": {"message": "Group announcement was cleared!", "status_code": 200}
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let created = client.create_group_announcement("grp_1", "Hello", "World", false).await.unwrap();
        assert!(!created.is_empty());
        assert_eq!(created.title.as_deref(), Some("Hello"));
        client.delete_group_announcement("grp_1").await.unwrap();
    }
}
