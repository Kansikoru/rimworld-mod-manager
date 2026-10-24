//! App settings that survive restarts (S13): paths chosen by hand, language, launch options,
//! optimizer threads. Saved to `%LOCALAPPDATA%\RimWorldModManager\settings.json`.
//! Paths are only stored when the user picked them; otherwise auto-detection is used.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::i18n::Language;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AppSettings {
    /// Game folder picked by hand (None = auto-detect via Steam).
    #[serde(default)]
    pub game_path: Option<PathBuf>,
    /// Workshop content folder picked by hand.
    #[serde(default)]
    pub workshop_path: Option<PathBuf>,
    /// ModsConfig.xml picked by hand.
    #[serde(default)]
    pub mods_config_path: Option<PathBuf>,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub launch_flags: Vec<String>,
    #[serde(default)]
    pub launch_extra_args: String,
    #[serde(default)]
    pub opt_threads: Option<usize>,
}

impl AppSettings {
    pub fn path() -> PathBuf {
        crate::game_install::manager_data_dir().join("settings.json")
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("rwmm_settings_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let s = AppSettings {
            game_path: Some(PathBuf::from(r"F:\Games\RimWorld")),
            language: Language::English,
            launch_flags: vec!["-ColdBoot".into()],
            launch_extra_args: "-mod=a;b".into(),
            opt_threads: Some(6),
            ..Default::default()
        };
        s.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path), Some(s));
        assert!(AppSettings::load_from(&dir.join("missing.json")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
