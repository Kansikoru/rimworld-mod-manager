//! Snapshot of the last known working mod configuration (F9).
//!
//! Stores the active mod list and per-mod update timestamps to detect what changed
//! when RimWorld starts crashing after an update or mod change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::ModInfo;
use crate::steam::SteamPaths;

/// Snapshot of the active mod list and their modification times when the game was working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastGoodSnapshot {
    /// Unix timestamp when this snapshot was recorded.
    pub saved_at: u64,
    /// Active mod package IDs in load order.
    pub active_mods: Vec<String>,
    /// Package ID (lowercase) -> mod update timestamp in unix seconds.
    pub mod_timestamps: HashMap<String, u64>,
}

/// Changes detected between the last good run and the current active configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastGoodDiff {
    /// Active mods that were updated since the snapshot (lowercase IDs).
    pub updated: Vec<String>,
    /// Mods currently active that were not active in the snapshot (lowercase IDs).
    pub added: Vec<String>,
    /// Mods that were active in the snapshot but are no longer active (lowercase IDs).
    pub removed: Vec<String>,
    /// Whether the relative load order of the retained mods changed.
    pub order_changed: bool,
}

impl LastGoodDiff {
    pub fn is_empty(&self) -> bool {
        self.updated.is_empty() && self.added.is_empty() && self.removed.is_empty() && !self.order_changed
    }
}

impl LastGoodSnapshot {
    /// Standard path to `last_good.json` in the manager's data directory.
    pub fn path() -> PathBuf {
        crate::game_install::manager_data_dir().join("last_good.json")
    }

    /// Loads the snapshot from the default manager data path.
    pub fn load() -> Option<Self> {
        Self::load_from(&Self::path())
    }

    /// Loads the snapshot from a specific JSON file path.
    pub fn load_from(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Saves the snapshot to the default manager data path.
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    /// Saves the snapshot to a specific JSON file path.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Creates a new snapshot from the currently active mods and their timestamps.
    pub fn create(
        active_mods: &[String],
        all_mods: &[ModInfo],
        steam_paths: &SteamPaths,
        steamcmd_dir: Option<&Path>,
    ) -> Self {
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mod_timestamps = collect_mod_timestamps(
            active_mods,
            all_mods,
            steam_paths.workshop_path.as_deref(),
            steamcmd_dir,
        );

        Self {
            saved_at,
            active_mods: active_mods.to_vec(),
            mod_timestamps,
        }
    }

    /// Computes the diff between this snapshot and the current configuration.
    pub fn diff(&self, current_active: &[String], current_timestamps: &HashMap<String, u64>) -> LastGoodDiff {
        let current_active_lower: Vec<String> = current_active.iter().map(|id| id.to_lowercase()).collect();
        let saved_active_lower: Vec<String> = self.active_mods.iter().map(|id| id.to_lowercase()).collect();

        let current_set: std::collections::HashSet<&String> = current_active_lower.iter().collect();
        let saved_set: std::collections::HashSet<&String> = saved_active_lower.iter().collect();

        let added: Vec<String> = current_active_lower
            .iter()
            .filter(|id| !saved_set.contains(*id))
            .cloned()
            .collect();

        let removed: Vec<String> = saved_active_lower
            .iter()
            .filter(|id| !current_set.contains(*id))
            .cloned()
            .collect();

        let mut updated = Vec::new();
        for id in &current_active_lower {
            if saved_set.contains(id) {
                let saved_ts = self.mod_timestamps.get(id).copied().unwrap_or(0);
                let curr_ts = current_timestamps.get(id).copied().unwrap_or(0);
                if curr_ts > saved_ts && saved_ts > 0 {
                    updated.push(id.clone());
                }
            }
        }

        let common_current: Vec<&String> = current_active_lower
            .iter()
            .filter(|id| saved_set.contains(*id))
            .collect();
        let common_saved: Vec<&String> = saved_active_lower
            .iter()
            .filter(|id| current_set.contains(*id))
            .collect();
        let order_changed = common_current != common_saved;

        LastGoodDiff {
            updated,
            added,
            removed,
            order_changed,
        }
    }
}

/// Collects update timestamps for active mods using Steam ACF record or file mtime.
pub fn collect_mod_timestamps(
    active_mods: &[String],
    all_mods: &[ModInfo],
    workshop_dir: Option<&Path>,
    steamcmd_dir: Option<&Path>,
) -> HashMap<String, u64> {
    let mut acf_map = HashMap::new();

    if let Some(ws) = workshop_dir {
        if let Some(acf) = crate::mod_updates::acf_path(ws) {
            if let Ok(text) = std::fs::read_to_string(&acf) {
                acf_map.extend(crate::mod_updates::parse_acf_installed(&text));
            }
        }
    }

    if let Some(scmd) = steamcmd_dir {
        let scmd_acf = scmd.join("steamapps").join("workshop").join("appworkshop_294100.acf");
        if scmd_acf.is_file() {
            if let Ok(text) = std::fs::read_to_string(&scmd_acf) {
                acf_map.extend(crate::mod_updates::parse_acf_installed(&text));
            }
        }
    }

    let mut out = HashMap::new();
    for id in active_mods {
        let key = id.to_lowercase();
        if let Some(m) = all_mods.iter().find(|m| m.package_id.eq_ignore_ascii_case(id)) {
            let mut time = None;
            if let Some(ref ws_id) = m.steam_item_id {
                if let Some(&t) = acf_map.get(ws_id) {
                    time = Some(t);
                }
            }
            if time.is_none() {
                time = crate::mod_updates::newest_file_time(&m.dir_path).or_else(|| {
                    std::fs::metadata(&m.dir_path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|st| st.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                });
            }
            if let Some(t) = time {
                out.insert(key, t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrip_test() {
        let dir = std::env::temp_dir().join(format!("rwmm_last_good_{}", std::process::id()));
        let path = dir.join("last_good.json");

        let mut timestamps = HashMap::new();
        timestamps.insert("ludeon.rimworld".into(), 1700000000);
        timestamps.insert("brrainz.harmony".into(), 1700001000);

        let snap = LastGoodSnapshot {
            saved_at: 1700005000,
            active_mods: vec!["ludeon.rimworld".into(), "brrainz.harmony".into()],
            mod_timestamps: timestamps,
        };

        snap.save_to(&path).unwrap();
        let loaded = LastGoodSnapshot::load_from(&path).unwrap();
        assert_eq!(snap, loaded);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_detects_updated_added_removed_order() {
        let mut timestamps = HashMap::new();
        timestamps.insert("mod.a".into(), 100);
        timestamps.insert("mod.b".into(), 100);
        timestamps.insert("mod.c".into(), 100);

        let snap = LastGoodSnapshot {
            saved_at: 1000,
            active_mods: vec!["mod.a".into(), "mod.b".into(), "mod.c".into()],
            mod_timestamps: timestamps,
        };

        // Identical state -> empty diff
        let mut current_ts = HashMap::new();
        current_ts.insert("mod.a".into(), 100);
        current_ts.insert("mod.b".into(), 100);
        current_ts.insert("mod.c".into(), 100);
        let current_active = vec!["mod.a".into(), "mod.b".into(), "mod.c".into()];
        assert!(snap.diff(&current_active, &current_ts).is_empty());

        // mod.b updated (100 -> 150)
        current_ts.insert("mod.b".into(), 150);
        let diff_updated = snap.diff(&current_active, &current_ts);
        assert_eq!(diff_updated.updated, vec!["mod.b"]);
        assert!(diff_updated.added.is_empty());
        assert!(diff_updated.removed.is_empty());
        assert!(!diff_updated.order_changed);

        // mod.d added, mod.c removed
        let current_mod_changed = vec!["mod.a".into(), "mod.b".into(), "mod.d".into()];
        let diff_mod_change = snap.diff(&current_mod_changed, &current_ts);
        assert_eq!(diff_mod_change.added, vec!["mod.d"]);
        assert_eq!(diff_mod_change.removed, vec!["mod.c"]);
        assert!(!diff_mod_change.order_changed);

        // Order changed: a, b -> b, a
        let current_reordered = vec!["mod.b".into(), "mod.a".into(), "mod.c".into()];
        let diff_order = snap.diff(&current_reordered, &current_ts);
        assert!(diff_order.order_changed);
    }
}
