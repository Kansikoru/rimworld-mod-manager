//! The mods' load while the game runs, measured by the companion mod when asked: how much
//! of a game tick each mod's own ticking code takes.
//!
//! The manager leaves a request file; the companion sees it on the next launch, removes it,
//! times every `Tick` / `CompTick` / `...ComponentTick` method of the mods for the asked
//! number of ticks and writes `tick_profile.json`. What a mod does inside its Harmony
//! patches of the game's own methods is not in the numbers.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Ticks one measurement takes: two minutes of play at normal speed.
pub const TICKS: u64 = 7200;
/// A tick's budget at normal speed (60 ticks a second), in milliseconds.
pub const TICK_BUDGET_MS: f64 = 1000.0 / 60.0;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct TickProfile {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub written_unix: u64,
    /// Ticks measured.
    #[serde(default)]
    pub ticks: u64,
    /// A whole tick on average, milliseconds.
    #[serde(default)]
    pub tick_ms: f64,
    #[serde(default)]
    pub patched_methods: u64,
    /// Methods left out because there were too many to time.
    #[serde(default)]
    pub skipped_methods: u64,
    /// Heaviest first.
    #[serde(default)]
    pub mods: Vec<ModLoad>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ModLoad {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Milliseconds per tick.
    #[serde(default)]
    pub ms: f64,
    /// Calls per tick.
    #[serde(default)]
    pub calls: f64,
    /// The mod's heaviest methods.
    #[serde(default)]
    pub methods: Vec<MethodLoad>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MethodLoad {
    pub name: String,
    #[serde(default)]
    pub ms: f64,
    #[serde(default)]
    pub calls: f64,
}

impl TickProfile {
    /// All measured mods together, milliseconds per tick.
    pub fn mods_ms(&self) -> f64 {
        self.mods.iter().map(|m| m.ms).sum()
    }

    /// A mod's share of the whole tick, percent.
    pub fn share(&self, m: &ModLoad) -> f64 {
        if self.tick_ms > 0.0 {
            m.ms / self.tick_ms * 100.0
        } else {
            0.0
        }
    }
}

pub fn path() -> PathBuf {
    crate::game_install::manager_data_dir().join("tick_profile.json")
}

pub fn request_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("tick_profile.request")
}

pub fn load() -> Option<TickProfile> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Option<TickProfile> {
    let profile: TickProfile = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (profile.format == 1).then_some(profile)
}

/// Asks the companion to measure on the next launch of the game.
pub fn request_at(path: &Path, ticks: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, ticks.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_companions_file_is_read() {
        let dir = std::env::temp_dir().join(format!("rwmm_ticks_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tick_profile.json");
        std::fs::write(
            &file,
            r#"{"format":1,"game":"1.7.5005 rev1082","written_unix":1794000000,"ticks":7200,"tick_ms":8,
            "patched_methods":412,"skipped_methods":0,
            "mods":[{"id":"a.heavy","name":"Heavy","ms":2,"calls":310.5,
                     "methods":[{"name":"Heavy.Comp.CompTick","ms":1.5,"calls":300}]},
                    {"id":"b.light","name":"Light","ms":0.4,"calls":2}],
            "errors":[]}"#,
        )
        .unwrap();
        let profile = load_from(&file).unwrap();
        assert_eq!((profile.ticks, profile.mods.len()), (7200, 2));
        assert_eq!(profile.mods[0].methods[0].name, "Heavy.Comp.CompTick");
        assert!((profile.mods_ms() - 2.4).abs() < 1e-9);
        assert!((profile.share(&profile.mods[0]) - 25.0).abs() < 1e-9);

        // Another format is not guessed at; a missing file is no measurement.
        std::fs::write(&file, r#"{"format":2,"mods":[]}"#).unwrap();
        assert_eq!(load_from(&file), None);
        assert_eq!(load_from(&dir.join("none.json")), None);

        let request = dir.join("sub").join("tick_profile.request");
        request_at(&request, TICKS).unwrap();
        assert_eq!(std::fs::read_to_string(&request).unwrap(), "7200");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
