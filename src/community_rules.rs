//! Load order rules (S14): ~50 built-in rules + the community database used by RimSort/RimPy
//! (`communityRules.json`, CC0). The community file is downloaded or imported once and kept in
//! `%LOCALAPPDATA%\RimWorldModManager\community_rules.json`.
//!
//! RimSort/RimPy format:
//! `{"timestamp": .., "rules": {"<packageId>": {"loadBefore": {"<id>": {"name": [..]}},
//!   "loadAfter": {..}, "loadBottom": {"value": true}}}}`

use crate::i18n::tr;
use crate::models::CommunityRule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_RULES_JSON: &str = include_str!("../data/default_community_rules.json");
static DEFAULT_DB: OnceLock<CommunityDatabase> = OnceLock::new();

/// Public RimSort community rules (CC0-1.0).
pub const RIMSORT_RULES_URL: &str =
    "https://raw.githubusercontent.com/RimSort/Community-Rules-Database/main/communityRules.json";

/// Tier given to `loadBottom` mods (the sorter puts tier 11 last).
const BOTTOM_TIER: u8 = 11;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommunityDatabase {
    pub rules: HashMap<String, CommunityRule>,
}

/// Built-in rules only.
pub fn get_default_community_db() -> &'static CommunityDatabase {
    DEFAULT_DB.get_or_init(|| {
        let parsed: HashMap<String, CommunityRule> = serde_json::from_str(DEFAULT_RULES_JSON).unwrap_or_default();
        CommunityDatabase {
            rules: parsed.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect(),
        }
    })
}

pub fn local_rules_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("community_rules.json")
}

fn ids_of(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Object(map)) => map.keys().map(|k| k.to_lowercase()).collect(),
        Some(serde_json::Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str().map(str::to_lowercase)).collect(),
        _ => Vec::new(),
    }
}

/// Parses the RimSort/RimPy format, or this app's own format (map of `CommunityRule`).
pub fn parse_rules_json(text: &str) -> Result<HashMap<String, CommunityRule>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| tr!("не JSON: {}", "not JSON: {}", e))?;

    if let Some(rules) = value.get("rules").and_then(|r| r.as_object()) {
        let mut out = HashMap::with_capacity(rules.len());
        for (id, rule) in rules {
            let bottom = rule
                .get("loadBottom")
                .and_then(|b| b.get("value"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.insert(
                id.to_lowercase(),
                CommunityRule {
                    tier: bottom.then_some(BOTTOM_TIER),
                    load_before: ids_of(rule.get("loadBefore")),
                    load_after: ids_of(rule.get("loadAfter")),
                    ..Default::default()
                },
            );
        }
        return Ok(out);
    }

    let own: HashMap<String, CommunityRule> = serde_json::from_value(value).map_err(|_| {
        tr!(
            "неизвестный формат: нет ключа \"rules\"",
            "unknown format: no \"rules\" key"
        )
        .to_string()
    })?;
    Ok(own.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect())
}

/// Adds `extra` to `base`: lists are merged without duplicates, an explicit tier wins.
pub fn merge_rules(base: &mut HashMap<String, CommunityRule>, extra: HashMap<String, CommunityRule>) {
    fn union(into: &mut Vec<String>, from: Vec<String>) {
        for id in from {
            if !into.iter().any(|x| x.eq_ignore_ascii_case(&id)) {
                into.push(id);
            }
        }
    }
    for (id, rule) in extra {
        let entry = base.entry(id).or_default();
        if rule.tier.is_some() {
            entry.tier = rule.tier;
        }
        union(&mut entry.load_before, rule.load_before);
        union(&mut entry.load_after, rule.load_after);
        union(&mut entry.force_load_before, rule.force_load_before);
        union(&mut entry.force_load_after, rule.force_load_after);
        union(&mut entry.incompatible, rule.incompatible);
    }
}

/// Built-in rules + the community file at `path` (if present and valid).
pub fn load_effective_db_from(path: &Path) -> CommunityDatabase {
    let mut rules = get_default_community_db().rules.clone();
    if let Some(extra) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| parse_rules_json(&t).ok())
    {
        merge_rules(&mut rules, extra);
    }
    CommunityDatabase { rules }
}

pub fn load_effective_db() -> CommunityDatabase {
    load_effective_db_from(&local_rules_path())
}

/// Validates and stores community rules text; returns how many rules it has.
fn store_rules_text(text: &str, dest: &Path) -> Result<usize, String> {
    let count = parse_rules_json(text)?.len();
    if count == 0 {
        return Err(tr!("файл не содержит правил", "the file has no rules").to_string());
    }
    let tmp = dest.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(count)
}

/// Imports a communityRules.json from disk (e.g. from a RimPy or RimSort folder).
pub fn import_rules_file(src: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(src).map_err(|e| format!("{}: {}", src.display(), e))?;
    store_rules_text(&text, &local_rules_path())
}

/// Downloads the RimSort community rules (blocking — call from a background task).
pub fn download_rules() -> Result<usize, String> {
    let text = ureq::get(RIMSORT_RULES_URL)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| tr!("нет связи с GitHub: {}", "can't reach GitHub: {}", e))?
        .into_string()
        .map_err(|e| e.to_string())?;
    store_rules_text(&text, &local_rules_path())
}

/// Where a RimPy install keeps its copy of the rules, if one is found on the desktop.
pub fn find_rimpy_rules_dir() -> Option<PathBuf> {
    let desktop = PathBuf::from(std::env::var_os("USERPROFILE")?).join("Desktop");
    std::fs::read_dir(&desktop)
        .ok()?
        .flatten()
        .map(|e| e.path().join("database"))
        .find(|p| p.join("communityRules.json").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIMSORT_SAMPLE: &str = r#"{
      "timestamp": 1789239946,
      "rules": {
        "Automatic.StartupImpact": {
          "loadBefore": { "unlimitedhugs.hugslib": { "name": ["HugsLib"] } },
          "loadAfter": { "brrainz.harmony": { "name": ["Harmony"], "comment": ["below harmony"] } }
        },
        "majorhoff.rimthreaded": { "loadBottom": { "value": true } }
      }
    }"#;

    #[test]
    fn parses_rimsort_format() {
        let rules = parse_rules_json(RIMSORT_SAMPLE).unwrap();
        let r = &rules["automatic.startupimpact"];
        assert_eq!(r.load_before, vec!["unlimitedhugs.hugslib"]);
        assert_eq!(r.load_after, vec!["brrainz.harmony"]);
        assert_eq!(r.tier, None);
        assert_eq!(rules["majorhoff.rimthreaded"].tier, Some(BOTTOM_TIER));
    }

    /// Real data check: `cargo test real_rules_smoke -- --ignored --nocapture`
    /// (parses RimPy's local copy if present, downloads RimSort's, sorts the local active list).
    #[test]
    #[ignore]
    fn real_rules_smoke() {
        if let Some(dir) = find_rimpy_rules_dir() {
            let text = std::fs::read_to_string(dir.join("communityRules.json")).unwrap();
            println!("RimPy rules: {}", parse_rules_json(&text).unwrap().len());
        }
        let text = ureq::get(RIMSORT_RULES_URL).call().unwrap().into_string().unwrap();
        let rimsort = parse_rules_json(&text).unwrap();
        let bottom = rimsort.values().filter(|r| r.tier == Some(BOTTOM_TIER)).count();
        println!("RimSort rules: {} (loadBottom: {})", rimsort.len(), bottom);

        let mut rules = get_default_community_db().rules.clone();
        merge_rules(&mut rules, rimsort);
        let db = CommunityDatabase { rules };
        let paths = crate::steam::detect_steam_paths();
        let mods = crate::scanner::scan_mods(
            paths.workshop_path.as_deref(),
            paths.local_mods_path.as_deref(),
            paths.game_data_path.as_deref(),
        );
        let ids: Vec<String> = mods.iter().map(|m| m.package_id.clone()).collect();
        let t = std::time::Instant::now();
        let sorted = crate::sorter::sort_load_order_with_db(&ids, &mods, Some(&db));
        println!(
            "sorted {} mods with {} rules in {:?}",
            sorted.len(),
            db.rules.len(),
            t.elapsed()
        );
        println!("first 8: {:?}", &sorted[..sorted.len().min(8)]);
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn parses_own_format_and_merges() {
        let own = parse_rules_json(DEFAULT_RULES_JSON).unwrap();
        assert!(own.len() > 40);
        let mut base = own.clone();
        let extra = parse_rules_json(RIMSORT_SAMPLE).unwrap();
        merge_rules(&mut base, extra);
        assert_eq!(base.len(), own.len() + 2);
        // merging the same rules again adds no duplicates
        merge_rules(&mut base, parse_rules_json(RIMSORT_SAMPLE).unwrap());
        assert_eq!(base["automatic.startupimpact"].load_after.len(), 1);
        assert!(parse_rules_json("not json").is_err());
        assert!(parse_rules_json("{\"x\": 1}").is_err());
    }
}
