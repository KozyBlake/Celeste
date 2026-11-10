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
/// Who to contact about requests this client makes -- VRChat, and every
/// avatar database the search commands query. It names whoever runs this
/// fork, not Scarlet's author -- a question about this client's traffic
/// reaching Vinyarion would be a question he cannot answer. Credit for
/// Scarlet lives in CREDITS.md, which is what that is for.
pub const DEV_DISCORD: &str = "Discord:@chieriontop/656328060701900836";
pub const COMMUNITY_DISCORD_URL: &str = "https://discord.gg/shXBVdppW";
/// A route that does not require Discord. Every other contact in the
/// User-Agent needs an account on one service, and an operator who wants
/// to say "throttle or I block you" will not make one in order to say it.
/// The address is on a domain this project controls, so it can be pointed
/// somewhere else without shipping a release to change the header.
pub const CONTACT_EMAIL: &str = "chieriontop@chierion.top";

pub const FORK_GITHUB_URL: &str = "https://github.com/KozyBlake/Celeste";
pub const GITHUB_URL: &str = FORK_GITHUB_URL;

pub const USER_AGENT_NAME: &str = "KozyBlake-Celeste";
/// The licence file itself rather than GitHub's licence tab: PolyForm
/// Shield is not a licence GitHub detects, so that tab shows nothing.
pub const LICENSE_URL: &str = "https://github.com/KozyBlake/Celeste/blob/main/LICENSE";
pub const API_VERSION: &str = "api/1";
pub const API_HOST_0: &str = "vrchat.com";
pub const API_HOST_1: &str = "api.vrchat.com";
pub const API_HOST_2: &str = "api.vrchat.cloud";

/// The identity every outbound request carries, in the shape RFC 9110
/// describes: one `product/version`, then a comment holding the contacts.
/// The previous form put them as bare text after the product, where `:`,
/// `@` and `;` are not legal token characters -- readable to a person, but
/// something a strict parser or a WAF may cut at the first `;`, which
/// leaves the client looking unidentified again. The URLs carry the `+`
/// prefix crawler operators already recognise.
///
/// `USER_AGENT_NAME` is the part a third party should match on. The
/// version changes every release, so an allowlist or rate-limit rule built
/// on the whole string breaks at the next one.
pub fn user_agent() -> String {
    format!(
        "{USER_AGENT_NAME}/{VERSION} (+{GITHUB_URL}; mailto:{CONTACT_EMAIL}; {DEV_DISCORD}; {COMMUNITY_DISCORD_URL})"
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

    /// VRChat reads this header to find whoever runs the client. It spent
    /// a release naming a repository that 404s, which is the same as
    /// giving them nothing, and nothing noticed.
    #[test]
    fn the_contact_details_point_somewhere_that_exists() {
        let ua = user_agent();
        assert!(
            !ua.contains("ScarletRust"),
            "the fork's repository was renamed to Celeste; ScarletRust 404s: {ua}"
        );
        assert!(ua.contains("github.com/KozyBlake/Celeste"), "{ua}");
        assert!(
            !LICENSE_URL.contains("MIT"),
            "Celeste is PolyForm Shield 1.0.0, not MIT: {LICENSE_URL}"
        );
    }

    /// The whole point of the address: an operator who wants to reach
    /// whoever runs this client should not have to join a Discord server
    /// first in order to do it.
    #[test]
    fn there_is_a_contact_that_is_not_discord() {
        let ua = user_agent();
        assert!(ua.contains("mailto:"), "no non-Discord contact in: {ua}");
        assert!(
            CONTACT_EMAIL.contains('@') && !CONTACT_EMAIL.contains(' '),
            "not an address: {CONTACT_EMAIL}"
        );
    }

    /// The contacts belong inside a parenthesised comment, because `:`,
    /// `@` and `;` are not legal in an RFC 9110 product token. Left
    /// outside, a strict parser may keep only the text before the first
    /// `;` -- which is the product, and nothing anyone could act on.
    #[test]
    fn the_contacts_sit_inside_a_comment() {
        let ua = user_agent();
        let (product, comment) = ua
            .split_once(' ')
            .expect("a product, then a space, then the comment");
        assert!(!product.contains(';'), "product carries a `;`: {product}");
        assert!(comment.starts_with('('), "comment is unopened: {ua}");
        assert!(comment.ends_with(')'), "comment is unclosed: {ua}");
    }
}
