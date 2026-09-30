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
//! Celeste's ported features actually need, growing incrementally rather
//! than attempting full coverage up front.
//!
//! ## Persistent cookie storage
//!
//! `ScarletVRChatCookieJar` keeps the session in the encrypted settings
//! store under `<domain>.cookieStore` (`global.cookieStore` for the primary
//! account) as one `Set-Cookie` line per cookie, and reloads it at startup.
//! That is what makes two-factor a once-per-expiry event rather than
//! once-per-launch: VRChat's `auth` cookie lasts a year, and the
//! `twoFactorAuth` cookie issued after a verified code is what stops VRChat
//! asking again.
//!
//! This client does the same, through the same key and the same line
//! format, so a session established by either build works in the other:
//! see [`VrChatClient::with_cookies`], [`VrChatClient::export_cookies`] and
//! [`load_session`]/[`save_session`]. Alt-account jars
//! (`<domain>.cookieStoreAlt`) are not ported, matching the rest of the
//! alt-account support.

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::sync::Arc;

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
/// Encrypted Base32 authenticator seed used by Java for unattended TOTP.
/// This is a reusable setup secret, never a six-digit one-time code.
pub const TOTP_SECRET_KEY: &str = "global:totpsecret";

/// Port of `ScarletVRChat`'s `vrchat_group_id` settings key (the "which
/// group does Celeste moderate" config, asked for via
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
pub const PERMISSION_GROUP_AUDIT_VIEW: &str = "group-audit-view";
pub const PERMISSION_GROUP_MEMBERS_VIEWALL: &str = "group-members-viewall";
pub const PERMISSION_GROUP_INVITES_MANAGE: &str = "group-invites-manage";
pub const PERMISSION_GROUP_ANNOUNCEMENT_MANAGE: &str = "group-announcement-manage";
pub const PERMISSION_GROUP_INSTANCE_MANAGE: &str = "group-instance-manage";
pub const PERMISSION_GROUP_INSTANCE_PUBLIC_CREATE: &str = "group-instance-public-create";
pub const PERMISSION_GROUP_INSTANCE_PLUS_CREATE: &str = "group-instance-plus-create";
pub const PERMISSION_GROUP_INSTANCE_OPEN_CREATE: &str = "group-instance-open-create";
pub const PERMISSION_GROUP_INSTANCE_RESTRICTED_CREATE: &str = "group-instance-restricted-create";
pub const PERMISSION_GROUP_INSTANCE_AGE_GATED_CREATE: &str = "group-instance-age-gated-create";
pub const PERMISSION_GROUP_INSTANCE_MODERATE: &str = "group-instance-moderate";

#[derive(Debug, thiserror::Error)]
pub enum VrChatError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("VRChat API returned HTTP {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("unexpected response shape: {0}")]
    UnexpectedShape(String),
}

impl VrChatError {
    /// Session probes only treat an explicit 401 as an authentication
    /// verdict. Rate limits, server errors, malformed responses and network
    /// failures are inconclusive and must not discard a usable cookie jar.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, VrChatError::Api { status, .. } if *status == StatusCode::UNAUTHORIZED)
    }
}

#[derive(Debug, Clone)]
pub enum LoginOutcome {
    LoggedIn(CurrentUser),
    RequiresTwoFactor(Vec<String>),
}

pub struct VrChatClient {
    http: Client,
    base_url: String,
    /// The per-entity JSON caches `ScarletVRChat` keeps (`cachedUsers`,
    /// `cachedGroups`, ...). `None` means "no caching at all": every lookup
    /// fetches and nothing is recorded, which is what the wiremock tests
    /// want and what a client built before the data directory is known
    /// gets. See [`crate::entity_cache`] and
    /// [`VrChatClient::with_entity_caches`].
    caches: Option<Arc<crate::entity_cache::EntityCaches>>,
    /// The live cookie jar, kept so the session (VRChat's `auth` cookie and
    /// the `twoFactorAuth` cookie that follows a verified code) can be saved
    /// and restored across restarts — see [`VrChatClient::export_cookies`].
    cookies: Arc<reqwest_cookie_store::CookieStoreMutex>,
}

impl VrChatClient {
    /// Builds a client against VRChat's canonical API host.
    ///
    /// `api.vrchat.com` redirects to `api.vrchat.cloud`. Because that is a
    /// cross-host redirect, reqwest correctly removes the sensitive
    /// `Authorization` header before following it, causing credential logins
    /// to fail with `401 Missing Credentials`. Start on the canonical host so
    /// the Basic Auth header reaches `/auth/user` intact. This also matches the
    /// Java client's use of `Scarlet.API_BASE_2`.
    pub fn new() -> Result<Self, VrChatError> {
        Self::with_base_url(&scarlet_core::identity::api_base(
            scarlet_core::identity::API_HOST_2,
        ))
    }

    /// [`VrChatClient::new`] with a restored cookie jar.
    pub fn with_cookies(saved_cookies: Option<&str>) -> Result<Self, VrChatError> {
        Self::with_base_url_and_cookies(
            &scarlet_core::identity::api_base(scarlet_core::identity::API_HOST_2),
            saved_cookies,
        )
    }

    /// Builds a client against an arbitrary base URL — used to point at a
    /// local mock server in tests.
    pub fn with_base_url(base_url: &str) -> Result<Self, VrChatError> {
        Self::with_base_url_and_cookies(base_url, None)
    }

    /// Builds a client whose cookie jar starts from `saved_cookies`, in the
    /// `Set-Cookie`-per-line format [`VrChatClient::export_cookies`]
    /// produces — the same format Java's `ScarletVRChatCookieJar` keeps in
    /// the encrypted store under `global.cookieStore`, so a session
    /// established by either build is usable by the other.
    ///
    /// Restoring the jar is what makes two-factor a once-per-expiry event
    /// rather than a once-per-launch one: VRChat's `auth` cookie is good for
    /// a year, and the `twoFactorAuth` cookie it issues after a verified
    /// code is what tells VRChat not to ask again.
    pub fn with_base_url_and_cookies(
        base_url: &str,
        saved_cookies: Option<&str>,
    ) -> Result<Self, VrChatError> {
        let base_url = base_url.trim_end_matches('/').to_string();
        let mut store = reqwest_cookie_store::CookieStore::new(None);
        if let Some(saved) = saved_cookies {
            if let Ok(url) = reqwest::Url::parse(&base_url) {
                for line in saved.lines().map(str::trim).filter(|line| !line.is_empty()) {
                    if let Err(err) = store.parse(line, &url) {
                        tracing::debug!("Ignoring unparseable saved cookie: {err}");
                    }
                }
            }
        }
        let cookies = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(store));
        let http = Client::builder()
            .cookie_provider(cookies.clone())
            .user_agent(scarlet_core::identity::user_agent())
            .build()?;
        Ok(VrChatClient {
            http,
            base_url,
            // Java's caches are a static shared by every `ScarletVRChat`,
            // so a client picks them up rather than being handed them.
            caches: crate::entity_cache::shared(),
            cookies,
        })
    }

    /// Attaches Java's on-disk entity caches, rooted at `caches_root`
    /// (`<data dir>/caches`, matching `new File(Scarlet.dir, "caches")`).
    /// Without this the client always fetches, so the application calls it
    /// once the data directory is resolved.
    pub fn with_entity_caches(mut self, caches_root: &std::path::Path) -> Self {
        self.caches = Some(Arc::new(crate::entity_cache::EntityCaches::new(caches_root)));
        self
    }

    /// Whether entity caching is active, for the status line and tests.
    pub fn has_entity_caches(&self) -> bool {
        self.caches.is_some()
    }

    /// The current session as `Set-Cookie` lines, one per cookie, for
    /// persisting. Expired cookies are dropped, matching Java's
    /// `removeStale` on save.
    pub fn export_cookies(&self) -> String {
        let store = self.cookies.lock().expect("cookie store poisoned");
        let mut out = String::new();
        for cookie in store.iter_unexpired() {
            // `cookie.to_string()` on a `cookie_store` cookie renders the
            // `name=value; Attr=...` Set-Cookie form Java's okhttp
            // `Cookie.toString()` also produces.
            out.push_str(&cookie.to_string());
            out.push('\n');
        }
        out
    }

    /// Whether the jar currently holds VRChat's session cookie at all. A
    /// client with no `auth` cookie can skip straight to a credential login
    /// instead of making a doomed request.
    pub fn has_session_cookie(&self) -> bool {
        let store = self.cookies.lock().expect("cookie store poisoned");
        let has_auth = store.iter_unexpired().any(|cookie| cookie.name() == "auth");
        has_auth
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
    pub async fn login_with_credentials(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginOutcome, VrChatError> {
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

    /// Port of `AuthenticationApi.logout()` used by Java's CLI `logout`
    /// command. VRChat invalidates the current cookie-backed session.
    pub async fn logout(&self) -> Result<(), VrChatError> {
        let response = self.http.put(self.url("/logout")).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    /// Returns VRChat's clock, used for TOTP generation so a machine with a
    /// slightly incorrect local clock can still sign in (matching Java's
    /// `MiscellaneousApi.getSystemTime()`).
    pub async fn get_server_time(&self) -> Result<DateTime<Utc>, VrChatError> {
        let response = self.http.get(self.url("/time")).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let timestamp: String = serde_json::from_str(&body)
            .map_err(|error| VrChatError::UnexpectedShape(error.to_string()))?;
        DateTime::parse_from_rfc3339(&timestamp)
            .map(|time| time.with_timezone(&Utc))
            .map_err(|error| VrChatError::UnexpectedShape(error.to_string()))
    }

    async fn parse_login_response(
        &self,
        response: reqwest::Response,
    ) -> Result<LoginOutcome, VrChatError> {
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let value: Value =
            serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        if let Some(methods) = value
            .get("requiresTwoFactorAuth")
            .and_then(|v| v.as_array())
        {
            let methods = methods
                .iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect();
            return Ok(LoginOutcome::RequiresTwoFactor(methods));
        }
        let user: CurrentUser = serde_json::from_value(value)
            .map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
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
        Self::parse_two_factor_response(response).await
    }

    /// Port of the email-OTP branch of `login()`/`loginAlt()`: `POST
    /// /auth/twofactorauth/emailotp/verify`, returning whether the emailed
    /// code was accepted.
    pub async fn verify_email_otp(&self, code: &str) -> Result<bool, VrChatError> {
        let response = self
            .http
            .post(self.url("/auth/twofactorauth/emailotp/verify"))
            .json(&serde_json::json!({ "code": code }))
            .send()
            .await?;
        Self::parse_two_factor_response(response).await
    }

    async fn parse_two_factor_response(response: reqwest::Response) -> Result<bool, VrChatError> {
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let value: Value =
            serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        Ok(value
            .get("verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// Port of `getGroupAuditLogsRaw`/one page of `auditQuery`: `GET
    /// /groups/{groupId}/auditLogs`.
    pub async fn get_group_audit_logs(
        &self,
        group_id: &str,
        n: u32,
        offset: u32,
    ) -> Result<PaginatedGroupAuditLogEntryList, VrChatError> {
        self.get_group_audit_logs_filtered(group_id, n, offset, None, None, None, None, None)
            .await
    }

    /// Filtered one-page form of `getGroupAuditLogs`, using the same query
    /// names as VRChat's generated Java client. Comma-separated id/type
    /// filters are passed through unchanged because the API accepts multiple
    /// values in one parameter.
    pub async fn get_group_audit_logs_filtered(
        &self,
        group_id: &str,
        n: u32,
        offset: u32,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
        actor_ids: Option<&str>,
        event_types: Option<&str>,
        target_ids: Option<&str>,
    ) -> Result<PaginatedGroupAuditLogEntryList, VrChatError> {
        let mut query = vec![("n", n.to_string()), ("offset", offset.to_string())];
        if let Some(start_date) = start_date {
            query.push(("startDate", start_date.to_rfc3339()));
        }
        if let Some(end_date) = end_date {
            query.push(("endDate", end_date.to_rfc3339()));
        }
        if let Some(actor_ids) = actor_ids.filter(|value| !value.trim().is_empty()) {
            query.push(("actorIds", actor_ids.to_string()));
        }
        if let Some(event_types) = event_types.filter(|value| !value.trim().is_empty()) {
            query.push(("eventTypes", event_types.to_string()));
        }
        if let Some(target_ids) = target_ids.filter(|value| !value.trim().is_empty()) {
            query.push(("targetIds", target_ids.to_string()));
        }
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/auditLogs")))
            .query(&query)
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of Java's `auditQuery`: fetches every page and returns entries
    /// oldest-to-newest, which is the ordering its history commands expect.
    pub async fn query_group_audit_logs(
        &self,
        group_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
        actor_ids: Option<&str>,
        event_types: Option<&str>,
        target_ids: Option<&str>,
    ) -> Result<Vec<crate::models::GroupAuditLogEntry>, VrChatError> {
        const PAGE_SIZE: u32 = 100;
        let mut offset = 0;
        let mut entries = Vec::new();
        loop {
            let page = self
                .get_group_audit_logs_filtered(
                    group_id,
                    PAGE_SIZE,
                    offset,
                    Some(start_date),
                    Some(end_date),
                    actor_ids,
                    event_types,
                    target_ids,
                )
                .await?;
            entries.extend(page.results);
            if !page.has_next {
                break;
            }
            offset = offset.saturating_add(PAGE_SIZE);
        }
        entries.sort_by_key(|entry| entry.created_at);
        Ok(entries)
    }

    /// Count-only counterpart to Java's `auditQueryCount`.
    pub async fn count_group_audit_logs(
        &self,
        group_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
        actor_ids: Option<&str>,
        event_types: Option<&str>,
        target_ids: Option<&str>,
    ) -> Result<i64, VrChatError> {
        Ok(self
            .get_group_audit_logs_filtered(
                group_id,
                0,
                0,
                Some(start_date),
                Some(end_date),
                actor_ids,
                event_types,
                target_ids,
            )
            .await?
            .total_count)
    }

    /// `GET /groups/{groupId}/auditLogTypes`, the list of audit event
    /// identifiers currently present in the group.
    pub async fn get_group_audit_log_entry_types(
        &self,
        group_id: &str,
    ) -> Result<Vec<String>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/auditLogTypes")))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getUser(String userId)`: `GET /users/{userId}`.
    /// Port of `getUser(String userId)`, i.e. `getUser(userId,
    /// Long.MAX_VALUE)`: no cached entry is ever new enough, so this always
    /// fetches — but it still short-circuits on a known 404 and still
    /// records the result, exactly as Java does.
    pub async fn get_user(&self, user_id: &str) -> Result<crate::models::User, VrChatError> {
        self.get_user_cached(user_id, crate::entity_cache::ALWAYS_FETCH).await
    }

    /// Port of `getUser(String userId, long minEpoch)`. Call sites that
    /// tolerate stale data (autocomplete, list rendering) pass
    /// [`ALWAYS_PREFER_CACHED`](crate::entity_cache::ALWAYS_PREFER_CACHED)
    /// and then cost nothing after the first fetch.
    pub async fn get_user_cached(
        &self,
        user_id: &str,
        min_epoch: i64,
    ) -> Result<crate::models::User, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.users);
        match crate::entity_cache::decide(cache, user_id, min_epoch) {
            crate::entity_cache::CacheDecision::Hit(user) => return Ok(user),
            crate::entity_cache::CacheDecision::Known404 => return Err(Self::known_404("user", user_id)),
            crate::entity_cache::CacheDecision::Miss => {}
        }
        let result = async {
            let response = self.http.get(self.url(&format!("/users/{user_id}"))).send().await?;
            Self::parse_response(response).await
        }
        .await;
        crate::entity_cache::record(cache, user_id, &result);
        result
    }

    /// Port of `getUserFresh(String userId)`: bypasses the cache read,
    /// refreshes the entry and clears any stale 404.
    pub async fn get_user_fresh(&self, user_id: &str) -> Result<crate::models::User, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.users);
        let result = async {
            let response = self.http.get(self.url(&format!("/users/{user_id}"))).send().await?;
            Self::parse_response(response).await
        }
        .await;
        crate::entity_cache::record(cache, user_id, &result);
        result
    }

    /// The error a known-404 short-circuit produces. Java returns `null`
    /// here; the port's getters are `Result`, and every caller already
    /// treats an error as "no such entity", so this keeps the shape without
    /// making every call site handle a third case.
    fn known_404(kind: &str, id: &str) -> VrChatError {
        VrChatError::Api {
            status: StatusCode::NOT_FOUND,
            body: format!("{kind} `{id}` is in the known-404 cache"),
        }
    }

    /// Fetches the public-profile shape that carries `bio` in current API
    /// versions (it is no longer present on the ordinary User response).
    pub async fn get_public_profile(
        &self,
        user_id: &str,
    ) -> Result<crate::models::PublicProfile, VrChatError> {
        self.get_public_profile_cached(user_id, crate::entity_cache::ALWAYS_FETCH).await
    }

    /// Port of `getPublicProfile(String userId, long minEpoch)`
    /// (`cachedPublicProfiles`, kind `pubprof`).
    pub async fn get_public_profile_cached(
        &self,
        user_id: &str,
        min_epoch: i64,
    ) -> Result<crate::models::PublicProfile, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.public_profiles);
        match crate::entity_cache::decide(cache, user_id, min_epoch) {
            crate::entity_cache::CacheDecision::Hit(profile) => return Ok(profile),
            crate::entity_cache::CacheDecision::Known404 => {
                return Err(Self::known_404("public profile for", user_id))
            }
            crate::entity_cache::CacheDecision::Miss => {}
        }
        let result = async {
            let response = self
                .http
                .get(self.url(&format!("/users/{user_id}/profile")))
                .query(&[("asSelf", "false"), ("withGroupsAndWorlds", "false")])
                .send()
                .await?;
            Self::parse_response(response).await
        }
        .await;
        crate::entity_cache::record(cache, user_id, &result);
        result
    }

    /// Port of `getUserGroups(String userId)`: `GET /users/{userId}/groups`.
    ///
    /// The watched-group join check runs against this. VRChat only returns
    /// groups whose membership the *requesting* account is allowed to see,
    /// so a hidden membership legitimately comes back absent — the check
    /// treats that as "not in a watched group" rather than retrying, which
    /// is what Java does too.
    pub async fn get_user_groups(
        &self,
        user_id: &str,
    ) -> Result<Vec<crate::models::LimitedUserGroup>, VrChatError> {
        self.get_user_groups_cached(user_id, crate::entity_cache::ALWAYS_FETCH).await
    }

    /// Port of `getUserGroups(String userId, long minEpoch)` (`cachedUserGroups`,
    /// kind `gmem`).
    pub async fn get_user_groups_cached(
        &self,
        user_id: &str,
        min_epoch: i64,
    ) -> Result<Vec<crate::models::LimitedUserGroup>, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.user_groups);
        match crate::entity_cache::decide(cache, user_id, min_epoch) {
            crate::entity_cache::CacheDecision::Hit(groups) => return Ok(groups),
            crate::entity_cache::CacheDecision::Known404 => {
                return Err(Self::known_404("user groups for", user_id))
            }
            crate::entity_cache::CacheDecision::Miss => {}
        }
        let result = async {
            let response =
                self.http.get(self.url(&format!("/users/{user_id}/groups"))).send().await?;
            Self::parse_response(response).await
        }
        .await;
        crate::entity_cache::record(cache, user_id, &result);
        result
    }

    /// Captures the unabridged user JSON used by moderation audit snapshots.
    /// Unlike [`Self::get_user`], this deliberately does not deserialize into
    /// the port's reduced `User` model: snapshots are historical records and
    /// must retain fields that Celeste does not otherwise inspect.
    pub async fn get_user_snapshot(&self, user_id: &str) -> Result<Value, VrChatError> {
        let response = self.http.get(self.url(&format!("/users/{user_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Captures the unabridged group-membership array for an audit target.
    pub async fn get_user_groups_snapshot(&self, user_id: &str) -> Result<Value, VrChatError> {
        let response = self.http.get(self.url(&format!("/users/{user_id}/groups"))).send().await?;
        Self::parse_response(response).await
    }

    /// Captures the group represented by an audit target, if any. The API
    /// returns JSON `null` when the user is not representing a group; keeping
    /// that value distinguishes a successful snapshot from a failed request.
    pub async fn get_user_represented_group_snapshot(&self, user_id: &str) -> Result<Value, VrChatError> {
        let response = self.http.get(self.url(&format!("/users/{user_id}/groups/represented"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getModelFile(String fileId)`: `GET /file/{fileId}`.
    pub async fn get_file(&self, file_id: &str) -> Result<crate::models::ModelFile, VrChatError> {
        let response = self.http.get(self.url(&format!("/file/{file_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// Port of `getFileAnalysis(VersionedFile)`: `GET
    /// /analysis/{fileId}/{version}`, with Java's two qualifier variants
    /// (`/security`, `/standard`). Anything else falls through to the plain
    /// endpoint, exactly as Java's `if/else if/else` does.
    ///
    /// This is where an avatar's performance rating comes from — the rating
    /// VRChat itself shows on a nameplate.
    pub async fn get_file_analysis(
        &self,
        file: &scarlet_core::versioned_file::VersionedFile,
    ) -> Result<crate::models::FileAnalysis, VrChatError> {
        let path = match file.qualifier.as_str() {
            "security" => format!("/analysis/{}/{}/security", file.id, file.version),
            "standard" => format!("/analysis/{}/{}/standard", file.id, file.version),
            _ => format!("/analysis/{}/{}", file.id, file.version),
        };
        let response = self.http.get(self.url(&path)).send().await?;
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
    pub async fn get_group(
        &self,
        group_id: &str,
        include_roles: bool,
    ) -> Result<crate::models::Group, VrChatError> {
        self.get_group_cached(group_id, include_roles, crate::entity_cache::ALWAYS_FETCH).await
    }

    /// Java keeps the configured group's owner in `ScarletVRChat.groupOwnerId`,
    /// filled from `group.getOwnerId()` when the group is resolved at startup
    /// (`ScarletVRChat.java:1284`). Resolving it on demand through the day-old
    /// group cache costs nothing after the first call and avoids a startup
    /// ordering dependency.
    pub async fn group_owner_id(&self, group_id: &str) -> Option<String> {
        self.get_group_cached(group_id, false, crate::entity_cache::within_1_day())
            .await
            .ok()
            .and_then(|group| group.owner_id)
    }

    /// Port of `getGroup(String groupId, Boolean includeRoles, long minEpoch)`.
    ///
    /// Note that Java caches by group id alone, ignoring `includeRoles`, so
    /// a cache hit populated without roles satisfies a later call that asked
    /// for them. That is Java's behaviour and callers already tolerate an
    /// absent role list, so it is reproduced rather than "fixed".
    pub async fn get_group_cached(
        &self,
        group_id: &str,
        include_roles: bool,
        min_epoch: i64,
    ) -> Result<crate::models::Group, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.groups);
        match crate::entity_cache::decide(cache, group_id, min_epoch) {
            crate::entity_cache::CacheDecision::Hit(group) => return Ok(group),
            crate::entity_cache::CacheDecision::Known404 => {
                return Err(Self::known_404("group", group_id))
            }
            crate::entity_cache::CacheDecision::Miss => {}
        }
        let result = async {
            let mut request = self.http.get(self.url(&format!("/groups/{group_id}")));
            if include_roles {
                request = request.query(&[("includeRoles", "true")]);
            }
            Self::parse_response(request.send().await?).await
        }
        .await;
        crate::entity_cache::record(cache, group_id, &result);
        result
    }

    /// Port of `getGroupRoles(String groupId)`: `GET /groups/{groupId}/roles`.
    pub async fn get_group_roles(
        &self,
        group_id: &str,
    ) -> Result<Vec<crate::models::GroupRole>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/roles")))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupBans(String groupId, int limit)`: `GET
    /// /groups/{groupId}/bans`.
    pub async fn get_group_bans(
        &self,
        group_id: &str,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::GroupMember>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/bans")))
            .query(&[("n", n.to_string()), ("offset", offset.to_string())])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupInvites`: `GET /groups/{groupId}/invites`. Unlike
    /// Java's version, this fetches a single page rather than looping with
    /// a 250ms sleep between pages up to some overall `limit` -- matching
    /// the same one-page-per-call shape already used for
    /// `get_group_bans`/`search_group_members` elsewhere in this client.
    pub async fn get_group_invites(
        &self,
        group_id: &str,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::GroupMember>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/invites")))
            .query(&[("n", n.to_string()), ("offset", offset.to_string())])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `createGroupInvite`.
    pub async fn create_group_invite(
        &self,
        group_id: &str,
        user_id: &str,
        confirm_override_block: bool,
    ) -> Result<(), VrChatError> {
        let response = self
            .http
            .post(self.url(&format!("/groups/{group_id}/invites")))
            .json(&serde_json::json!({
                "userId": user_id,
                "confirmOverrideBlock": confirm_override_block
            }))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(VrChatError::Api { status, body })
        }
    }

    /// Port of `getGroupJoinRequests`: `GET /groups/{groupId}/requests`.
    pub async fn get_group_join_requests(
        &self,
        group_id: &str,
        blocked: bool,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::GroupMember>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/requests")))
            .query(&[
                ("n", n.to_string()),
                ("offset", offset.to_string()),
                ("blocked", blocked.to_string()),
            ])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// `PUT /groups/{groupId}/requests/{userId}` from the community OpenAPI
    /// `respondGroupJoinRequest` operation.
    pub async fn respond_group_join_request(
        &self,
        group_id: &str,
        user_id: &str,
        action: &str,
        block: Option<bool>,
    ) -> Result<(), VrChatError> {
        let mut body = serde_json::json!({ "action": action });
        if let Some(block) = block {
            body["block"] = serde_json::json!(block);
        }
        let response = self.http
            .put(self.url(&format!("/groups/{group_id}/requests/{user_id}")))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(VrChatError::Api { status, body })
        }
    }

    /// Port of `banFromGroup(String targetUserId)`: `POST
    /// /groups/{groupId}/bans` with `{"userId": ...}`.
    pub async fn ban_from_group(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<crate::models::GroupMember, VrChatError> {
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
    pub async fn unban_from_group(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<crate::models::GroupMember, VrChatError> {
        let response = self
            .http
            .delete(self.url(&format!("/groups/{group_id}/bans/{user_id}")))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupMembership(String groupId, String targetUserId)`:
    /// `GET /groups/{groupId}/members/{userId}`. Returns `Ok(None)` (not an
    /// error) when the response body is JSON `null`, matching the spec's
    /// documented "returns a GroupMember, or null when not a member".
    pub async fn get_group_member(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<Option<crate::models::GroupMember>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/members/{user_id}")))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        let value: Value =
            serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|e| VrChatError::UnexpectedShape(e.to_string()))
    }

    /// Port of `updateGroupMembershipNotes`: `PUT
    /// /groups/{groupId}/members/{userId}` with an `UpdateGroupMemberRequest`.
    pub async fn update_group_member_notes(
        &self,
        group_id: &str,
        user_id: &str,
        manager_notes: &str,
    ) -> Result<crate::models::GroupMember, VrChatError> {
        let response = self.http
            .put(self.url(&format!("/groups/{group_id}/members/{user_id}")))
            .json(&serde_json::json!({ "managerNotes": manager_notes }))
            .send()
            .await?;
        Self::parse_response(response).await
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
            .query(&[
                ("query", query),
                ("n", &n.to_string()),
                ("offset", &offset.to_string()),
            ])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of the kick half of group member management: `DELETE
    /// /groups/{groupId}/members/{userId}`.
    pub async fn kick_group_member(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<(), VrChatError> {
        let response = self
            .http
            .delete(self.url(&format!("/groups/{group_id}/members/{user_id}")))
            .send()
            .await?;
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
    pub async fn add_group_member_role(
        &self,
        group_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, VrChatError> {
        let response = self
            .http
            .put(self.url(&format!(
                "/groups/{group_id}/members/{user_id}/roles/{role_id}"
            )))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `removeGroupRole(String groupId, String targetUserId, String
    /// groupRoleId)`: `DELETE /groups/{groupId}/members/{userId}/roles/{roleId}`.
    pub async fn remove_group_member_role(
        &self,
        group_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> Result<Vec<String>, VrChatError> {
        let response = self
            .http
            .delete(self.url(&format!(
                "/groups/{group_id}/members/{user_id}/roles/{role_id}"
            )))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getGroupAnnouncement`: `GET /groups/{groupId}/announcement`.
    /// Returns an "empty" `GroupAnnouncement` (see its `is_empty`) rather
    /// than an error when the group has no current announcement, matching
    /// the API's own documented behavior.
    pub async fn get_group_announcement(
        &self,
        group_id: &str,
    ) -> Result<crate::models::GroupAnnouncement, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/announcement")))
            .send()
            .await?;
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
        image_id: Option<&str>,
    ) -> Result<crate::models::GroupAnnouncement, VrChatError> {
        let mut body = serde_json::json!({ "title": title, "text": text, "sendNotification": send_notification });
        if let Some(image_id) = image_id {
            body["imageId"] = serde_json::json!(image_id);
        }
        let response = self
            .http
            .post(self.url(&format!("/groups/{group_id}/announcement")))
            .json(&body)
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `deleteGroupAnnouncement`: `DELETE /groups/{groupId}/announcement`.
    pub async fn delete_group_announcement(&self, group_id: &str) -> Result<(), VrChatError> {
        let response = self
            .http
            .delete(self.url(&format!("/groups/{group_id}/announcement")))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    /// Port of `getGroupPosts(String groupId, boolean publicOnly, int
    /// limit)`: `GET /groups/{groupId}/posts?number=...&offset=...&publicOnly=...`.
    /// The response wraps results as `{posts, total}` rather than a bare
    /// array (confirmed against
    /// `components/responses/groups/GroupPostsResponse.yaml`) -- unlike
    /// Java, which loops pages internally up to an overall `limit`, this
    /// fetches a single page, matching the one-page-per-call shape already
    /// used for bans/invites/join-requests/member-search in this client.
    pub async fn get_group_posts(
        &self,
        group_id: &str,
        public_only: bool,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::GroupPost>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/posts")))
            .query(&[
                ("number", n.to_string()),
                ("offset", offset.to_string()),
                ("publicOnly", public_only.to_string()),
            ])
            .send()
            .await?;
        let wrapper: crate::models::GroupPostsResponse = Self::parse_response(response).await?;
        Ok(wrapper.posts)
    }

    /// Port of `createGroupPost`: `POST /groups/{groupId}/posts`, body shape
    /// confirmed against `components/requests/CreateGroupPostRequest.yaml`.
    pub async fn create_group_post(
        &self,
        group_id: &str,
        title: &str,
        text: &str,
        visibility: &str,
        send_notification: bool,
        image_id: Option<&str>,
        role_id: Option<&str>,
    ) -> Result<crate::models::GroupPost, VrChatError> {
        let mut body = serde_json::json!({
            "title": title,
            "text": text,
            "visibility": visibility,
            "sendNotification": send_notification,
        });
        if let Some(role_id) = role_id {
            body["roleIds"] = serde_json::json!([role_id]);
        }
        if let Some(image_id) = image_id {
            body["imageId"] = serde_json::json!(image_id);
        }
        let response = self
            .http
            .post(self.url(&format!("/groups/{group_id}/posts")))
            .json(&body)
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `deleteGroupPost`: `DELETE /groups/{groupId}/posts/{postId}`.
    pub async fn delete_group_post(
        &self,
        group_id: &str,
        post_id: &str,
    ) -> Result<(), VrChatError> {
        let response = self
            .http
            .delete(self.url(&format!("/groups/{group_id}/posts/{post_id}")))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    /// Port of `getGroupTransferability(String groupId, String
    /// targetUserId)`: `GET
    /// /groups/{groupId}/transfer?transferTargetId=...`, confirmed against
    /// `components/schemas/GroupTransferable.yaml`.
    pub async fn get_group_transferability(
        &self,
        group_id: &str,
        target_user_id: &str,
    ) -> Result<crate::models::GroupTransferable, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/transfer")))
            .query(&[("transferTargetId", target_user_id)])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `initiateOrAcceptGroupTransfer`: `POST
    /// /groups/{groupId}/transfer` with `{"transferTargetId": ...}`. "Must be
    /// logged in as current owner to initiate; must be logged in as transfer
    /// target to accept" per the spec -- this client doesn't distinguish the
    /// two cases, matching Java's own `initiateOrAcceptGroupTransfer`.
    pub async fn initiate_or_accept_group_transfer(
        &self,
        group_id: &str,
        target_user_id: &str,
    ) -> Result<(), VrChatError> {
        let response = self
            .http
            .post(self.url(&format!("/groups/{group_id}/transfer")))
            .json(&serde_json::json!({ "transferTargetId": target_user_id }))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    /// Port of `cancelGroupTransfer`: `DELETE /groups/{groupId}/transfer`.
    pub async fn cancel_group_transfer(&self, group_id: &str) -> Result<(), VrChatError> {
        let response = self
            .http
            .delete(self.url(&format!("/groups/{group_id}/transfer")))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        Ok(())
    }

    // ---- worlds / avatars / instances ----

    /// Java `GroupsApi.getGroupInstances`: `GET /groups/{groupId}/instances`.
    /// Kept as JSON because the lifecycle worker only needs the location,
    /// instance id, and nested world identity from VRChat's much larger
    /// `GroupInstance` response.
    pub async fn get_group_instances(&self, group_id: &str) -> Result<Vec<serde_json::Value>, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/groups/{group_id}/instances")))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// `GET /prints/{printId}`. Kept as JSON because Celeste only needs the
    /// owner and nested file/image fields for Discord audit decoration.
    pub async fn get_print(&self, print_id: &str) -> Result<serde_json::Value, VrChatError> {
        let response = self.http.get(self.url(&format!("/prints/{print_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// `GET /props/{propId}`. The endpoint is documented by the community
    /// OpenAPI specification and mirrors Java's generated `PropsApi` call.
    pub async fn get_prop(&self, prop_id: &str) -> Result<serde_json::Value, VrChatError> {
        let response = self.http.get(self.url(&format!("/props/{prop_id}"))).send().await?;
        Self::parse_response(response).await
    }

    /// `GET /user/{userId}/inventory/{inventoryItemId}`, matching Java's
    /// hand-written call used to identify spawned emoji inventory items.
    pub async fn get_user_inventory_item(
        &self,
        user_id: &str,
        inventory_item_id: &str,
    ) -> Result<serde_json::Value, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/user/{user_id}/inventory/{inventory_item_id}")))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getWorld(String worldId)`: `GET /worlds/{worldId}`.
    pub async fn get_world(&self, world_id: &str) -> Result<crate::models::World, VrChatError> {
        self.get_world_cached(world_id, crate::entity_cache::ALWAYS_FETCH).await
    }

    /// Port of `getWorld(String worldId, long minEpoch)`.
    pub async fn get_world_cached(
        &self,
        world_id: &str,
        min_epoch: i64,
    ) -> Result<crate::models::World, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.worlds);
        match crate::entity_cache::decide(cache, world_id, min_epoch) {
            crate::entity_cache::CacheDecision::Hit(world) => return Ok(world),
            crate::entity_cache::CacheDecision::Known404 => {
                return Err(Self::known_404("world", world_id))
            }
            crate::entity_cache::CacheDecision::Miss => {}
        }
        let result = async {
            let response = self.http.get(self.url(&format!("/worlds/{world_id}"))).send().await?;
            Self::parse_response(response).await
        }
        .await;
        crate::entity_cache::record(cache, world_id, &result);
        result
    }

    /// Port of `getAvatar(String avatarId)`: `GET /avatars/{avatarId}`.
    pub async fn get_avatar(&self, avatar_id: &str) -> Result<crate::models::Avatar, VrChatError> {
        self.get_avatar_cached(avatar_id, crate::entity_cache::ALWAYS_FETCH).await
    }

    /// Port of `getAvatar(String avatarId, long minEpoch)`.
    pub async fn get_avatar_cached(
        &self,
        avatar_id: &str,
        min_epoch: i64,
    ) -> Result<crate::models::Avatar, VrChatError> {
        let cache = self.caches.as_ref().map(|caches| &caches.avatars);
        match crate::entity_cache::decide(cache, avatar_id, min_epoch) {
            crate::entity_cache::CacheDecision::Hit(avatar) => return Ok(avatar),
            crate::entity_cache::CacheDecision::Known404 => {
                return Err(Self::known_404("avatar", avatar_id))
            }
            crate::entity_cache::CacheDecision::Miss => {}
        }
        let result = async {
            let response = self.http.get(self.url(&format!("/avatars/{avatar_id}"))).send().await?;
            Self::parse_response(response).await
        }
        .await;
        crate::entity_cache::record(cache, avatar_id, &result);
        result
    }

    /// Port of `VrchatSearch.world` / `WorldsApi.searchWorlds`: `GET
    /// /worlds?search=...&n=...&offset=...`. Confirmed against the spec --
    /// the shared `number` parameter's actual wire name is `n`, not
    /// `number` (unlike `getGroupPosts`'s own locally-defined `number`
    /// param -- VRChat's API is inconsistent about this between endpoints,
    /// not a typo here).
    pub async fn search_worlds(
        &self,
        query: &str,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::World>, VrChatError> {
        let response = self
            .http
            .get(self.url("/worlds"))
            .query(&[
                ("search", query),
                ("n", &n.to_string()),
                ("offset", &offset.to_string()),
            ])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `VrchatSearch.user` / `UsersApi.searchUsers`: `GET
    /// /users?search=...&n=...&offset=...`. Returns `LimitedUserSearch`
    /// shapes ([`crate::models::UserSearchResult`]), not the fuller
    /// [`crate::models::User`] `get_user` returns.
    pub async fn search_users(
        &self,
        query: &str,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::UserSearchResult>, VrChatError> {
        let response = self
            .http
            .get(self.url("/users"))
            .query(&[
                ("search", query),
                ("n", &n.to_string()),
                ("offset", &offset.to_string()),
            ])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `VrchatSearch.group` / `GroupsApi.searchGroups`: `GET
    /// /groups?query=...&offset=...&n=...`. Reuses [`crate::models::Group`]
    /// for the response -- the spec's `LimitedGroup` search-result shape is
    /// a subset of the same field names (`roles`/`onlineMemberCount` just
    /// come back `None`/`0`).
    pub async fn search_groups(
        &self,
        query: &str,
        n: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::Group>, VrChatError> {
        let response = self
            .http
            .get(self.url("/groups"))
            .query(&[
                ("query", query),
                ("n", &n.to_string()),
                ("offset", &offset.to_string()),
            ])
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `getInstance(String worldId, String instanceId)`: `GET
    /// /instances/{worldId}:{instanceId}`.
    pub async fn get_instance(
        &self,
        world_id: &str,
        instance_id: &str,
    ) -> Result<crate::models::Instance, VrChatError> {
        let response = self
            .http
            .get(self.url(&format!("/instances/{world_id}:{instance_id}")))
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of Celeste's `createInstanceEx`, including fields omitted by the
    /// older generated Java model such as `minimumAvatarPerformance`.
    pub async fn create_instance(
        &self,
        request: &serde_json::Value,
    ) -> Result<crate::models::Instance, VrChatError> {
        let response = self
            .http
            .post(self.url("/instances"))
            .json(request)
            .send()
            .await?;
        Self::parse_response(response).await
    }

    /// Port of `closeInstance(String worldId, String instanceId, Boolean
    /// hardClose, OffsetDateTime closedAt)`: `DELETE
    /// /instances/{worldId}:{instanceId}` with optional `hardClose`/`closedAt`
    /// query params, both confirmed against the spec
    /// (`components/paths/instances.yaml`).
    pub async fn close_instance(
        &self,
        world_id: &str,
        instance_id: &str,
        hard_close: Option<bool>,
        closed_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<crate::models::Instance, VrChatError> {
        let mut request = self
            .http
            .delete(self.url(&format!("/instances/{world_id}:{instance_id}")));
        if let Some(hard_close) = hard_close {
            request = request.query(&[("hardClose", hard_close.to_string())]);
        }
        if let Some(closed_at) = closed_at {
            request = request.query(&[("closedAt", closed_at.to_rfc3339())]);
        }
        Self::parse_response(request.send().await?).await
    }

    /// Port of `checkUserHasVRChatPermission(GroupMember, GroupPermissions)`
    /// plus the group-owner fallback from `checkSelfUserHasVRChatPermission`
    /// (`this.currentUserId.equals(this.groupOwnerId)`). **Not ported**: the
    /// `allGroupPermissions` live-event permission cache Java checks first —
    /// that's a separate, not-yet-ported subsystem fed by gateway events;
    /// this always does a fresh `get_group`/`get_group_member` fetch
    /// instead, which is correct, just uncached (an extra round trip per
    /// check rather than a cache hit).
    /// Port of `checkUserHasVRChatPermission`: does this *member* hold the
    /// permission through one of their group roles?
    ///
    /// **Group ownership is deliberately not a grant here.** Java's owner
    /// fallback lives only in `checkSelfUserHasVRChatPermission` — the check
    /// against Celeste's *own* account — and this port used to apply it to
    /// everyone, which quietly let a group owner past member-level
    /// moderation gates their roles didn't grant. Use
    /// [`VrChatClient::self_has_group_permission`] for the bot's own check.
    pub async fn user_has_group_permission(
        &self,
        group_id: &str,
        user_id: &str,
        permission: &str,
    ) -> Result<bool, VrChatError> {
        self.has_group_permission(group_id, user_id, permission, false).await
    }

    /// Port of `checkSelfUserHasVRChatPermission`: as above, plus Java's
    /// explicit group-owner fallback, which exists so an owner-run bot
    /// isn't blocked by its own role configuration.
    pub async fn self_has_group_permission(
        &self,
        group_id: &str,
        user_id: &str,
        permission: &str,
    ) -> Result<bool, VrChatError> {
        self.has_group_permission(group_id, user_id, permission, true).await
    }

    async fn has_group_permission(
        &self,
        group_id: &str,
        user_id: &str,
        permission: &str,
        owner_is_a_grant: bool,
    ) -> Result<bool, VrChatError> {
        let group = self.get_group(group_id, true).await?;
        if owner_is_a_grant && group.owner_id.as_deref() == Some(user_id) {
            tracing::debug!("VRChat permission `{permission}` granted by group owner fallback for `{user_id}`");
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
            .any(|role| {
                role.permissions
                    .iter()
                    .any(|p| p == PERMISSION_ALL || p == permission)
            }))
    }

    async fn parse_response<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, VrChatError> {
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
/// What an unattended login attempt with stored credentials ended up as.
///
/// The two-factor case used to be collapsed into "no client" with a log
/// line telling the user to re-run the setup wizard — which they can't,
/// because the wizard only runs when setup hasn't completed. On an account
/// with 2FA (the normal case for a moderation bot account) that made every
/// launch after the first permanently signed out: the player table still
/// filled from the log, but account age, pronouns, trust rank, 18+ status
/// and every watched-group check silently stayed empty. Java prompts for
/// the code at each launch instead; returning the half-authenticated client
/// here is what lets the UI do the same.
pub enum StoredLogin {
    LoggedIn(VrChatClient),
    RequiresTwoFactor {
        client: VrChatClient,
        methods: Vec<String>,
    },
    /// No credentials stored, or VRChat rejected them (in which case they
    /// have been cleared).
    Unavailable,
}

/// Unattended login that reports the two-factor case instead of swallowing
/// it. See [`StoredLogin`].
/// Encrypted-registry key Java's cookie jar uses for the primary account
/// (`ScarletVRChatCookieJar`'s `domain + ".cookieStore"`, with domain
/// `global`).
pub const COOKIE_STORE_KEY: &str = "global.cookieStore";

/// Loads Java's encrypted TOTP seed, migrating the old plaintext
/// `vrc_secret` setting if one is present.
pub fn stored_totp_secret(settings: &scarlet_core::settings::Settings) -> Option<String> {
    use scarlet_core::settings::registry::registry_string_encrypted;
    let encrypted = registry_string_encrypted(settings.encrypted.clone(), TOTP_SECRET_KEY);
    if let Some(legacy) = settings.store.get_string("vrc_secret") {
        match crate::totp::normalize_secret(&legacy) {
            Ok(secret) => {
                encrypted.set(Some(secret.clone()));
                settings.store.remove("vrc_secret");
                tracing::info!("Migrated the legacy VRChat TOTP secret into encrypted storage");
                return Some(secret);
            }
            Err(error) => tracing::warn!("Ignoring invalid legacy VRChat TOTP secret: {error}"),
        }
    }
    encrypted
        .get_or_null()
        .and_then(|secret| match crate::totp::normalize_secret(&secret) {
            Ok(secret) => Some(secret),
            Err(error) => {
                tracing::warn!("Ignoring invalid stored VRChat TOTP secret: {error}");
                None
            }
        })
}

/// Encrypts an authenticator setup key under the same registry key Java
/// uses. The returned value is normalized and safe to pass to the TOTP
/// generator.
pub fn save_totp_secret(
    settings: &scarlet_core::settings::Settings,
    secret: &str,
) -> Result<String, String> {
    use scarlet_core::settings::registry::registry_string_encrypted;
    let secret = crate::totp::normalize_secret(secret)?;
    registry_string_encrypted(settings.encrypted.clone(), TOTP_SECRET_KEY)
        .set(Some(secret.clone()));
    Ok(secret)
}

/// Attempts Java-style unattended TOTP verification with VRChat's own
/// clock. `false` means no usable secret was stored or VRChat rejected both
/// generated codes; API failures are returned so callers can log them and
/// fall back to an interactive prompt.
async fn verify_with_stored_totp(
    settings: &scarlet_core::settings::Settings,
    client: &VrChatClient,
) -> Result<bool, VrChatError> {
    let Some(secret) = stored_totp_secret(settings) else {
        return Ok(false);
    };
    for attempt in 0..2 {
        let server_time = client.get_server_time().await?;
        let code = crate::totp::generate_code(&secret, server_time.timestamp())
            .map_err(VrChatError::UnexpectedShape)?;
        if client.verify_totp(&code).await? {
            return Ok(true);
        }
        if attempt == 0 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
    Ok(false)
}

/// Builds a client from the saved session, or `None` if nothing is saved.
pub fn client_from_saved_session(
    settings: &scarlet_core::settings::Settings,
) -> Option<VrChatClient> {
    use scarlet_core::settings::registry::registry_string_encrypted;
    let saved =
        registry_string_encrypted(settings.encrypted.clone(), COOKIE_STORE_KEY).get_or_null()?;
    if saved.trim().is_empty() {
        return None;
    }
    match VrChatClient::with_cookies(Some(&saved)) {
        Ok(client) if client.has_session_cookie() => Some(client),
        Ok(_) => None,
        Err(err) => {
            tracing::warn!("Could not restore the saved VRChat session: {err}");
            None
        }
    }
}

/// Persists the client's current cookies (port of the jar's `save()`).
/// Call after any step that can change them: a credential login, a verified
/// two-factor code, a refresh.
pub fn save_session(settings: &scarlet_core::settings::Settings, client: &VrChatClient) {
    use scarlet_core::settings::registry::registry_string_encrypted;
    let cookies = client.export_cookies();
    let store = registry_string_encrypted(settings.encrypted.clone(), COOKIE_STORE_KEY);
    if cookies.trim().is_empty() {
        store.clear();
    } else {
        store.set(Some(cookies));
    }
}

/// Clears the saved session, for when VRChat rejects it.
pub fn clear_session(settings: &scarlet_core::settings::Settings) {
    use scarlet_core::settings::registry::registry_string_encrypted;
    registry_string_encrypted(settings.encrypted.clone(), COOKIE_STORE_KEY).clear();
}

pub async fn login_with_stored_credentials_detailed(
    settings: &scarlet_core::settings::Settings,
) -> StoredLogin {
    use scarlet_core::settings::registry::registry_string_encrypted;

    let username_reg = registry_string_encrypted(settings.encrypted.clone(), USERNAME_KEY);
    let password_reg = registry_string_encrypted(settings.encrypted.clone(), PASSWORD_KEY);
    // The saved cookie jar first: if the session is still good this needs
    // no credentials and no two-factor code at all, which is how Java
    // behaves and what makes a restart free.
    if let Some(client) = client_from_saved_session(settings) {
        match client.get_current_user().await {
            Ok(user) => {
                tracing::info!(
                    "VRChat session restored from the saved cookie as {}",
                    user.display_name.as_deref().unwrap_or(&user.id)
                );
                // Refresh whatever VRChat rotated on that request.
                save_session(settings, &client);
                return StoredLogin::LoggedIn(client);
            }
            Err(err) => {
                tracing::info!("Saved VRChat session is no longer valid ({err}); signing in again");
                clear_session(settings);
            }
        }
    }

    let (Some(username), Some(password)) = (username_reg.get_or_null(), password_reg.get_or_null())
    else {
        return StoredLogin::Unavailable;
    };

    let client = match VrChatClient::with_cookies(None) {
        Ok(client) => client,
        Err(err) => {
            tracing::error!(
                "Could not build VRChat client for unattended login: {}",
                err
            );
            return StoredLogin::Unavailable;
        }
    };

    match client.login_with_credentials(&username, &password).await {
        Ok(LoginOutcome::LoggedIn(user)) => {
            tracing::info!(
                "VRChat unattended login succeeded as {}",
                user.display_name.as_deref().unwrap_or(&user.id)
            );
            save_session(settings, &client);
            StoredLogin::LoggedIn(client)
        }
        Ok(LoginOutcome::RequiresTwoFactor(methods)) => {
            if methods.iter().any(|method| method == "totp") {
                match verify_with_stored_totp(settings, &client).await {
                    Ok(true) => {
                        tracing::info!("VRChat unattended TOTP verification succeeded");
                        save_session(settings, &client);
                        return StoredLogin::LoggedIn(client);
                    }
                    Ok(false) => {}
                    Err(error) => tracing::warn!(
                        "Could not use the stored VRChat authenticator secret: {error}"
                    ),
                }
            }
            tracing::info!(
                "VRChat unattended login needs a two-factor code ({:?})",
                methods
            );
            StoredLogin::RequiresTwoFactor { client, methods }
        }
        Err(err) => {
            tracing::error!("VRChat unattended login failed: {}", err);
            // Port of `ScarletVRChat.login()`'s `catch (ApiException)`: only
            // VRChat actually rejecting the credentials clears them, not a
            // transient network error.
            if matches!(err, VrChatError::Api { .. }) {
                username_reg.clear();
                password_reg.clear();
            }
            StoredLogin::Unavailable
        }
    }
}

pub async fn login_with_stored_credentials(
    settings: &scarlet_core::settings::Settings,
) -> Option<VrChatClient> {
    match login_with_stored_credentials_detailed(settings).await {
        StoredLogin::LoggedIn(client) => Some(client),
        StoredLogin::RequiresTwoFactor { methods, .. } => {
            tracing::warn!(
                "VRChat unattended login requires two-factor ({:?}); run Celeste with its UI to log in",
                methods
            );
            None
        }
        StoredLogin::Unavailable => None,
    }
}

/// The actual login-then-clear-on-rejection logic, split out from
/// [`login_with_stored_credentials`] so it can be exercised against a mock
/// server in tests (the public function always builds a [`VrChatClient`]
/// pointed at the real VRChat API, with no test seam of its own).
#[cfg(test)]
async fn attempt_login_clearing_stored_credentials_on_rejection(
    client: VrChatClient,
    username: &str,
    password: &str,
    username_reg: &scarlet_core::settings::registry::RegistryStringValued<String>,
    password_reg: &scarlet_core::settings::registry::RegistryStringValued<String>,
) -> Option<VrChatClient> {
    match client.login_with_credentials(username, password).await {
        Ok(LoginOutcome::LoggedIn(user)) => {
            tracing::info!(
                "VRChat unattended login succeeded as {}",
                user.display_name.as_deref().unwrap_or(&user.id)
            );
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
            // Port of `ScarletVRChat.login()`'s `catch (ApiException) {
            // this.username.set(null); this.password.set(null); ... }` --
            // VRChat itself rejecting the stored credentials (not a
            // transient network/parse error) must clear them, or every
            // future launch keeps silently retrying the same bad
            // credentials forever with no way for the setup wizard to
            // notice and re-prompt (its own gate only checks "is something
            // stored", not "does it actually work").
            if matches!(err, VrChatError::Api { .. }) {
                username_reg.clear();
                password_reg.clear();
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn java_cookie_store_format_round_trips_the_vrchat_session() {
        let saved = concat!(
            "auth=auth-token; expires=Fri, 01 Jan 2038 00:00:00 GMT; path=/; domain=.vrchat.cloud; secure; httponly\n",
            "twoFactorAuth=2fa-token; expires=Fri, 01 Jan 2038 00:00:00 GMT; path=/; domain=.vrchat.cloud; secure; httponly\n",
        );

        let client = VrChatClient::with_cookies(Some(saved)).unwrap();
        assert!(client.has_session_cookie());

        let exported = client.export_cookies();
        assert!(exported.contains("auth=auth-token"));
        assert!(exported.contains("twoFactorAuth=2fa-token"));

        let restored = VrChatClient::with_cookies(Some(&exported)).unwrap();
        assert!(restored.has_session_cookie());
        assert!(restored
            .export_cookies()
            .contains("twoFactorAuth=2fa-token"));
    }

    /// Pins the user endpoint's real response shape, including the two
    /// fields the join checks depend on. `date_joined` is snake_case on the
    /// wire while everything else on User is camelCase — getting that wrong
    /// deserializes to `None` and silently disables the new-account
    /// advisory rather than failing, which is exactly the kind of bug a
    /// mock-server test catches and a compile does not.
    /// A temp directory for one test's caches. Tests must not share a
    /// caches root, and must not use the process-wide `entity_cache::install`
    /// (a `OnceLock` — the first test to touch it would decide for all).
    fn temp_cache_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "celeste-entity-cache-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// The bug this guards: with no cache wired up, watched-entity
    /// autocomplete re-fetched every watched entity on every keystroke.
    /// Java answers those from `ScarletJsonCache` with
    /// `ALWAYS_PREFER_CACHED`, so the API is touched once per entity, ever.
    #[tokio::test]
    async fn always_prefer_cached_fetches_once_however_often_it_is_asked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_1",
                "displayName": "Someone",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let root = temp_cache_root("prefer-cached");
        let client = VrChatClient::with_base_url(&format!("{}/", server.uri()))
            .unwrap()
            .with_entity_caches(&root);
        assert!(client.has_entity_caches());

        for _ in 0..8 {
            let user = client
                .get_user_cached("usr_1", crate::entity_cache::ALWAYS_PREFER_CACHED)
                .await
                .unwrap();
            assert_eq!(user.display_name.as_deref(), Some("Someone"));
        }
        // wiremock verifies `.expect(1)` when the server drops.
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Java's `getUser(userId)` is `getUser(userId, Long.MAX_VALUE)`: no
    /// cached entry is ever new enough, so it always fetches. Keeping that
    /// means wiring the cache doesn't quietly staleify call sites that
    /// wanted current data.
    #[tokio::test]
    async fn the_default_getter_still_fetches_every_time() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_1",
                "displayName": "Someone",
            })))
            .expect(3)
            .mount(&server)
            .await;

        let root = temp_cache_root("always-fetch");
        let client = VrChatClient::with_base_url(&format!("{}/", server.uri()))
            .unwrap()
            .with_entity_caches(&root);
        for _ in 0..3 {
            client.get_user("usr_1").await.unwrap();
        }
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Java's `cachedUsers.add404` / `is404`: a 404 is remembered and the
    /// id is never re-fetched, even through the always-fetch getter. That
    /// is what stops a watch list full of deleted users from generating
    /// traffic forever.
    #[tokio::test]
    async fn a_404_is_remembered_and_never_refetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_gone"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .expect(1)
            .mount(&server)
            .await;

        let root = temp_cache_root("known-404");
        let client = VrChatClient::with_base_url(&format!("{}/", server.uri()))
            .unwrap()
            .with_entity_caches(&root);
        for _ in 0..5 {
            assert!(client.get_user("usr_gone").await.is_err());
        }
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Without a caches root the client behaves exactly as it did before
    /// caching existed — which is what the rest of this test module and the
    /// wiremock suite rely on.
    #[tokio::test]
    async fn a_client_without_caches_is_unchanged() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_1",
                "displayName": "Someone",
            })))
            .expect(4)
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&format!("{}/", server.uri())).unwrap();
        assert!(!client.has_entity_caches());
        for _ in 0..4 {
            client
                .get_user_cached("usr_1", crate::entity_cache::ALWAYS_PREFER_CACHED)
                .await
                .unwrap();
        }
        drop(server);
    }

    #[tokio::test]
    async fn get_user_reads_date_joined_pronouns_and_tags() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_1",
                "displayName": "Someone",
                "date_joined": "2026-09-01",
                "pronouns": "she/her",
                "tags": ["system_trust_basic"],
                "ageVerificationStatus": "18+"
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&format!("{}/", server.uri())).unwrap();
        let user = client.get_user("usr_1").await.unwrap();
        assert_eq!(
            user.date_joined,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap())
        );
        assert_eq!(user.pronouns.as_deref(), Some("she/her"));
        assert_eq!(user.age_verification_status.as_deref(), Some("18+"));
        assert_eq!(user.tags, vec!["system_trust_basic".to_string()]);
    }

    #[tokio::test]
    async fn get_user_groups_hits_the_documented_path_and_reads_group_ids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "gmem_1", "groupId": "grp_watched", "name": "Watched Group", "shortCode": "WATCH"},
                {"id": "gmem_2", "groupId": "grp_other", "name": "Other"}
            ])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&format!("{}/", server.uri())).unwrap();
        let groups = client.get_user_groups("usr_1").await.unwrap();
        assert_eq!(
            groups
                .iter()
                .filter_map(|g| g.group_id.clone())
                .collect::<Vec<_>>(),
            vec!["grp_watched".to_string(), "grp_other".to_string()]
        );
    }

    #[tokio::test]
    async fn audit_snapshot_endpoints_preserve_unmodeled_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_1", "displayName": "Someone",
                "futureApiField": {"mustRemain": true}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"groupId": "grp_1", "bannerUrl": "https://example.invalid/banner.png"}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1/groups/represented"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "groupId": "grp_1", "unmodeled": 42
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&format!("{}/", server.uri())).unwrap();
        assert_eq!(client.get_user_snapshot("usr_1").await.unwrap()["futureApiField"]["mustRemain"], true);
        assert_eq!(client.get_user_groups_snapshot("usr_1").await.unwrap()[0]["bannerUrl"], "https://example.invalid/banner.png");
        assert_eq!(client.get_user_represented_group_snapshot("usr_1").await.unwrap()["unmodeled"], 42);
    }

    #[test]
    fn production_client_uses_canonical_vrchat_api_host() {
        let client = VrChatClient::new().unwrap();
        assert_eq!(client.base_url, "https://api.vrchat.cloud/api/1");
    }

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
    async fn a_rejected_unattended_login_clears_the_stored_credentials() {
        use scarlet_core::settings::registry::registry_string_encrypted;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/user"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(
                    serde_json::json!({"error": {"message": "Invalid credentials"}}),
                ),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        let username_reg = registry_string_encrypted(settings.encrypted.clone(), USERNAME_KEY);
        let password_reg = registry_string_encrypted(settings.encrypted.clone(), PASSWORD_KEY);
        username_reg.set(Some("someone".to_string()));
        password_reg.set(Some("wrong-password".to_string()));

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let result = attempt_login_clearing_stored_credentials_on_rejection(
            client,
            "someone",
            "wrong-password",
            &username_reg,
            &password_reg,
        )
        .await;

        assert!(result.is_none());
        assert_eq!(
            username_reg.get_or_null(),
            None,
            "a rejected login must not leave stale credentials stored"
        );
        assert_eq!(
            password_reg.get_or_null(),
            None,
            "a rejected login must not leave stale credentials stored"
        );
    }

    #[tokio::test]
    async fn a_network_error_during_unattended_login_leaves_stored_credentials_alone() {
        use scarlet_core::settings::registry::registry_string_encrypted;

        // Port 1 ("tcpmux") is essentially never open on a test/CI host, so
        // this fails at the transport level (connection refused) -- a
        // network hiccup, not VRChat actively rejecting the credentials.
        // (A `MockServer` that's `drop`ped but never mocked isn't a
        // reliable stand-in: an in-flight request can still race the async
        // shutdown and land on its default "no matching mock" response,
        // which is itself a `VrChatError::Api`, not a transport error.)
        let unreachable_url = "http://127.0.0.1:1".to_string();

        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        let username_reg = registry_string_encrypted(settings.encrypted.clone(), USERNAME_KEY);
        let password_reg = registry_string_encrypted(settings.encrypted.clone(), PASSWORD_KEY);
        username_reg.set(Some("someone".to_string()));
        password_reg.set(Some("correct-password".to_string()));

        let client = VrChatClient::with_base_url(&unreachable_url).unwrap();
        let result = attempt_login_clearing_stored_credentials_on_rejection(
            client,
            "someone",
            "correct-password",
            &username_reg,
            &password_reg,
        )
        .await;

        assert!(result.is_none());
        assert_eq!(
            username_reg.get_or_null().as_deref(),
            Some("someone"),
            "a transient network error must not clear good credentials"
        );
        assert_eq!(
            password_reg.get_or_null().as_deref(),
            Some("correct-password")
        );
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
        let outcome = client
            .login_with_credentials("someone", "p&ss")
            .await
            .unwrap();
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
        let outcome = client
            .login_with_credentials("someone", "pass")
            .await
            .unwrap();
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
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "verified": true })),
            )
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(client.verify_totp("123456").await.unwrap());
    }

    #[tokio::test]
    async fn verify_email_otp_uses_email_endpoint_and_reports_acceptance() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/twofactorauth/emailotp/verify"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "verified": true })),
            )
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(client.verify_email_otp("123456").await.unwrap());
    }

    #[tokio::test]
    async fn server_time_reads_the_api_json_string() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/time"))
            .respond_with(ResponseTemplate::new(200).set_body_json("1970-01-01T00:00:59Z"))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert_eq!(client.get_server_time().await.unwrap().timestamp(), 59);
    }

    #[tokio::test]
    async fn stored_totp_secret_generates_and_verifies_a_fresh_code() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/time"))
            .respond_with(ResponseTemplate::new(200).set_body_json("1970-01-01T00:00:59Z"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/twofactorauth/totp/verify"))
            .and(body_json(serde_json::json!({ "code": "287082" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "verified": true })),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        save_totp_secret(&settings, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        let client = VrChatClient::with_base_url(&server.uri()).unwrap();

        assert!(verify_with_stored_totp(&settings, &client).await.unwrap());
    }

    #[test]
    fn legacy_plaintext_totp_secret_is_migrated_to_encrypted_storage() {
        use scarlet_core::settings::registry::registry_string_encrypted;

        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        settings
            .store
            .set_string("vrc_secret", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");

        assert_eq!(
            stored_totp_secret(&settings).as_deref(),
            Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")
        );
        assert_eq!(settings.store.get_string("vrc_secret"), None);
        assert_eq!(
            registry_string_encrypted(settings.encrypted.clone(), TOTP_SECRET_KEY)
                .get_or_null()
                .as_deref(),
            Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")
        );
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
    async fn get_group_audit_log_entry_types_uses_types_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/auditLogTypes"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!(["group.user.ban", "group.instance.kick"])),
            )
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let types = client
            .get_group_audit_log_entry_types("grp_1")
            .await
            .unwrap();
        assert_eq!(types, ["group.user.ban", "group.instance.kick"]);
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
        let err = client
            .login_with_credentials("someone", "wrong")
            .await
            .unwrap_err();
        match err {
            VrChatError::Api { status, .. } => assert_eq!(status, StatusCode::UNAUTHORIZED),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn only_a_401_is_an_unauthorized_session_verdict() {
        assert!(VrChatError::Api {
            status: StatusCode::UNAUTHORIZED,
            body: String::new(),
        }
        .is_unauthorized());
        assert!(!VrChatError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: String::new(),
        }
        .is_unauthorized());
        assert!(!VrChatError::UnexpectedShape("temporary bad response".into()).is_unauthorized());
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
        assert!(client
            .get_group_member("grp_1", "usr_2")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn logout_uses_vrchats_put_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/logout"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": {"message": "ok"}})))
            .expect(1)
            .mount(&server)
            .await;
        VrChatClient::with_base_url(&server.uri()).unwrap().logout().await.unwrap();
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
        let member = client
            .get_group_member("grp_1", "usr_2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(member.role_ids, vec!["grol_1".to_string()]);
    }

    #[tokio::test]
    async fn update_group_member_notes_uses_the_generated_java_clients_request_shape() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/groups/grp_1/members/usr_2"))
            .and(body_json(serde_json::json!({"managerNotes": "Known alt"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_1", "groupId": "grp_1", "userId": "usr_2",
                "roleIds": [], "managerNotes": "Known alt"
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let member = client.update_group_member_notes("grp_1", "usr_2", "Known alt").await.unwrap();
        assert_eq!(member.manager_notes.as_deref(), Some("Known alt"));
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
        let result = client
            .search_group_members("grp_1", "ali", 10, 0)
            .await
            .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.results[0].user_id.as_deref(), Some("usr_2"));
    }

    #[tokio::test]
    async fn add_and_remove_group_member_role_return_the_role_id_list() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/groups/grp_1/members/usr_2/roles/grol_1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!(["grol_1", "grol_2"])),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/members/usr_2/roles/grol_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(["grol_2"])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let after_add = client
            .add_group_member_role("grp_1", "usr_2", "grol_1")
            .await
            .unwrap();
        assert_eq!(after_add, vec!["grol_1".to_string(), "grol_2".to_string()]);
        let after_remove = client
            .remove_group_member_role("grp_1", "usr_2", "grol_1")
            .await
            .unwrap();
        assert_eq!(after_remove, vec!["grol_2".to_string()]);
    }

    #[tokio::test]
    async fn kick_group_member_succeeds_on_a_2xx_response() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/members/usr_2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"success": {"message": "ok"}})),
            )
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
    async fn print_prop_and_inventory_item_use_the_documented_paths() {
        let server = MockServer::start().await;
        for (request_path, body) in [
            ("/prints/prnt_1", serde_json::json!({"id": "prnt_1", "ownerId": "usr_1"})),
            ("/props/prop_1", serde_json::json!({"id": "prop_1", "authorId": "usr_2"})),
            ("/user/usr_1/inventory/inv_1", serde_json::json!({"id": "inv_1", "itemType": "emoji"})),
        ] {
            Mock::given(method("GET"))
                .and(path(request_path))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
        }
        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert_eq!(client.get_print("prnt_1").await.unwrap()["ownerId"], "usr_1");
        assert_eq!(client.get_prop("prop_1").await.unwrap()["authorId"], "usr_2");
        assert_eq!(
            client.get_user_inventory_item("usr_1", "inv_1").await.unwrap()["itemType"],
            "emoji"
        );
    }

    #[tokio::test]
    async fn get_group_instances_uses_the_groups_instances_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "instanceId": "123~group(grp_1)",
                "location": "wrld_1:123~group(grp_1)",
                "world": {"id": "wrld_1", "name": "Test World"}
            }])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let instances = client.get_group_instances("grp_1").await.unwrap();
        assert_eq!(instances[0]["location"], "wrld_1:123~group(grp_1)");
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
        let closed = client
            .close_instance("wrld_1", "12345", None, None)
            .await
            .unwrap();
        assert_eq!(closed.instance_id.as_deref(), Some("12345"));
    }

    #[tokio::test]
    async fn close_instance_passes_hard_close_and_closed_at_query_params() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/instances/wrld_1:12345"))
            .and(query_param("hardClose", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "worldId": "wrld_1", "instanceId": "12345", "closed": true
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let closed_at = chrono::Utc::now();
        let closed = client
            .close_instance("wrld_1", "12345", Some(true), Some(closed_at))
            .await
            .unwrap();
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

    /// Java's owner fallback is in `checkSelfUserHasVRChatPermission` only,
    /// so it applies to Celeste's own account and to nobody else.
    #[tokio::test]
    async fn group_ownership_grants_permission_only_for_the_bots_own_check() {
        let server = MockServer::start().await;
        mock_group_with_roles(&server, "usr_owner").await;
        // The owner holds no role carrying the permission.
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/members/usr_owner"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gmem_owner", "groupId": "grp_1", "userId": "usr_owner", "roleIds": ["grol_member"]
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        assert!(
            client.self_has_group_permission("grp_1", "usr_owner", PERMISSION_GROUP_BANS_MANAGE).await.unwrap(),
            "the bot's own check keeps Java's owner fallback"
        );
        assert!(
            !client.user_has_group_permission("grp_1", "usr_owner", PERMISSION_GROUP_BANS_MANAGE).await.unwrap(),
            "a member check must not be satisfied by ownership alone"
        );
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
        assert!(client
            .user_has_group_permission("grp_1", "usr_mod", PERMISSION_GROUP_BANS_MANAGE)
            .await
            .unwrap());
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
        assert!(!client
            .user_has_group_permission("grp_1", "usr_plain", PERMISSION_GROUP_BANS_MANAGE)
            .await
            .unwrap());
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
        assert!(!client
            .user_has_group_permission("grp_1", "usr_stranger", PERMISSION_GROUP_BANS_MANAGE)
            .await
            .unwrap());
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
            .and(body_json(serde_json::json!({
                "title": "Hello", "text": "World", "sendNotification": false,
                "imageId": "file_1"
            })))
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
        let created = client
            .create_group_announcement("grp_1", "Hello", "World", false, Some("file_1"))
            .await
            .unwrap();
        assert!(!created.is_empty());
        assert_eq!(created.title.as_deref(), Some("Hello"));
        client.delete_group_announcement("grp_1").await.unwrap();
    }

    #[tokio::test]
    async fn get_group_invites_parses_a_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/invites"))
            .and(query_param("n", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "gmem_1", "userId": "usr_1", "roleIds": []}
            ])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let invites = client.get_group_invites("grp_1", 10, 0).await.unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].user_id.as_deref(), Some("usr_1"));
    }

    #[tokio::test]
    async fn public_profile_and_group_invite_use_current_shapes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/usr_1/profile"))
            .and(query_param("asSelf", "false"))
            .and(query_param("withGroupsAndWorlds", "false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "usr_1", "displayName": "Alice", "bio": "VRC-VERIFY-12345678"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/groups/grp_1/invites"))
            .and(body_json(serde_json::json!({
                "userId": "usr_1", "confirmOverrideBlock": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let profile = client.get_public_profile("usr_1").await.unwrap();
        assert_eq!(profile.bio.as_deref(), Some("VRC-VERIFY-12345678"));
        client
            .create_group_invite("grp_1", "usr_1", false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_group_join_requests_passes_the_blocked_flag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/requests"))
            .and(query_param("blocked", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "gmem_2", "userId": "usr_2", "roleIds": []}
            ])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let requests = client
            .get_group_join_requests("grp_1", true, 10, 0)
            .await
            .unwrap();
        assert_eq!(requests[0].user_id.as_deref(), Some("usr_2"));
    }

    #[tokio::test]
    async fn respond_group_join_request_sends_action_and_optional_block() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/groups/grp_1/requests/usr_1"))
            .and(body_json(serde_json::json!({"action": "reject", "block": true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        client.respond_group_join_request("grp_1", "usr_1", "reject", Some(true)).await.unwrap();
    }

    #[tokio::test]
    async fn get_group_posts_unwraps_the_posts_field() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/posts"))
            .and(query_param("publicOnly", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "posts": [{"id": "not_1", "title": "Hello", "text": "World"}],
                "total": 1
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let posts = client.get_group_posts("grp_1", true, 25, 0).await.unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].title.as_deref(), Some("Hello"));
    }

    #[tokio::test]
    async fn create_group_post_sends_optional_image_and_role_ids() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/groups/grp_1/posts"))
            .and(body_json(serde_json::json!({
                "title": "Hello", "text": "World", "visibility": "group",
                "sendNotification": false, "roleIds": ["grol_1"],
                "imageId": "file_1"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "not_1", "title": "Hello", "text": "World", "visibility": "group"
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let post = client
            .create_group_post(
                "grp_1", "Hello", "World", "group", false,
                Some("file_1"), Some("grol_1")
            )
            .await
            .unwrap();
        assert_eq!(post.id.as_deref(), Some("not_1"));
    }

    #[tokio::test]
    async fn delete_group_post_succeeds_on_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/posts/not_1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": true})),
            )
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        client.delete_group_post("grp_1", "not_1").await.unwrap();
    }

    #[tokio::test]
    async fn get_group_transferability_parses_requirements() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/transfer"))
            .and(query_param("transferTargetId", "usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "requirements": {
                    "groupNotMonetized": true,
                    "hasVRCPlus": false,
                    "hasVerifiedEmail": true,
                    "targetCanOwnMoreGroups": true,
                    "targetIsGroupMember": false
                }
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let transferable = client
            .get_group_transferability("grp_1", "usr_1")
            .await
            .unwrap();
        let req = transferable.requirements.unwrap();
        assert_eq!(req.group_not_monetized, Some(true));
        assert_eq!(req.has_vrc_plus, Some(false));
    }

    #[tokio::test]
    async fn group_transferability_preserves_missing_requirement_fields() {
        // The bug this guards: serde defaults made an omitted field look
        // like `false`, while Java reports a nullable Boolean as Unknown.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/transfer"))
            .and(query_param("transferTargetId", "usr_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "requirements": { "groupNotMonetized": true }
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let req = client.get_group_transferability("grp_1", "usr_1").await
            .unwrap().requirements.unwrap();
        assert_eq!(req.group_not_monetized, Some(true));
        assert_eq!(req.has_vrc_plus, None);
        assert_eq!(req.has_verified_email, None);
    }

    #[tokio::test]
    async fn initiate_and_cancel_group_transfer_succeed_on_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/groups/grp_1/transfer"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": true})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/groups/grp_1/transfer"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": true})),
            )
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        client
            .initiate_or_accept_group_transfer("grp_1", "usr_1")
            .await
            .unwrap();
        client.cancel_group_transfer("grp_1").await.unwrap();
    }

    #[tokio::test]
    async fn search_worlds_passes_the_search_query_and_n_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/worlds"))
            .and(query_param("search", "castle"))
            .and(query_param("n", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "wrld_1", "name": "Castle World"}
            ])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let results = client.search_worlds("castle", 10, 0).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Castle World");
    }

    #[tokio::test]
    async fn search_users_parses_limited_user_search_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users"))
            .and(query_param("search", "alice"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "usr_1", "displayName": "Alice", "iconUrl": "https://example.com/a.png"}
            ])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let results = client.search_users("alice", 25, 0).await.unwrap();
        assert_eq!(results[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(
            results[0].icon_url.as_deref(),
            Some("https://example.com/a.png")
        );
    }

    #[tokio::test]
    async fn search_groups_passes_the_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups"))
            .and(query_param("query", "moderators"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "grp_1", "name": "Moderators Group", "memberCount": 5}
            ])))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let results = client.search_groups("moderators", 25, 0).await.unwrap();
        assert_eq!(results[0].name, "Moderators Group");
        assert_eq!(results[0].member_count, 5);
    }

    #[tokio::test]
    async fn audit_query_passes_filters_and_sorts_oldest_first() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/groups/grp_1/auditLogs"))
            .and(query_param("n", "100"))
            .and(query_param("offset", "0"))
            .and(query_param("startDate", "2026-01-01T00:00:00+00:00"))
            .and(query_param("endDate", "2026-01-02T00:00:00+00:00"))
            .and(query_param("actorIds", "usr_actor"))
            .and(query_param("eventTypes", "group.user.ban"))
            .and(query_param("targetIds", "usr_target"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "id": "gaud_new",
                        "groupId": "grp_1",
                        "createdAt": "2026-01-01T12:00:00Z",
                        "eventType": "group.user.ban"
                    },
                    {
                        "id": "gaud_old",
                        "groupId": "grp_1",
                        "createdAt": "2026-01-01T10:00:00Z",
                        "eventType": "group.user.ban"
                    }
                ],
                "totalCount": 2,
                "hasNext": false
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let start = "2026-01-01T00:00:00Z".parse().unwrap();
        let end = "2026-01-02T00:00:00Z".parse().unwrap();
        let entries = client
            .query_group_audit_logs(
                "grp_1",
                start,
                end,
                Some("usr_actor"),
                Some("group.user.ban"),
                Some("usr_target"),
            )
            .await
            .unwrap();
        assert_eq!(entries[0].id, "gaud_old");
        assert_eq!(entries[1].id, "gaud_new");
    }

    #[tokio::test]
    async fn create_instance_posts_the_complete_picker_payload() {
        let server = MockServer::start().await;
        let request = serde_json::json!({
            "worldId": "wrld_1",
            "ownerId": "grp_1",
            "type": "group",
            "region": "us",
            "groupAccessType": "public",
            "minimumAvatarPerformance": "Good"
        });
        Mock::given(method("POST"))
            .and(path("/instances"))
            .and(body_json(&request))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "wrld_1:12345~group(grp_1)",
                "worldId": "wrld_1",
                "instanceId": "12345~group(grp_1)"
            })))
            .mount(&server)
            .await;

        let client = VrChatClient::with_base_url(&server.uri()).unwrap();
        let instance = client.create_instance(&request).await.unwrap();
        assert_eq!(instance.instance_id.as_deref(), Some("12345~group(grp_1)"));
    }
}
