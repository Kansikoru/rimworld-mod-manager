//! App settings that survive restarts (S13): paths chosen by hand, language, launch options,
//! optimizer threads. Saved to `%LOCALAPPDATA%\RimWorldModManager\settings.json`.
//! Paths are only stored when the user picked them; otherwise auto-detection is used.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::i18n::Language;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Look for a new version on GitHub once a day (on unless turned off).
    #[serde(default = "default_true")]
    pub check_updates: bool,
    /// Download Workshop mods with SteamCMD instead of subscribing in the Steam client.
    /// None = never set: on when SteamCMD is installed.
    #[serde(default)]
    pub use_steamcmd: Option<bool>,
    /// SteamCMD folder picked by hand (None = next to the settings).
    #[serde(default)]
    pub steamcmd_dir: Option<PathBuf>,
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            game_path: None,
            workshop_path: None,
            mods_config_path: None,
            language: Language::default(),
            launch_flags: Vec::new(),
            launch_extra_args: String::new(),
            opt_threads: None,
            check_updates: true,
            use_steamcmd: None,
            steamcmd_dir: None,
        }
    }
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
        // Settings saved by an older version have no "check_updates": it is on; SteamCMD
        // was never chosen either way.
        std::fs::write(&path, r#"{"language":"English"}"#).unwrap();
        let old = AppSettings::load_from(&path).unwrap();
        assert!(old.check_updates);
        assert_eq!(old.use_steamcmd, None);
        // 2.1.0 wrote a plain true/false.
        std::fs::write(&path, r#"{"language":"English","use_steamcmd":false}"#).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap().use_steamcmd, Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
