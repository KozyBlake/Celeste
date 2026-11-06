//! Asking GitHub, once at launch, whether a newer Celeste has been released.
//!
//! The version comparison is in [`celeste_core::update_check`]; this module
//! is only the request. It is deliberately incurious: one call, a short
//! timeout, and `None` on every kind of failure. A launch must never be
//! delayed, blocked or made noisy by a version check, and an install with no
//! internet is a supported way to run Celeste rather than a fault.

use std::time::Duration;

/// The releases endpoint for the published repository.
///
/// Hardcoded rather than built from `identity::FORK_GITHUB_URL`: this is
/// an api.github.com endpoint, not the repository's own address, so
/// deriving one from the other would only couple them.
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/KozyBlake/Celeste/releases/latest";

/// Where to send someone when the API answered but gave no release page.
const RELEASES_PAGE: &str = "https://github.com/KozyBlake/Celeste/releases/latest";

const TIMEOUT: Duration = Duration::from_secs(10);

/// A published release newer than the running build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRelease {
    /// The tag as GitHub published it, such as `v0.11.0`.
    pub tag: String,
    /// The release page, which is where the "what changed" lives.
    pub url: String,
}

/// Returns a release newer than `current`, or `None`.
///
/// `None` covers every failure as well as "nothing newer": no network, a
/// rate-limited or unreachable GitHub, an unparseable tag. The caller
/// cannot distinguish them and should not try to — there is nothing useful
/// to tell a person about a version check that did not happen.
///
/// GitHub's `releases/latest` excludes drafts and pre-releases, so an
/// unfinished tag never reaches anyone.
pub async fn check(current: &str) -> Option<NewRelease> {
    let client = reqwest::Client::builder()
        .user_agent(celeste_core::identity::user_agent())
        .timeout(TIMEOUT)
        .build()
        .ok()?;
    let response = client.get(LATEST_RELEASE_URL).send().await.ok()?;
    if !response.status().is_success() {
        tracing::debug!(status = %response.status(), "Update check: GitHub declined to answer");
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    let tag = body.get("tag_name")?.as_str()?.trim().to_string();
    if !celeste_core::update_check::is_update(&tag, current) {
        return None;
    }
    let url = body
        .get("html_url")
        .and_then(serde_json::Value::as_str)
        .filter(|url| url.starts_with("https://github.com/"))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    tracing::info!("A newer Celeste is available: {tag}");
    Some(NewRelease { tag, url })
}
