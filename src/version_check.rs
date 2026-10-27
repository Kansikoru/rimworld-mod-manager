//! Checks of the active mod list against the installed game version (1.6 Unity / 1.7 Waylith).
//!
//! Ludeon: mods made for 1.6 work fully or partially on 1.7, so a missing "1.7" in
//! supportedVersions is only a soft hint. Hard problems are listed separately.

use crate::game_install::GameEngine;
use crate::models::ModInfo;

/// Whether a mod declares support for `game_mm` ("1.7"). None when it can't be told
/// (version unknown, mod lists no versions, or an official Core/DLC package).
pub fn supports_version(m: &ModInfo, game_mm: Option<&str>) -> Option<bool> {
    let game_mm = game_mm?;
    if m.supported_versions.is_empty() || m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
        return None;
    }
    Some(m.supported_versions.iter().any(|v| v.trim() == game_mm))
}

/// Newest version the mod declares, e.g. "1.6".
pub fn newest_supported(m: &ModInfo) -> Option<String> {
    let parse = |v: &str| -> Option<(u32, u32)> {
        let mut it = v.trim().split('.');
        Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    };
    m.supported_versions
        .iter()
        .filter_map(|v| parse(v).map(|p| (p, v.trim().to_string())))
        .max_by_key(|(p, _)| *p)
        .map(|(_, v)| v)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionIssue {
    /// Prepatcher can't hook the 1.7 engine until it is updated (Ludeon's 1.7 modder primer).
    PrepatcherOnWaylith,
    /// RimWorld 1.7 requires Harmony 3; this Harmony copy does not declare 1.7.
    HarmonyNotUpdated,
}

/// Hard problems of the active list for the installed engine.
pub fn active_list_issues(
    active: &[String],
    all_mods: &[ModInfo],
    engine: GameEngine,
    game_mm: Option<&str>,
) -> Vec<VersionIssue> {
    let mut issues = Vec::new();
    if engine != GameEngine::Waylith {
        return issues;
    }
    let is_active = |id: &str| active.iter().any(|a| a.eq_ignore_ascii_case(id));
    if is_active("zetrith.prepatcher") {
        issues.push(VersionIssue::PrepatcherOnWaylith);
    }
    if is_active("brrainz.harmony") {
        let harmony = all_mods
            .iter()
            .find(|m| m.package_id.eq_ignore_ascii_case("brrainz.harmony"));
        if let Some(h) = harmony {
            if supports_version(h, game_mm) == Some(false) {
                issues.push(VersionIssue::HarmonyNotUpdated);
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, versions: &[&str]) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            supported_versions: versions.iter().map(|v| v.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn version_support() {
        let old = m("a.b", &["1.4", "1.5", "1.6"]);
        assert_eq!(supports_version(&old, Some("1.7")), Some(false));
        assert_eq!(supports_version(&old, Some("1.6")), Some(true));
        assert_eq!(supports_version(&old, None), None);
        assert_eq!(supports_version(&m("ludeon.rimworld", &["1.6"]), Some("1.7")), None);
        assert_eq!(
            newest_supported(&m("a.b", &["1.10", "1.9", "1.6"])).as_deref(),
            Some("1.10")
        );
    }

    #[test]
    fn waylith_issues() {
        let mods = vec![m("brrainz.harmony", &["1.6"]), m("zetrith.prepatcher", &["1.6"])];
        let active = vec!["zetrith.prepatcher".to_string(), "brrainz.harmony".to_string()];
        assert_eq!(
            active_list_issues(&active, &mods, GameEngine::Waylith, Some("1.7")),
            vec![VersionIssue::PrepatcherOnWaylith, VersionIssue::HarmonyNotUpdated]
        );
        assert!(active_list_issues(&active, &mods, GameEngine::Unity, Some("1.6")).is_empty());
    }
}
