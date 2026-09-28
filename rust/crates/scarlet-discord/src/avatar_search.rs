//! Multi-provider VRCX-compatible avatar search.
//!
//! This is the Rust counterpart of Java's `AvatarSearch`: providers are
//! queried with the VRCX `?n=&search=` contract, successful results are cached
//! for the life of the process, and failing providers are temporarily backed
//! off so one dead service cannot make every Discord command slow.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use tokio::sync::Mutex;

pub const SEARCH_N: u32 = 5_000;
pub const BUILTIN_PROVIDERS: [&str; 6] = [
    "https://api.avtrdb.com/v2/avatar/search/vrcx",
    "https://vrcavatarsearch.nekosunevr.co.uk/vrcx_search",
    "https://vrcx.vrcdb.com/avatars/Avatar/VRCX",
    "https://avatarwbvrcxsearch.worldbalancer.com/vrcx_search",
    "https://paw-api.amelia.fun/vrcx_search",
    "https://avtr.fumikoecho.net/api/integrations/avatars/vrcx",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvatarPerformance {
    pub pc_rating: Option<String>,
    pub android_rating: Option<String>,
    pub ios_rating: Option<String>,
    pub has_impostor: bool,
    pub has_security_variant: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VrcxAvatar {
    pub id: String,
    pub name: Option<String>,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub thumbnail_image_url: Option<String>,
    pub release_status: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub performance: Option<AvatarPerformance>,
}

#[derive(Debug, Clone)]
struct FailureState {
    streak: u32,
    blocked_until: Instant,
}

#[derive(Default)]
struct SearchState {
    cache: HashMap<(String, u32, String), Arc<Vec<VrcxAvatar>>>,
    failures: HashMap<String, FailureState>,
}

pub struct AvatarSearch {
    client: reqwest::Client,
    state: Mutex<SearchState>,
}

impl Default for AvatarSearch {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Scarlet/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("valid avatar-search HTTP client");
        Self {
            client,
            state: Mutex::new(SearchState::default()),
        }
    }
}

impl AvatarSearch {
    pub async fn search_all(&self, providers: &[String], search: &str) -> Vec<VrcxAvatar> {
        let mut all = Vec::new();
        for provider in providers {
            if let Some(results) = self.search_cached(provider, SEARCH_N, search).await {
                all.extend(results.iter().cloned());
            }
        }
        all
    }

    async fn search_cached(
        &self,
        provider: &str,
        n: u32,
        search: &str,
    ) -> Option<Arc<Vec<VrcxAvatar>>> {
        if !provider.starts_with("http://") && !provider.starts_with("https://") {
            return Some(Arc::new(Vec::new()));
        }
        let key = (provider.to_string(), n, search.to_string());
        {
            let mut state = self.state.lock().await;
            if let Some(cached) = state.cache.get(&key) {
                return Some(Arc::clone(cached));
            }
            if let Some(failure) = state.failures.get(provider) {
                if failure.blocked_until > Instant::now() {
                    return None;
                }
            }
            state.failures.remove(provider);
        }

        let result = self.fetch(provider, n, search).await;
        let mut state = self.state.lock().await;
        match result {
            Ok(results) => {
                if state.failures.remove(provider).is_some() {
                    tracing::info!(provider, "Avatar search provider recovered");
                }
                let results = Arc::new(results);
                state.cache.insert(key, Arc::clone(&results));
                Some(results)
            }
            Err(error) => {
                let base = error.base_backoff();
                let previous = state
                    .failures
                    .get(provider)
                    .map_or(0, |failure| failure.streak);
                let streak = previous.saturating_add(1);
                let shift = streak.saturating_sub(1).min(6);
                let backoff = base
                    .saturating_mul(1u32 << shift)
                    .min(Duration::from_secs(2 * 60 * 60));
                state.failures.insert(
                    provider.to_string(),
                    FailureState {
                        streak,
                        blocked_until: Instant::now() + backoff,
                    },
                );
                if streak == 1 || streak == 3 {
                    tracing::warn!(provider, streak, backoff_seconds = backoff.as_secs(), error = %error, "Avatar search provider unavailable");
                } else {
                    tracing::debug!(provider, streak, error = %error, "Avatar search provider unavailable");
                }
                None
            }
        }
    }

    async fn fetch(
        &self,
        provider: &str,
        n: u32,
        search: &str,
    ) -> Result<Vec<VrcxAvatar>, SearchError> {
        let response = self
            .client
            .get(provider)
            .query(&[("n", n.to_string()), ("search", search.to_string())])
            .send()
            .await
            .map_err(SearchError::Network)?;
        let status = response.status();
        if !status.is_success() {
            return Err(SearchError::Http(status.as_u16()));
        }
        let json = response
            .json::<Value>()
            .await
            .map_err(SearchError::Malformed)?;
        parse_vrcx_results(&json).map_err(SearchError::Shape)
    }
}

#[derive(Debug, thiserror::Error)]
enum SearchError {
    #[error("network error: {0}")]
    Network(reqwest::Error),
    #[error("HTTP {0}")]
    Http(u16),
    #[error("malformed JSON: {0}")]
    Malformed(reqwest::Error),
    #[error("{0}")]
    Shape(&'static str),
}

impl SearchError {
    fn base_backoff(&self) -> Duration {
        match self {
            Self::Http(429) | Self::Malformed(_) | Self::Shape(_) => Duration::from_secs(5 * 60),
            Self::Http(code) if (400..500).contains(code) => Duration::from_secs(30 * 60),
            Self::Http(code) if *code >= 500 => Duration::from_secs(5 * 60),
            Self::Network(error) if error.is_timeout() => Duration::from_secs(45),
            Self::Network(_) => Duration::from_secs(30 * 60),
            Self::Http(_) => Duration::from_secs(45),
        }
    }
}

pub fn parse_vrcx_results(json: &Value) -> Result<Vec<VrcxAvatar>, &'static str> {
    let array = match json {
        Value::Array(array) => array,
        Value::Object(object) => ["avatars", "results", "data", "items", "records"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_array))
            .ok_or("unrecognized avatar search response shape")?,
        _ => return Err("unrecognized avatar search response shape"),
    };
    Ok(array
        .iter()
        .filter_map(Value::as_object)
        .filter_map(parse_avatar)
        .collect())
}

fn parse_avatar(entry: &Map<String, Value>) -> Option<VrcxAvatar> {
    let id = find_string(entry, &["id", "avatarid", "vrcid"])?;
    let nested_author = find_value(entry, &["author", "user"]).and_then(Value::as_object);
    let author_id = find_string(entry, &["userid", "authorid"]).or_else(|| {
        nested_author.and_then(|a| find_string(a, &["id", "vrcid", "userid", "authorid"]))
    });
    let author_name = find_string(
        entry,
        &[
            "username",
            "userdisplay",
            "userdisplayname",
            "authorname",
            "authordisplay",
            "authordisplayname",
        ],
    )
    .or_else(|| {
        nested_author.and_then(|a| {
            find_string(
                a,
                &[
                    "name",
                    "display",
                    "displayname",
                    "username",
                    "userdisplay",
                    "userdisplayname",
                    "authorname",
                    "authordisplay",
                    "authordisplayname",
                ],
            )
        })
    });
    let performance = find_value(
        entry,
        &["performance", "perf", "avatarperformance", "avatarperf"],
    )
    .and_then(Value::as_object)
    .map(parse_performance);
    Some(VrcxAvatar {
        id,
        name: find_string(
            entry,
            &[
                "name",
                "display",
                "displayname",
                "avatarname",
                "avatardisplay",
                "avatardisplayname",
            ],
        ),
        author_id,
        author_name,
        description: find_string(
            entry,
            &["desc", "description", "avatardesc", "avatardescription"],
        ),
        image_url: find_string(
            entry,
            &["image", "imageurl", "avatarimage", "avatarimageurl"],
        ),
        thumbnail_image_url: find_string(
            entry,
            &[
                "thumbnail",
                "thumbnailimage",
                "thumbnailurl",
                "thumbnailimageurl",
                "avatarthumbnail",
                "avatarthumbnailimage",
                "avatarthumbnailurl",
                "avatarthumbnailimageurl",
            ],
        ),
        release_status: find_string(
            entry,
            &[
                "release",
                "status",
                "releasestatus",
                "avatarrelease",
                "avatarstatus",
                "avatarreleasestatus",
            ],
        ),
        created_at: find_string(
            entry,
            &["created", "createdat", "avatarcreated", "avatarcreatedat"],
        ),
        updated_at: find_string(
            entry,
            &["updated", "updatedat", "avatarupdated", "avatarupdatedat"],
        ),
        performance,
    })
}

fn parse_performance(entry: &Map<String, Value>) -> AvatarPerformance {
    AvatarPerformance {
        pc_rating: find_string(entry, &["pc", "pcrating"]),
        android_rating: find_string(entry, &["android", "androidrating", "quest", "questrating"]),
        ios_rating: find_string(entry, &["ios", "iosrating"]),
        has_impostor: find_bool(entry, &["hasimpostor", "impostor"]).unwrap_or(false),
        has_security_variant: find_bool(
            entry,
            &[
                "hassecurityvariant",
                "securityvariant",
                "hassecurity",
                "security",
            ],
        ),
    }
}

fn normalized(key: &str) -> String {
    key.to_ascii_lowercase().replace('_', "")
}

fn find_value<'a>(entry: &'a Map<String, Value>, aliases: &[&str]) -> Option<&'a Value> {
    entry
        .iter()
        .find(|(key, _)| aliases.contains(&normalized(key).as_str()))
        .map(|(_, value)| value)
}

fn find_string(entry: &Map<String, Value>, aliases: &[&str]) -> Option<String> {
    find_value(entry, aliases).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn find_bool(entry: &Map<String, Value>, aliases: &[&str]) -> Option<bool> {
    find_value(entry, aliases).and_then(|value| match value {
        Value::Bool(value) => Some(*value),
        Value::String(value)
            if value.eq_ignore_ascii_case("null") || value.eq_ignore_ascii_case("none") =>
        {
            None
        }
        Value::String(value) => Some(!value.is_empty() && !value.eq_ignore_ascii_case("false")),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_bare_and_wrapped_provider_shapes() {
        let record = json!({
            "avatar_id": "avtr_1",
            "display_name": "Test Avatar",
            "author": { "vrc_id": "usr_1", "name": "Alice" },
            "avatar_description": "Description",
            "thumbnail_image_url": "https://example.test/a.png",
            "avatar_performance": {
                "pc_rating": "Medium",
                "quest": "Poor",
                "has_impostor": "true",
                "security_variant": false
            }
        });
        for response in [
            json!([record.clone()]),
            json!({"results": [record.clone()]}),
        ] {
            let parsed = parse_vrcx_results(&response).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].id, "avtr_1");
            assert_eq!(parsed[0].author_id.as_deref(), Some("usr_1"));
            assert_eq!(parsed[0].author_name.as_deref(), Some("Alice"));
            assert_eq!(
                parsed[0]
                    .performance
                    .as_ref()
                    .unwrap()
                    .android_rating
                    .as_deref(),
                Some("Poor")
            );
            assert!(parsed[0].performance.as_ref().unwrap().has_impostor);
        }
    }

    #[test]
    fn skips_odd_records_and_rejects_unknown_top_level_shapes() {
        let parsed = parse_vrcx_results(&json!([null, 3, {}, {"id": "avtr_ok"}])).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "avtr_ok");
        assert!(parse_vrcx_results(&json!({"message": "maintenance"})).is_err());
    }
}
