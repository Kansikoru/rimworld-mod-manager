//! "New version available" badge (F2). Asks GitHub for the latest release at most once a day,
//! in the background; without a network it silently shows nothing. It never downloads or
//! replaces the exe: the badge only opens the release page.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/Kansikoru/rimworld-mod-manager/releases/latest";
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestRelease {
    /// Release tag, e.g. "v2.1.0".
    pub tag: String,
    /// Release page on GitHub.
    pub url: String,
}

/// Result of the last check, so the badge shows at once and GitHub is asked only once a day.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCache {
    /// Unix time of the last successful check.
    pub checked_at: u64,
    pub latest: Option<LatestRelease>,
}

pub fn cache_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("update_check.json")
}

pub fn load_cache(path: &Path) -> UpdateCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_cache(path: &Path, cache: &UpdateCache) {
    if let Ok(text) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(path, text);
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A day has passed since the last check (or the clock went back).
pub fn needs_check(cache: &UpdateCache, now: u64) -> bool {
    cache.checked_at > now || now - cache.checked_at >= CHECK_EVERY_SECS
}

/// "v2.0.10" -> [2, 0, 10]. A suffix such as "-beta" is ignored.
pub fn parse_version(s: &str) -> Option<Vec<u32>> {
    let core = s.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    core.split('.').map(|part| part.parse().ok()).collect()
}

/// Compares numbers, not text: 2.0.10 is newer than 2.0.9, and 2.1 equals 2.1.0.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse_version(latest), parse_version(current)) else {
        return false;
    };
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn fetch_latest() -> Result<LatestRelease, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let text = agent
        .get(LATEST_RELEASE_API)
        // GitHub refuses requests without a User-Agent.
        .set(
            "User-Agent",
            concat!("rimworld-mod-manager/", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().ok_or("no tag_name")?.to_string();
    let url = json["html_url"].as_str().ok_or("no html_url")?.to_string();
    Ok(LatestRelease { tag, url })
}

/// Asks GitHub (background thread) and remembers the answer. `None` when offline.
pub fn check_now(cache_file: &Path) -> Option<LatestRelease> {
    let latest = fetch_latest().ok()?;
    save_cache(
        cache_file,
        &UpdateCache {
            checked_at: now_secs(),
            latest: Some(latest.clone()),
        },
    );
    Some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(parse_version("v2.0.10"), Some(vec![2, 0, 10]));
        assert_eq!(parse_version("2.1.0-beta"), Some(vec![2, 1, 0]));
        assert_eq!(parse_version("latest"), None);
        assert!(is_newer("v2.0.10", "2.0.9"));
        assert!(is_newer("v2.1", "2.0.4"));
        assert!(!is_newer("v2.0.4", "2.0.4"));
        assert!(!is_newer("v2.1", "2.1.0"));
        assert!(!is_newer("v2.0.3", "2.0.4"));
        assert!(!is_newer("nightly", "2.0.4"));
    }

    #[test]
    fn checks_once_a_day() {
        let day = 24 * 60 * 60;
        let cache = UpdateCache {
            checked_at: 1_000_000,
            latest: None,
        };
        assert!(!needs_check(&cache, 1_000_000 + day - 1));
        assert!(needs_check(&cache, 1_000_000 + day));
        assert!(needs_check(&cache, 999_000), "clock went back");
        assert!(needs_check(&UpdateCache::default(), 1_000_000), "never checked");
    }

    #[test]
    fn cache_roundtrip() {
        let path = std::env::temp_dir().join(format!("rwmm_update_{}.json", std::process::id()));
        let cache = UpdateCache {
            checked_at: 42,
            latest: Some(LatestRelease {
                tag: "v9.9.9".into(),
                url: "https://example.org".into(),
            }),
        };
        save_cache(&path, &cache);
        assert_eq!(load_cache(&path), cache);
        let _ = std::fs::remove_file(&path);
        assert_eq!(load_cache(&path), UpdateCache::default());
    }
}
