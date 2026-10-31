//! Game loading time and launch history (F11).
//!
//! Neither 1.6 nor 1.7 writes a "loading finished" line, and the times some mods log
//! (Prepatcher's "vanilla load took") cover only a part of the loading. So the manager
//! measures it itself for games started from it: loading is over when the game window
//! answers and the game goes quiet (CPU, disk, memory and log), as at the main menu.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::{fmt, Language, TranslationKey};

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
    /// Names of the mods added since the previous launch.
    #[serde(default)]
    pub added: Vec<String>,
    /// Names of the mods removed since the previous launch.
    #[serde(default)]
    pub removed: Vec<String>,
    /// Loading time change vs the previous launch, in seconds.
    #[serde(default)]
    pub delta_secs: Option<f32>,
    /// How it was measured (`PROBE_VERSION`); 0: read from the log, only a part of the loading.
    #[serde(default)]
    pub probe_version: u32,
}

impl LoadTimeEntry {
    /// "after adding `X` (+40 s)" in the current language; `None` when nothing changed.
    pub fn change_text(&self) -> Option<String> {
        self.change_text_in(crate::i18n::current_language())
    }

    pub fn change_text_in(&self, lang: Language) -> Option<String> {
        let delta = self.delta_secs.unwrap_or(0.0);
        if self.added.is_empty() && self.removed.is_empty() && delta.abs() < 1.0 {
            return None;
        }
        let delta_str = if delta >= 0.0 {
            format!("+{}", format_duration_in(delta, lang))
        } else {
            format!("-{}", format_duration_in(-delta, lang))
        };
        let mut parts = Vec::new();
        let mut describe = |names: &[String], one: TranslationKey, many: TranslationKey| match names {
            [] => {}
            [name] => parts.push(fmt(one.translate(lang), &[name.as_str()])),
            _ => parts.push(fmt(many.translate(lang), &[&names.len().to_string()])),
        };
        describe(
            &self.added,
            TranslationKey::LoadTimeAfterAdding,
            TranslationKey::LoadTimeAfterAddingMany,
        );
        describe(
            &self.removed,
            TranslationKey::LoadTimeAfterRemoving,
            TranslationKey::LoadTimeAfterRemovingMany,
        );
        Some(if parts.is_empty() {
            delta_str
        } else {
            format!("{} ({})", parts.join(", "), delta_str)
        })
    }

    /// Loading got slower than on the previous launch.
    pub fn is_slower(&self) -> bool {
        self.delta_secs.unwrap_or(0.0) > 0.0
    }
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

    /// Loads history from a specific file. Entries measured an older way are dropped: a
    /// part of the loading read from the log, or a frozen load taken for the menu.
    pub fn load_from(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut history: Self = serde_json::from_str(&text).ok()?;
        history.entries.retain(|e| e.probe_version >= PROBE_VERSION);
        Some(history)
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

    /// Records a new launch and what changed since the previous one. Saved by the caller
    /// (`save`): tests record into a history of their own without touching the real file.
    pub fn record_launch(&mut self, duration_secs: f32, active_mods: &[String], mod_name_fn: impl Fn(&str) -> String) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let (added, removed, delta_secs) = match self.entries.last() {
            Some(prev) => (
                active_mods
                    .iter()
                    .filter(|id| !prev.active_mods.contains(id))
                    .map(|id| mod_name_fn(id))
                    .collect(),
                prev.active_mods
                    .iter()
                    .filter(|id| !active_mods.contains(id))
                    .map(|id| mod_name_fn(id))
                    .collect(),
                Some(duration_secs - prev.duration_secs),
            ),
            None => (Vec::new(), Vec::new(), None),
        };

        self.entries.push(LoadTimeEntry {
            recorded_at: now,
            duration_secs,
            mod_count: active_mods.len(),
            active_mods: active_mods.to_vec(),
            added,
            removed,
            delta_secs,
            probe_version: PROBE_VERSION,
        });

        if self.entries.len() > MAX_HISTORY_ENTRIES {
            let remove_count = self.entries.len() - MAX_HISTORY_ENTRIES;
            self.entries.drain(0..remove_count);
        }
    }
}

/// A duration as "1 мин 25 с" / "1m 25s" / "1分25秒" in the current language.
pub fn format_duration(secs: f32) -> String {
    format_duration_in(secs, crate::i18n::current_language())
}

pub fn format_duration_in(secs: f32, lang: Language) -> String {
    let rounded = secs.round() as u64;
    if rounded >= 60 {
        fmt(
            TranslationKey::DurationMinSec.translate(lang),
            &[&(rounded / 60).to_string(), &(rounded % 60).to_string()],
        )
    } else {
        fmt(TranslationKey::DurationSec.translate(lang), &[&format!("{secs:.1}")])
    }
}

/// Version of the way loading is measured; history entries from older ways are dropped.
/// 1: CPU + log only (took a frozen, disk-bound load for an idle menu).
/// 2: also the window answering, disk reads and memory.
pub const PROBE_VERSION: u32 = 2;

/// Seconds of quiet that end the loading.
const QUIET_WINDOW_SECS: f32 = 10.0;
/// Loading takes at least this long: the process starting up is no "quiet".
const MIN_LOAD_SECS: f32 = 15.0;
/// Average CPU use (in cores) below which the game is idle. The main menu needs a fraction
/// of one core.
const IDLE_CORES: f64 = 0.5;
/// Disk reads allowed in a quiet window (menu music streams a little). Loading reads mods.
const QUIET_READ_BYTES: u64 = 8 * 1024 * 1024;
/// Memory growth allowed in a quiet window. Loading keeps filling memory with textures and
/// defs, even while the window hangs and the CPU waits for the disk or the graphics card.
const QUIET_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
/// Log growth allowed in a quiet window: some mods write a line now and then at the menu.
const QUIET_LOG_BYTES: u64 = 4096;
/// Stop measuring after this: something else is going on (a dialog, a stuck load).
const MAX_LOAD_SECS: f32 = 1800.0;

/// One look at the game, taken once a second.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProbeSample {
    /// CPU seconds the game has used so far.
    pub cpu_secs: f64,
    /// Bytes the game has read from disk so far.
    pub read_bytes: u64,
    /// The game's private memory.
    pub private_bytes: u64,
    /// Player.log size.
    pub log_len: u64,
    /// The game window answers (`None`: no window yet, `Some(false)`: "Not responding").
    pub responsive: Option<bool>,
}

/// `load_probe.csv` in the manager data folder: the samples of the last launch.
pub fn probe_log_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("load_probe.csv")
}

impl ProbeSample {
    pub const CSV_HEADER: &'static str = "secs,cpu_secs,read_mb,private_mb,log_kb,responsive,load_time";

    /// One line of `load_probe.csv`.
    pub fn csv_row(&self, secs: f32, load_time: Option<f32>) -> String {
        let mb = |b: u64| b as f64 / 1_048_576.0;
        format!(
            "{secs:.1},{:.2},{:.1},{:.1},{:.1},{},{}",
            self.cpu_secs,
            mb(self.read_bytes),
            mb(self.private_bytes),
            self.log_len as f64 / 1024.0,
            match self.responsive {
                Some(true) => "yes",
                Some(false) => "hung",
                None => "no_window",
            },
            load_time.map(|t| format!("{t:.1}")).unwrap_or_default()
        )
    }
}

/// Measures the loading time: it is over when, for a whole quiet window, the game window
/// answers and the game neither works the CPU, nor reads the disk, nor fills memory, nor
/// writes the log. A load that freezes the window, or that waits for the disk with an idle
/// CPU, does not look quiet.
#[derive(Debug, Default)]
pub struct LoadProbe {
    /// (seconds since the launch, sample).
    samples: Vec<(f32, ProbeSample)>,
    result: Option<f32>,
    gave_up: bool,
}

impl LoadProbe {
    /// Adds a sample; returns the loading time once it is known.
    pub fn push(&mut self, secs: f32, sample: ProbeSample) -> Option<f32> {
        if self.result.is_some() || self.gave_up {
            return self.result;
        }
        if secs > MAX_LOAD_SECS {
            self.gave_up = true;
            return None;
        }
        self.samples.push((secs, sample));
        let start = self.samples.iter().rposition(|(t, _)| *t <= secs - QUIET_WINDOW_SECS)?;
        let (start_t, first) = self.samples[start];
        if start_t < MIN_LOAD_SECS {
            return None;
        }
        let window_answers = self.samples[start..].iter().all(|(_, s)| s.responsive == Some(true));
        let cores = (sample.cpu_secs - first.cpu_secs) / f64::from(secs - start_t);
        let quiet = window_answers
            && cores < IDLE_CORES
            && sample.read_bytes.saturating_sub(first.read_bytes) <= QUIET_READ_BYTES
            && sample.private_bytes.saturating_sub(first.private_bytes) <= QUIET_MEMORY_BYTES
            && sample.log_len.saturating_sub(first.log_len) <= QUIET_LOG_BYTES;
        if quiet {
            self.result = Some(start_t);
        }
        self.result
    }

    pub fn result(&self) -> Option<f32> {
        self.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_tests() {
        assert_eq!(format_duration_in(25.4, Language::Russian), "25.4 с");
        assert_eq!(format_duration_in(25.4, Language::English), "25.4s");
        assert_eq!(format_duration_in(85.0, Language::Russian), "1 мин 25 с");
        assert_eq!(format_duration_in(85.0, Language::English), "1m 25s");
        assert_eq!(format_duration_in(85.0, Language::Chinese), "1分25秒");
    }

    /// How the game behaves during one second.
    #[derive(Clone, Copy)]
    struct Second {
        cores: f64,
        read: u64,
        memory: u64,
        log: u64,
        responsive: Option<bool>,
    }

    const LOADING: Second = Second {
        cores: 1.2,
        read: 20_000_000,
        memory: 15_000_000,
        log: 20_000,
        responsive: Some(true),
    };
    const MENU: Second = Second {
        cores: 0.15,
        read: 50_000,
        memory: 100_000,
        log: 0,
        responsive: Some(true),
    };

    /// Feeds a run made of (seconds, behaviour) phases; returns the measured loading time.
    fn run(phases: &[(u32, Second)]) -> Option<f32> {
        let mut probe = LoadProbe::default();
        let mut s = ProbeSample::default();
        let mut t = 0u32;
        for &(len, b) in phases {
            for _ in 0..len {
                s.cpu_secs += b.cores;
                s.read_bytes += b.read;
                s.private_bytes += b.memory;
                s.log_len += b.log;
                s.responsive = b.responsive;
                if let Some(r) = probe.push(t as f32, s) {
                    return Some(r);
                }
                t += 1;
            }
        }
        probe.result()
    }

    #[test]
    fn probe_finds_the_end_of_loading() {
        // The last loading second is sample 94.
        let t = run(&[(95, LOADING), (60, MENU)]).unwrap();
        assert!((94.0..=96.0).contains(&t), "loading ended at {t}");
        // A mod writing a little at the menu does not keep the "loading" going.
        let chatty_menu = Second { log: 200, ..MENU };
        let t = run(&[(95, LOADING), (60, chatty_menu)]).unwrap();
        assert!((94.0..=96.0).contains(&t), "loading ended at {t}");
    }

    /// The case from a real 189-mod run: the window froze ("Not responding") for minutes
    /// while the game read textures with an idle CPU and a quiet log. Measured 10 s before.
    #[test]
    fn a_frozen_disk_bound_load_is_not_the_menu() {
        let frozen_reading = Second {
            cores: 0.2,
            read: 30_000_000,
            memory: 20_000_000,
            log: 0,
            responsive: Some(false),
        };
        let t = run(&[(10, LOADING), (280, frozen_reading), (60, MENU)]).unwrap();
        assert!((289.0..=291.0).contains(&t), "loading ended at {t}");

        // Each sign on its own keeps the loading going: the window hangs, the disk is read,
        // memory fills.
        let hung = Second {
            responsive: Some(false),
            ..MENU
        };
        let reading = Second {
            read: 5_000_000,
            ..MENU
        };
        let filling = Second {
            memory: 20_000_000,
            ..MENU
        };
        let no_window = Second {
            responsive: None,
            ..MENU
        };
        for busy in [hung, reading, filling, no_window] {
            assert_eq!(run(&[(20, LOADING), (60, busy)]), None);
        }
    }

    #[test]
    fn probe_needs_the_game_to_go_quiet() {
        // Busy menu (more than half a core) or a log still pouring: no measurement yet.
        assert_eq!(run(&[(95, LOADING), (60, Second { cores: 0.9, ..MENU })]), None);
        assert_eq!(run(&[(95, LOADING), (60, Second { log: 50_000, ..MENU })]), None);
    }

    #[test]
    fn probe_ignores_quiet_start_and_gives_up_late() {
        // The first seconds (Steam starting the game) are quiet but are no loading.
        assert_eq!(run(&[(MIN_LOAD_SECS as u32, MENU)]), None);
        let mut probe = LoadProbe::default();
        let quiet = ProbeSample {
            responsive: Some(true),
            ..Default::default()
        };
        assert_eq!(probe.push(MAX_LOAD_SECS + 1.0, quiet), None);
        assert_eq!(probe.push(MAX_LOAD_SECS + 20.0, quiet), None);
    }

    #[test]
    fn history_records_changes_and_formats_them_per_language() {
        let mut history = LoadTimeHistory::default();
        let mods1 = vec!["core".to_string(), "harmony".to_string()];
        history.record_launch(30.0, &mods1, |id| id.to_uppercase());
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].change_text_in(Language::English), None);

        let mods2 = vec!["core".to_string(), "harmony".to_string(), "alpha.animals".to_string()];
        history.record_launch(70.0, &mods2, |id| id.to_uppercase());
        let e = &history.entries[1];
        assert!(e.is_slower());
        assert_eq!(
            e.change_text_in(Language::English).unwrap(),
            "after adding `ALPHA.ANIMALS` (+40.0s)"
        );
        assert!(e.change_text_in(Language::Russian).unwrap().contains("ALPHA.ANIMALS"));
        assert!(e.change_text_in(Language::Chinese).unwrap().contains("+40.0秒"));
    }

    #[test]
    fn entries_of_older_measuring_are_dropped() {
        let dir = std::env::temp_dir().join(format!("rwmm_loadtime_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("load_times.json");
        // From the log (Prepatcher), and from the first CPU-only measuring.
        std::fs::write(
            &path,
            r#"{"entries":[
                {"recorded_at":1,"duration_secs":9.8,"mod_count":1,"active_mods":["a"],"diff_summary":null},
                {"recorded_at":2,"duration_secs":10.3,"mod_count":1,"active_mods":["a"],"measured":true},
                {"recorded_at":3,"duration_secs":290.0,"mod_count":1,"active_mods":["a"],"probe_version":2}
            ]}"#,
        )
        .unwrap();
        let history = LoadTimeHistory::load_from(&path).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].duration_secs, 290.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
