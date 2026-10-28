//! Workshop IDs of mods by package ID (D1).
//!
//! The game writes Workshop ID 0 for many mods into saves and mod lists, so "subscribe to
//! the missing mods" can't find them. RimSort's Steam Workshop Database (CC0,
//! https://github.com/RimSort/Steam-Workshop-Database) maps every Workshop item to its
//! package ID. It is about 47 MB, so it is downloaded only on request, read as a stream,
//! and kept as a small index next to the settings.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

pub const STEAM_DB_URL: &str = "https://raw.githubusercontent.com/RimSort/Steam-Workshop-Database/main/steamDB.json";

/// Package ID -> the Workshop items that carry it, and dependencies of Workshop items.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkshopIndex {
    /// The database's own version (unix time of its build).
    pub version: u64,
    /// Lowercase package ID -> (Workshop ID, mod name), sorted by Workshop ID.
    pub by_package: HashMap<String, Vec<(String, String)>>,
    /// Workshop ID -> required Workshop IDs (`dependencies` from `steamDB.json`).
    #[serde(default)]
    pub dependencies: HashMap<String, Vec<String>>,
    /// Workshop ID -> (lowercase package ID, mod name).
    #[serde(default)]
    pub by_id: HashMap<String, (String, String)>,
}

impl WorkshopIndex {
    /// The Workshop item for a package ID. When several items share it (re-uploads, forks),
    /// the one named like `name_hint` is chosen; without a clear match there is no answer,
    /// so nothing is ever subscribed to by a guess.
    pub fn lookup(&self, package_id: &str, name_hint: Option<&str>) -> Option<&str> {
        let items = self.by_package.get(&package_id.to_lowercase())?;
        if let [(id, _)] = items.as_slice() {
            return Some(id);
        }
        let hint = normalize(name_hint?);
        let mut named = items.iter().filter(|(_, name)| normalize(name) == hint);
        match (named.next(), named.next()) {
            (Some((id, _)), None) => Some(id),
            _ => None,
        }
    }

    /// A package ID's display name from the database, if known.
    pub fn name_for_package(&self, package_id: &str) -> Option<&str> {
        self.by_package
            .get(&package_id.to_lowercase())?
            .first()
            .map(|(_, name)| name.as_str())
            .filter(|n| !n.is_empty())
    }

    /// A Workshop item's display name from the database, if known.
    pub fn name_for_id(&self, workshop_id: &str) -> Option<&str> {
        self.by_id
            .get(workshop_id)
            .map(|(_, name)| name.as_str())
            .filter(|n| !n.is_empty())
    }

    /// A Workshop item's lowercase package ID from the database, if known.
    pub fn package_for_id(&self, workshop_id: &str) -> Option<&str> {
        self.by_id
            .get(workshop_id)
            .map(|(pkg, _)| pkg.as_str())
            .filter(|p| !p.is_empty())
    }

    /// Direct Workshop dependencies of an item: `(workshop_id, display_name)`.
    pub fn dependencies_of(&self, workshop_id: &str) -> Vec<(String, String)> {
        let Some(deps) = self.dependencies.get(workshop_id) else {
            return Vec::new();
        };
        deps.iter()
            .map(|dep_id| {
                let name = self.name_for_id(dep_id).unwrap_or(dep_id).to_string();
                (dep_id.clone(), name)
            })
            .collect()
    }

    /// All Workshop dependencies of an item (transitive, in breadth-first order).
    pub fn all_dependencies_of(&self, workshop_id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        seen.insert(workshop_id.to_string());
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(workshop_id.to_string());
        while let Some(cur) = queue.pop_front() {
            if let Some(deps) = self.dependencies.get(&cur) {
                for dep in deps {
                    if seen.insert(dep.clone()) {
                        out.push(dep.clone());
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.by_package.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_package.is_empty()
    }
}

/// Names compared without case and extra spaces.
fn normalize(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[derive(Deserialize)]
struct RawDb {
    #[serde(default)]
    version: u64,
    database: HashMap<String, RawEntry>,
}

/// Only the fields the index needs; the rest (versions...) is skipped.
#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "packageId")]
    package_id: Option<String>,
    /// Core and DLC are Steam apps, not Workshop items: they can't be subscribed to.
    #[serde(default)]
    appid: bool,
    name: Option<String>,
    #[serde(rename = "steamName")]
    steam_name: Option<String>,
    #[serde(default)]
    dependencies: HashMap<String, serde_json::Value>,
}

/// Builds the index from the database JSON (read as a stream).
pub fn build_index(reader: impl Read) -> Result<WorkshopIndex, String> {
    let raw: RawDb = serde_json::from_reader(std::io::BufReader::new(reader)).map_err(|e| e.to_string())?;
    let mut by_package: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut dependencies: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_id: HashMap<String, (String, String)> = HashMap::new();
    let app_ids: std::collections::HashSet<String> = raw
        .database
        .iter()
        .filter(|(_, e)| e.appid)
        .map(|(id, _)| id.clone())
        .collect();
    for (workshop_id, entry) in raw.database {
        if entry.appid || !crate::modlists::is_workshop_id(&workshop_id) {
            continue;
        }
        let mut deps: Vec<String> = entry
            .dependencies
            .into_keys()
            .filter(|id| crate::modlists::is_workshop_id(id) && !app_ids.contains(id))
            .collect();
        deps.sort();
        deps.dedup();
        if !deps.is_empty() {
            dependencies.insert(workshop_id.clone(), deps);
        }
        let Some(package_id) = entry.package_id.filter(|p| !p.trim().is_empty()) else {
            continue;
        };
        let pkg_lower = package_id.trim().to_lowercase();
        let name = entry.name.or(entry.steam_name).unwrap_or_default();
        by_id.insert(workshop_id.clone(), (pkg_lower.clone(), name.clone()));
        by_package.entry(pkg_lower).or_default().push((workshop_id, name));
    }
    for items in by_package.values_mut() {
        items.sort();
    }
    Ok(WorkshopIndex {
        version: raw.version,
        by_package,
        dependencies,
        by_id,
    })
}

pub fn index_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("workshop_ids.json")
}

/// The index saved earlier, if any.
pub fn load() -> Option<WorkshopIndex> {
    let text = std::fs::read_to_string(index_path()).ok()?;
    let mut index: WorkshopIndex = serde_json::from_str(&text).ok()?;
    if index.by_id.is_empty() && !index.by_package.is_empty() {
        for (pkg, items) in &index.by_package {
            for (ws_id, name) in items {
                index.by_id.insert(ws_id.clone(), (pkg.clone(), name.clone()));
            }
        }
    }
    Some(index)
}

fn save(index: &WorkshopIndex) -> Result<(), String> {
    let path = index_path();
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(index).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Downloads the database, builds the index and saves it (network, ~47 MB).
pub fn download() -> Result<WorkshopIndex, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(300)).build();
    let response = agent.get(STEAM_DB_URL).call().map_err(|e| e.to_string())?;
    let index = build_index(response.into_reader())?;
    if index.is_empty() {
        return Err("the database is empty".into());
    }
    save(&index)?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "version": 1790337009,
        "database": {
            "294100": { "appid": true, "packageId": "ludeon.rimworld", "name": "RimWorld" },
            "2009463077": { "packageId": "brrainz.harmony", "name": "Harmony", "gameVersions": ["1.6"] },
            "1111": { "packageId": "Some.Mod", "name": "Some Mod", "dependencies": { "2009463077": ["Harmony", "url"] } },
            "2222": { "packageId": "some.mod", "steamName": "Some Mod (Continued)" },
            "3333": { "name": "No package id" },
            "notanid": { "packageId": "bad.entry", "name": "Bad" }
        }
    }"#;

    #[test]
    fn builds_the_index_from_the_database() {
        let index = build_index(SAMPLE.as_bytes()).unwrap();
        assert_eq!(index.version, 1790337009);
        assert_eq!(
            index.len(),
            2,
            "Harmony and some.mod; apps and broken entries are left out"
        );
        assert_eq!(index.lookup("BrrAinz.Harmony", None), Some("2009463077"));
        assert_eq!(index.name_for_package("brrainz.harmony"), Some("Harmony"));
        assert_eq!(
            index.dependencies_of("1111"),
            vec![("2009463077".to_string(), "Harmony".to_string())]
        );
        assert_eq!(index.all_dependencies_of("1111"), vec!["2009463077".to_string()]);
        assert_eq!(
            index.lookup("ludeon.rimworld", None),
            None,
            "DLC and Core can't be subscribed to"
        );
    }

    #[test]
    fn several_items_need_a_matching_name() {
        let index = build_index(SAMPLE.as_bytes()).unwrap();
        assert_eq!(index.lookup("some.mod", None), None, "no guessing between re-uploads");
        assert_eq!(index.lookup("some.mod", Some("some  mod")), Some("1111"));
        assert_eq!(index.lookup("some.mod", Some("Some Mod (Continued)")), Some("2222"));
        assert_eq!(index.lookup("some.mod", Some("Other")), None);
    }

    #[test]
    fn index_survives_saving() {
        let index = build_index(SAMPLE.as_bytes()).unwrap();
        let back: WorkshopIndex = serde_json::from_str(&serde_json::to_string(&index).unwrap()).unwrap();
        assert_eq!(back, index);
    }

    /// Downloads the real database and checks it against the user's saves (run by hand:
    /// `cargo test --release live_workshop_db -- --ignored --nocapture`). Saves nothing.
    #[test]
    #[ignore]
    fn live_workshop_db() {
        let started = std::time::Instant::now();
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(300)).build();
        let response = agent.get(STEAM_DB_URL).call().unwrap();
        let index = build_index(response.into_reader()).unwrap();
        println!(
            "{} package IDs, version {}, {:?}",
            index.len(),
            index.version,
            started.elapsed()
        );
        assert!(index.len() > 1000);

        let saves_dir = PathBuf::from(std::env::var("USERPROFILE").unwrap())
            .join(r"AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Saves");
        for save in crate::saves::list_saves(&saves_dir) {
            let no_id: Vec<&String> = save.ids.iter().filter(|id| save.steam_id(id).is_none()).collect();
            let found = no_id
                .iter()
                .filter(|id| index.lookup(id, save.name_of(id)).is_some())
                .count();
            println!(
                "{}: {} mods, {} without a Workshop ID in the save, {} found in the index",
                save.name,
                save.ids.len(),
                no_id.len(),
                found
            );
        }
    }
}
