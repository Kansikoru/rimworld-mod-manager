//! Mod updates (F10): which installed Workshop mods are older than the Workshop version.
//!
//! The Workshop side comes from Steam's public `GetPublishedFileDetails` (no key needed):
//! `time_updated` and the version tags ("1.6", "1.7"...). The installed side comes from
//! Steam's own record `appworkshop_294100.acf` (`WorkshopItemsInstalled/<id>/timeupdated`);
//! a game without Steam has no record, so the newest file date of the mod folder is used
//! instead (marked as an estimate).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DETAILS_URL: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
/// Items per request (Steam accepts more, but smaller requests fail less).
const BATCH: usize = 100;
/// File dates are rough: only a difference bigger than this counts as "outdated".
const FILE_DATE_SLACK: u64 = 24 * 60 * 60;

/// A Workshop item as Steam describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteItem {
    pub time_updated: u64,
    /// Game versions from the tags: "1.6", "1.7"...
    pub versions: Vec<String>,
    pub title: String,
}

/// One installed mod to check.
#[derive(Debug, Clone)]
pub struct CheckItem {
    pub package_id: String,
    pub workshop_id: String,
    pub dir: PathBuf,
    /// Versions the installed copy declares in About.xml.
    pub local_versions: Vec<String>,
}

/// What the check found for one mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModUpdate {
    pub workshop_id: String,
    /// The Workshop copy is newer than the installed one.
    pub outdated: bool,
    /// Compared by file dates (no Steam record), so only an estimate.
    pub estimated: bool,
    /// The Workshop copy supports the installed game version, the installed one doesn't:
    /// the author updated it, Steam hasn't downloaded it yet.
    pub newer_supports_game: bool,
    pub remote_time: u64,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateReport {
    /// By lowercase package ID; only mods with something to report.
    pub mods: HashMap<String, ModUpdate>,
    pub checked: usize,
    /// Items Steam doesn't know (removed from the Workshop, or hidden).
    pub not_found: usize,
    /// Unix time of the check.
    pub checked_at: u64,
}

impl UpdateReport {
    pub fn outdated_count(&self) -> usize {
        self.mods.values().filter(|u| u.outdated).count()
    }
}

/// Reads `publishedfiledetails` of a `GetPublishedFileDetails` answer: Workshop ID -> item.
pub fn parse_details(json: &str) -> Result<HashMap<String, RemoteItem>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = value["response"]["publishedfiledetails"]
        .as_array()
        .ok_or("no publishedfiledetails")?;
    let mut out = HashMap::new();
    for item in list {
        // result 1 = OK; 9 = not found or hidden.
        if item["result"].as_i64() != Some(1) {
            continue;
        }
        let Some(id) = item["publishedfileid"].as_str() else {
            continue;
        };
        let time_updated = item["time_updated"].as_u64().unwrap_or(0);
        let versions = item["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .filter_map(|t| t["tag"].as_str())
                    .filter(|t| is_version_tag(t))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let title = item["title"].as_str().unwrap_or_default().to_string();
        out.insert(
            id.to_string(),
            RemoteItem {
                time_updated,
                versions,
                title,
            },
        );
    }
    Ok(out)
}

/// "1.6", "1.7" — not "Mod" or other tags.
fn is_version_tag(tag: &str) -> bool {
    let mut parts = tag.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty() && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
    )
}

/// Asks Steam about the items, in batches.
pub fn fetch_details(ids: &[String]) -> Result<HashMap<String, RemoteItem>, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let mut out = HashMap::new();
    for chunk in ids.chunks(BATCH) {
        let mut form: Vec<(String, String)> = vec![("itemcount".into(), chunk.len().to_string())];
        form.extend(
            chunk
                .iter()
                .enumerate()
                .map(|(i, id)| (format!("publishedfileids[{i}]"), id.clone())),
        );
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let text = agent
            .post(DETAILS_URL)
            .send_form(&pairs)
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())?;
        out.extend(parse_details(&text)?);
    }
    Ok(out)
}

/// `...\steamapps\workshop\content\294100` -> `...\steamapps\workshop\appworkshop_294100.acf`.
pub fn acf_path(workshop_content_dir: &Path) -> Option<PathBuf> {
    let workshop = workshop_content_dir.parent()?.parent()?;
    let path = workshop.join("appworkshop_294100.acf");
    path.is_file().then_some(path)
}

/// `WorkshopItemsInstalled` of Steam's record: Workshop ID -> `timeupdated` of the copy on disk.
pub fn parse_acf_installed(text: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    let Some(start) = text.find("\"WorkshopItemsInstalled\"") else {
        return out;
    };
    // Simple walk over the VDF text: quoted strings and braces.
    let mut depth = 0usize;
    let mut current: Option<String> = None;
    let mut last_key: Option<String> = None;
    let mut chars = text[start + "\"WorkshopItemsInstalled\"".len()..].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                depth += 1;
                if depth == 2 {
                    current = last_key.take();
                }
            }
            '}' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
                if depth == 1 {
                    current = None;
                }
            }
            '"' => {
                let mut s = String::new();
                for ch in chars.by_ref() {
                    if ch == '"' {
                        break;
                    }
                    s.push(ch);
                }
                if depth == 2 && last_key.as_deref() == Some("timeupdated") {
                    if let (Some(id), Ok(t)) = (&current, s.parse::<u64>()) {
                        out.insert(id.clone(), t);
                    }
                    last_key = None;
                } else {
                    last_key = Some(s);
                }
            }
            _ => {}
        }
    }
    out
}

/// The newest modification time of the files in a mod folder (unix seconds).
pub fn newest_file_time(dir: &Path) -> Option<u64> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .max()
}

/// Compares installed copies with the Workshop. `installed` is Steam's record (may be empty);
/// `file_time` gives a folder's newest file date when Steam has no record of the item.
pub fn compare(
    items: &[CheckItem],
    remote: &HashMap<String, RemoteItem>,
    installed: &HashMap<String, u64>,
    game_version: Option<&str>,
    file_time: impl Fn(&Path) -> Option<u64>,
) -> UpdateReport {
    let mut report = UpdateReport {
        checked: items.len(),
        ..Default::default()
    };
    for item in items {
        let Some(online) = remote.get(&item.workshop_id) else {
            report.not_found += 1;
            continue;
        };
        let (outdated, estimated) = match installed.get(&item.workshop_id) {
            Some(&local) => (online.time_updated > local, false),
            None => match file_time(&item.dir) {
                Some(local) => (online.time_updated > local + FILE_DATE_SLACK, true),
                None => (false, true),
            },
        };
        // If Steam's record says the copy is the latest, a tag alone changes nothing:
        // downloading would bring the same files.
        let confirmed_current = !outdated && !estimated;
        let newer_supports_game = !confirmed_current
            && game_version
                .is_some_and(|v| online.versions.iter().any(|t| t == v) && !item.local_versions.iter().any(|t| t == v));
        if outdated || newer_supports_game {
            report.mods.insert(
                item.package_id.to_lowercase(),
                ModUpdate {
                    workshop_id: item.workshop_id.clone(),
                    outdated: outdated || newer_supports_game,
                    estimated,
                    newer_supports_game,
                    remote_time: online.time_updated,
                },
            );
        }
    }
    report.checked_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    report
}

/// The whole check (network): asks Steam about every item and compares.
pub fn check(
    items: Vec<CheckItem>,
    acf: Option<PathBuf>,
    game_version: Option<String>,
) -> Result<UpdateReport, String> {
    let ids: Vec<String> = {
        let mut ids: Vec<String> = items.iter().map(|i| i.workshop_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let remote = fetch_details(&ids)?;
    let installed = acf
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_acf_installed(&t))
        .unwrap_or_default();
    Ok(compare(
        &items,
        &remote,
        &installed,
        game_version.as_deref(),
        newest_file_time,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETAILS: &str = r#"{"response":{"result":1,"resultcount":3,"publishedfiledetails":[
        {"publishedfileid":"2009463077","result":1,"title":"Harmony","time_updated":1792065891,
         "tags":[{"tag":"Mod"},{"tag":"1.7"},{"tag":"1.6"}]},
        {"publishedfileid":"730936602","result":1,"title":"Achtung!","time_updated":1786739756,
         "tags":[{"tag":"Mod"},{"tag":"1.6"},{"tag":"1.5"}]},
        {"publishedfileid":"111","result":9}]}}"#;

    const ACF: &str = "\"AppWorkshop\"\n{\n\t\"appid\"\t\t\"294100\"\n\t\"WorkshopItemsInstalled\"\n\t{\n\t\t\"2009463077\"\n\t\t{\n\t\t\t\"size\"\t\t\"8306239\"\n\t\t\t\"timeupdated\"\t\t\"1792065891\"\n\t\t\t\"manifest\"\t\t\"376\"\n\t\t}\n\t\t\"730936602\"\n\t\t{\n\t\t\t\"size\"\t\t\"6311551\"\n\t\t\t\"timeupdated\"\t\t\"1780000000\"\n\t\t}\n\t}\n\t\"WorkshopItemDetails\"\n\t{\n\t\t\"2009463077\"\n\t\t{\n\t\t\t\"timeupdated\"\t\t\"1\"\n\t\t}\n\t}\n}";

    fn item(pkg: &str, ws: &str, versions: &[&str]) -> CheckItem {
        CheckItem {
            package_id: pkg.into(),
            workshop_id: ws.into(),
            dir: PathBuf::from(format!("mods/{ws}")),
            local_versions: versions.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn reads_steam_answer() {
        let items = parse_details(DETAILS).unwrap();
        assert_eq!(items.len(), 2, "not-found items are skipped");
        assert_eq!(items["2009463077"].versions, ["1.7", "1.6"]);
        assert_eq!(items["730936602"].time_updated, 1786739756);
        assert!(parse_details("not json").is_err());
    }

    #[test]
    fn reads_installed_times_from_steam_record_only() {
        let installed = parse_acf_installed(ACF);
        assert_eq!(installed.len(), 2);
        assert_eq!(
            installed["2009463077"], 1792065891,
            "WorkshopItemDetails is not mixed in"
        );
        assert_eq!(installed["730936602"], 1780000000);
        assert!(parse_acf_installed("\"AppWorkshop\" { }").is_empty());
    }

    #[test]
    fn finds_outdated_mods_and_new_version_support() {
        let remote = parse_details(DETAILS).unwrap();
        let installed = parse_acf_installed(ACF);
        let items = vec![
            item("brrainz.harmony", "2009463077", &["1.6", "1.7"]),
            item("brrainz.achtung", "730936602", &["1.6"]),
            item("gone.mod", "111", &[]),
        ];
        let report = compare(&items, &remote, &installed, Some("1.7"), |_| None);
        assert_eq!(report.checked, 3);
        assert_eq!(report.not_found, 1);
        assert!(!report.mods.contains_key("brrainz.harmony"), "up to date");
        let achtung = &report.mods["brrainz.achtung"];
        assert!(achtung.outdated && !achtung.estimated && !achtung.newer_supports_game);
        assert_eq!(report.outdated_count(), 1);
    }

    #[test]
    fn workshop_has_the_game_version_the_copy_lacks() {
        let remote = parse_details(DETAILS).unwrap();
        // Installed Harmony is older and declares only 1.6; the Workshop one has 1.7.
        let installed: HashMap<String, u64> = [("2009463077".to_string(), 1_700_000_000)].into();
        let items = vec![item("brrainz.harmony", "2009463077", &["1.6"])];
        let report = compare(&items, &remote, &installed, Some("1.7"), |_| None);
        assert!(report.mods["brrainz.harmony"].newer_supports_game);

        // The copy is the latest already: the tag alone is not reported.
        let current = parse_acf_installed(ACF);
        let report = compare(&items, &remote, &current, Some("1.7"), |_| None);
        assert!(report.mods.is_empty());
    }

    #[test]
    fn without_steam_record_file_dates_are_an_estimate() {
        let remote = parse_details(DETAILS).unwrap();
        let empty = HashMap::new();
        let items = vec![item("brrainz.achtung", "730936602", &["1.6"])];
        // Files a week older than the Workshop version: outdated (estimate).
        let old = compare(&items, &remote, &empty, None, |_| Some(1786739756 - 7 * 86400));
        assert!(old.mods["brrainz.achtung"].outdated && old.mods["brrainz.achtung"].estimated);
        // A few hours apart: copying shifts dates, so not counted.
        let close = compare(&items, &remote, &empty, None, |_| Some(1786739756 - 3600));
        assert!(close.mods.is_empty());
    }

    #[test]
    fn version_tags_only() {
        assert!(is_version_tag("1.7"));
        assert!(!is_version_tag("Mod"));
        assert!(!is_version_tag("1.7.4990"));
        assert!(!is_version_tag("v1.6"));
    }

    #[test]
    fn steam_record_next_to_workshop_content() {
        let dir = std::env::temp_dir().join(format!("rwmm_acf_{}", std::process::id()));
        let content = dir.join("steamapps").join("workshop").join("content").join("294100");
        std::fs::create_dir_all(&content).unwrap();
        assert!(acf_path(&content).is_none());
        std::fs::write(
            dir.join("steamapps").join("workshop").join("appworkshop_294100.acf"),
            ACF,
        )
        .unwrap();
        assert!(acf_path(&content).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Asks the real Steam (run by hand: `cargo test live_workshop_details -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_workshop_details() {
        let items = fetch_details(&["2009463077".into(), "730936602".into()]).unwrap();
        println!("{items:?}");
        assert_eq!(items.len(), 2);
    }
}
