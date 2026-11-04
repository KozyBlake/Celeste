//! Deciding whether a published release is newer than this build.
//!
//! Celeste ships as a single binary that people download once and keep, so
//! most installs only learn about a release by someone noticing one. A copy
//! that quietly stops working is bad; a copy that quietly stops working on a
//! version whose fix shipped three releases ago is worse.
//!
//! The comparison lives here, apart from the HTTP call that fetches the tag,
//! because version ordering is where this kind of feature actually goes
//! wrong. `0.10.0` sorts *before* `0.9.0` as text, and a checker that
//! announces a downgrade is worse than one that says nothing at all. So the
//! rule throughout is: when the answer is not clear, stay quiet.

/// A Celeste version: `MAJOR.MINOR.PATCH`, optionally with a pre-release
/// suffix, as in `0.4.19-alpha-unreleased`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Dot-separated numbers, compared left to right. A missing component
    /// counts as zero, so `1.0` and `1.0.0` are the same version.
    numbers: Vec<u64>,
    /// Everything after the first `-`. A build carrying one is *older* than
    /// the same numbers without it, which is semver's rule and also the one
    /// that matters here: `0.4.19-alpha` must not look newer than `0.4.19`.
    pre_release: Option<String>,
}

impl Version {
    /// Reads a version from a release tag (`v0.11.0`) or a plain version
    /// string (`0.11.0`). `None` when it is not a version at all, which is
    /// the signal to say nothing rather than to guess.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let (numbers, pre_release) = match text.split_once('-') {
            Some((numbers, rest)) if !rest.is_empty() => (numbers, Some(rest.to_string())),
            _ => (text, None),
        };
        let numbers: Option<Vec<u64>> = numbers
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect();
        let numbers = numbers?;
        if numbers.is_empty() {
            return None;
        }
        Some(Version {
            numbers,
            pre_release,
        })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let width = self.numbers.len().max(other.numbers.len());
        for index in 0..width {
            let mine = self.numbers.get(index).copied().unwrap_or(0);
            let theirs = other.numbers.get(index).copied().unwrap_or(0);
            match mine.cmp(&theirs) {
                std::cmp::Ordering::Equal => {}
                unequal => return unequal,
            }
        }
        // Same numbers: a release outranks a pre-release of itself.
        match (&self.pre_release, &other.pre_release) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(mine), Some(theirs)) => mine.cmp(theirs),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Whether `latest_tag` is a release worth telling this build about.
///
/// False whenever either side cannot be read as a version. A checker that
/// cannot parse what it fetched knows nothing, and saying nothing is the
/// correct output of knowing nothing.
pub fn is_update(latest_tag: &str, current: &str) -> bool {
    match (Version::parse(latest_tag), Version::parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_release_is_an_update() {
        assert!(is_update("v0.11.0", "0.10.0"));
        assert!(is_update("0.10.1", "0.10.0"));
        assert!(is_update("v1.0.0", "0.99.99"));
    }

    #[test]
    fn the_same_version_is_not_an_update() {
        assert!(!is_update("v0.10.0", "0.10.0"));
        assert!(!is_update("0.10.0", "0.10.0"));
    }

    /// The bug every naive checker has: as text, `0.9.0` sorts after
    /// `0.10.0`, so a string comparison tells everyone running the newer
    /// build to downgrade.
    #[test]
    fn an_older_release_is_never_an_update_even_though_it_sorts_later() {
        assert!("v0.9.0" > "v0.10.0", "the text comparison really is wrong");
        assert!(!is_update("v0.9.0", "0.10.0"));
        assert!(!is_update("v0.4.19", "0.10.0"));
    }

    #[test]
    fn a_missing_component_counts_as_zero() {
        assert!(!is_update("v1.0", "1.0.0"));
        assert!(is_update("v1.0.1", "1.0"));
    }

    /// `0.4.19-alpha-unreleased` is the shape Celeste's own in-progress
    /// builds have carried, and it must not look newer than `0.4.19`.
    #[test]
    fn a_pre_release_is_older_than_the_release_it_precedes() {
        assert!(!is_update("v0.11.0-alpha", "0.11.0"));
        assert!(is_update("v0.11.0", "0.11.0-alpha"));
        assert!(is_update("v0.11.0-alpha", "0.10.0"));
    }

    /// A tag that is not a version, an error page parsed as JSON, or an
    /// empty string all mean the same thing: nothing is known.
    #[test]
    fn nonsense_is_never_an_update() {
        assert!(!is_update("", "0.10.0"));
        assert!(!is_update("latest", "0.10.0"));
        assert!(!is_update("v", "0.10.0"));
        assert!(!is_update("v0.x.0", "0.10.0"));
        assert!(!is_update("v0.11.0", "not-a-version"));
    }

    #[test]
    fn tags_parse_with_or_without_the_v() {
        assert_eq!(Version::parse("v0.10.0"), Version::parse("0.10.0"));
    }
}
