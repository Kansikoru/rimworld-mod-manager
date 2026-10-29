//! Game loading time detection and launch history (F11).
//!
//! Measures total startup time (from engine start to main menu) and records
//! a history of launches to show players how mod changes affected loading speed.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

/// One entry in the launch loading time history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadTimeEntry {
    /// Unix timestamp when this launch was recorded.
    pub recorded_at: u64,
    /// Total loading time in seconds.
    pub duration_secs: f32,
    /// Number of active mods during this launch.
    pub mod_count: usize,
    /// Active mod package IDs in load order.
    pub active_mods: Vec<String>,
    /// Summary of changes and duration delta vs the previous launch.
    #[serde(default)]
    pub diff_summary: Option<String>,
}

/// History of recent game launches and their loading times.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadTimeHistory {
    pub entries: Vec<LoadTimeEntry>,
}

const MAX_HISTORY_ENTRIES: usize = 30;

impl LoadTimeHistory {
    /// Standard path to `load_times.json` in the manager data directory.
    pub fn path() -> PathBuf {
        crate::game_install::manager_data_dir().join("load_times.json")
    }

    /// Loads history from the default path.
    pub fn load() -> Self {
        Self::load_from(&Self::path()).unwrap_or_default()
    }

    /// Loads history from a specific file.
    pub fn load_from(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Saves history to the default path.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// Records a new launch and computes the difference vs the previous run.
    pub fn record_launch(
        &mut self,
        duration_secs: f32,
        active_mods: &[String],
        mod_name_fn: impl Fn(&str) -> String,
        is_ru: bool,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let diff_summary = if let Some(prev) = self.entries.last() {
            let added: Vec<&String> = active_mods.iter().filter(|id| !prev.active_mods.contains(id)).collect();
            let removed: Vec<&String> = prev.active_mods.iter().filter(|id| !active_mods.contains(id)).collect();
            let delta = duration_secs - prev.duration_secs;

            if !added.is_empty() || !removed.is_empty() || delta.abs() >= 1.0 {
                let delta_str = if delta >= 0.0 {
                    format!("+{}", format_duration(delta, is_ru))
                } else {
                    format!("-{}", format_duration(-delta, is_ru))
                };

                let mut change_desc = Vec::new();
                if added.len() == 1 {
                    let name = mod_name_fn(added[0]);
                    change_desc.push(if is_ru {
                        format!("после добавления `{name}`")
                    } else {
                        format!("after adding `{name}`")
                    });
                } else if !added.is_empty() {
                    let n = added.len();
                    change_desc.push(if is_ru {
                        format!("после добавления {n} модов")
                    } else {
                        format!("after adding {n} mods")
                    });
                }

                if removed.len() == 1 {
                    let name = mod_name_fn(removed[0]);
                    change_desc.push(if is_ru {
                        format!("после удаления `{name}`")
                    } else {
                        format!("after removing `{name}`")
                    });
                } else if !removed.is_empty() {
                    let n = removed.len();
                    change_desc.push(if is_ru {
                        format!("после удаления {n} модов")
                    } else {
                        format!("after removing {n} mods")
                    });
                }

                if change_desc.is_empty() {
                    Some(delta_str)
                } else {
                    Some(format!("{} ({})", change_desc.join(", "), delta_str))
                }
            } else {
                None
            }
        } else {
            None
        };

        self.entries.push(LoadTimeEntry {
            recorded_at: now,
            duration_secs,
            mod_count: active_mods.len(),
            active_mods: active_mods.to_vec(),
            diff_summary,
        });

        if self.entries.len() > MAX_HISTORY_ENTRIES {
            let remove_count = self.entries.len() - MAX_HISTORY_ENTRIES;
            self.entries.drain(0..remove_count);
        }

        let _ = self.save();
    }
}

/// Formats a seconds duration as "1 мин 25 с" (RU) or "1m 25s" (EN).
pub fn format_duration(secs: f32, is_ru: bool) -> String {
    let rounded = secs.round() as u64;
    let minutes = rounded / 60;
    let rem_seconds = rounded % 60;
    if minutes > 0 {
        if is_ru {
            format!("{minutes} мин {rem_seconds} с")
        } else {
            format!("{minutes}m {rem_seconds}s")
        }
    } else if is_ru {
        format!("{secs:.1} с")
    } else {
        format!("{secs:.1}s")
    }
}

/// Parses total loading time from Player.log contents, if present.
pub fn parse_load_time_from_log(content: &str) -> Option<f32> {
    // 1. Waylith (1.7+) timestamp analysis:
    // Started: 2026-10-27 18:44:30.150 +01:00
    // Entries: 2026-10-27 18:44:53.368 INFO ...
    if let Some(t) = parse_waylith_load_time(content) {
        return Some(t);
    }

    // 2. Mod-logged startup times (FasterGameLoading, StartupImpact, etc.)
    let re_mod_time = Regex::new(
        r"(?i)(?:Total loading time|Startup took|Game loaded in|Loaded in)\s*[:=]?\s*([0-9]+(?:\.[0-9]+)?)\s*s",
    )
    .ok();
    if let Some(ref re) = re_mod_time {
        for line in content.lines().rev() {
            if let Some(caps) = re.captures(line) {
                if let Some(val) = caps.get(1).and_then(|m| m.as_str().parse::<f32>().ok()) {
                    if val > 0.0 && val < 3600.0 {
                        return Some(val);
                    }
                }
            }
        }
    }

    // 3. Prepatcher vanilla load time
    let re_prepatcher = Regex::new(r"vanilla load took\s*([0-9]+(?:\.[0-9]+)?)\s*s").ok();
    if let Some(ref re) = re_prepatcher {
        for line in content.lines() {
            if let Some(caps) = re.captures(line) {
                if let Some(val) = caps.get(1).and_then(|m| m.as_str().parse::<f32>().ok()) {
                    if val > 0.0 && val < 3600.0 {
                        return Some(val);
                    }
                }
            }
        }
    }

    None
}

/// Extracts seconds from a "HH:MM:SS.fff" time string.
fn parse_time_of_day_secs(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let s: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

fn parse_waylith_load_time(content: &str) -> Option<f32> {
    let re_started = Regex::new(r"Started:\s+\d{4}-\d{2}-\d{2}\s+(\d{2}:\d{2}:\d{2}\.\d{3})").ok()?;
    let re_entry_time = Regex::new(r"^(\d{4}-\d{2}-\d{2}\s+)?(\d{2}:\d{2}:\d{2}\.\d{3})").ok()?;

    let mut start_secs: Option<f64> = None;
    for line in content.lines().take(20) {
        if let Some(caps) = re_started.captures(line) {
            if let Some(m) = caps.get(1) {
                start_secs = parse_time_of_day_secs(m.as_str());
                break;
            }
        }
    }

    let start = start_secs?;

    // Search backwards for the last log line that has a timestamp
    for line in content.lines().rev() {
        if let Some(caps) = re_entry_time.captures(line) {
            if let Some(m) = caps.get(2) {
                if let Some(end_secs) = parse_time_of_day_secs(m.as_str()) {
                    let mut diff = end_secs - start;
                    if diff < 0.0 {
                        diff += 86400.0; // crossed midnight
                    }
                    if diff > 0.1 && diff < 3600.0 {
                        return Some(diff as f32);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_tests() {
        assert_eq!(format_duration(25.4, true), "25.4 с");
        assert_eq!(format_duration(25.4, false), "25.4s");
        assert_eq!(format_duration(85.0, true), "1 мин 25 с");
        assert_eq!(format_duration(85.0, false), "1m 25s");
    }

    #[test]
    fn parse_waylith_load_time_test() {
        let log = "\
=== Waylith log ===
Started:   2026-10-27 18:44:30.150 +01:00
Version:   1.7.4990.1069
OS:        Microsoft Windows 10

2026-10-27 18:44:31.048  INFO Vulkan initialized
2026-10-27 18:44:53.368  INFO Loading complete
";
        let time = parse_load_time_from_log(log);
        assert!(time.is_some());
        let secs = time.unwrap();
        assert!((secs - 23.218).abs() < 0.01);
    }

    #[test]
    fn parse_mod_reported_time_test() {
        let log = "\
Initialize engine version: 2022.3.35f1
[FasterGameLoading] Total loading time: 42.5s
";
        let time = parse_load_time_from_log(log);
        assert_eq!(time, Some(42.5));

        let prepatcher_log = "\
Initialize engine version: 2022.3.35f1
Prepatcher: Starting... (vanilla load took 12.3s)
";
        let time2 = parse_load_time_from_log(prepatcher_log);
        assert_eq!(time2, Some(12.3));
    }

    #[test]
    fn history_recording_and_diff_test() {
        let dir = std::env::temp_dir().join(format!("rwmm_loadtime_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("load_times.json");

        let mut history = LoadTimeHistory::default();
        let mods1 = vec!["core".to_string(), "harmony".to_string()];
        history.record_launch(30.0, &mods1, |id| id.to_uppercase(), true);
        assert_eq!(history.entries.len(), 1);
        assert!(history.entries[0].diff_summary.is_none());

        let mods2 = vec!["core".to_string(), "harmony".to_string(), "alpha.animals".to_string()];
        history.record_launch(70.0, &mods2, |id| id.to_uppercase(), true);
        assert_eq!(history.entries.len(), 2);
        assert!(history.entries[1].diff_summary.is_some());
        let diff = history.entries[1].diff_summary.as_ref().unwrap();
        assert!(diff.contains("ALPHA.ANIMALS"));
        assert!(diff.contains("+40.0 с") || diff.contains("+40 с"));

        let json = serde_json::to_string(&history).unwrap();
        std::fs::write(&path, json).unwrap();
        let loaded: LoadTimeHistory = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.entries.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
