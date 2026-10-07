//! Port of the app-identity constants from `net.sybyline.scarlet.Scarlet`
//! (lines ~156-198 of the Java source). `Scarlet.java` is a 3,655-line
//! god-class mixing these constants with executors, Swing bootstrap, and
//! data-folder migration; only the plain constant block is ported here so
//! far — see ../../PORTING.md for what's still outstanding from that file
//! (data-dir resolution/migration, the settings/executor wiring).

pub const GROUP: &str = "KozyBlake";
/// The organization name used by upstream Scarlet. Data discovery keeps
/// this because community installs can still
/// live under `SybylineNetwork/Scarlet` on Windows or Linux.
pub const UPSTREAM_GROUP: &str = "SybylineNetwork";
pub const NAME: &str = "Celeste";
/// The name this project used while it was still based directly on Scarlet.
/// Kept because anything that has to *find* an existing install — the
/// data-folder discovery, the log-target rewrite, the log-file pattern —
/// must still recognise it.
pub const LEGACY_NAME: &str = "Scarlet";
/// Java resolves this from the jar manifest's `Implementation-Version` at
/// runtime, falling back to this literal. Rust has no manifest equivalent,
/// so this constant (kept in sync with `Cargo.toml`'s workspace version) is
/// authoritative here.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "KozyBlake/Celeste";
/// Freedesktop/Wayland application identity. This must match the installed
/// `.desktop` file name so compositors can associate Celeste with its icon.
pub const DESKTOP_APP_ID: &str = "io.github.KozyBlake.Celeste";
pub const FORK_GROUP: &str = GROUP;
/// The current GitHub repository name.
pub const FORK_REPOSITORY: &str = NAME;
pub const FORK_NOTE: &str =
    "Maintained by KozyBlake \u{2014} Based on Scarlet \u{2014} Windows & Linux";
pub const DEV_DISCORD: &str = "Discord:@vinyarion/Vinyarion#0292/393412191547555841";
pub const COMMUNITY_DISCORD_URL: &str = "https://discord.gg/CP3AyhypBF";

pub const FORK_GITHUB_URL: &str = "https://github.com/KozyBlake/ScarletRust";
pub const GITHUB_URL: &str = FORK_GITHUB_URL;

pub const USER_AGENT_NAME: &str = "KozyBlake-Celeste";
pub const LICENSE_URL: &str = "https://github.com/KozyBlake/ScarletRust?tab=MIT-1-ov-file";
pub const API_VERSION: &str = "api/1";
pub const API_HOST_0: &str = "vrchat.com";
pub const API_HOST_1: &str = "api.vrchat.com";
pub const API_HOST_2: &str = "api.vrchat.cloud";

pub fn user_agent() -> String {
    format!(
        "{USER_AGENT_NAME}/{VERSION} {DEV_DISCORD}; {COMMUNITY_DISCORD_URL}; {GITHUB_URL}"
    )
}

pub fn api_base(host: &str) -> String {
    format!("https://{host}/{API_VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_matches_java_format() {
        assert_eq!(api_base(API_HOST_0), "https://vrchat.com/api/1");
        assert_eq!(api_base(API_HOST_1), "https://api.vrchat.com/api/1");
        assert_eq!(api_base(API_HOST_2), "https://api.vrchat.cloud/api/1");
    }

    #[test]
    fn user_agent_contains_identity() {
        let ua = user_agent();
        assert!(ua.starts_with("KozyBlake-Celeste/"));
        assert!(ua.contains(GITHUB_URL));
    }
}
