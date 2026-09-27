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
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(VrChatError::Api { status, body });
        }
        serde_json::from_str(&body).map_err(|e| VrChatError::UnexpectedShape(e.to_string()))
    }

    /// Port of `getUser(String userId)`: `GET /users/{userId}`.
    pub async fn get_user(&self, user_id: &str) -> Result<crate::models::User, VrChatError> {
        let response = self.http.get(self.url(&format!("/users/{user_id}"))).send().await?;
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
}
