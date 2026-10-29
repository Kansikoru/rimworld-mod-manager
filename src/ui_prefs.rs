//! Interface preferences (font and text size), saved to
//! `%LOCALAPPDATA%\RimWorldModManager\ui_prefs.json`.
//!
//! egui draws text without hinting (no snapping of letter stems to the pixel grid, no ClearType),
//! so thin fonts at small sizes look soft. A heavier font and a slightly larger size help a lot;
//! which one is most comfortable depends on the monitor, so the user can choose.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UiFont {
    SegoeUi,
    #[default]
    SegoeUiSemibold,
    Verdana,
    Tahoma,
    Arial,
    Montserrat,
    MontserratSemibold,
}

impl UiFont {
    pub const ALL: [UiFont; 7] = [
        UiFont::SegoeUiSemibold,
        UiFont::SegoeUi,
        UiFont::Montserrat,
        UiFont::MontserratSemibold,
        UiFont::Verdana,
        UiFont::Tahoma,
        UiFont::Arial,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            UiFont::SegoeUi => "Segoe UI",
            UiFont::SegoeUiSemibold => "Segoe UI Semibold",
            UiFont::Verdana => "Verdana",
            UiFont::Tahoma => "Tahoma",
            UiFont::Arial => "Arial",
            UiFont::Montserrat => "Montserrat",
            UiFont::MontserratSemibold => "Montserrat SemiBold",
        }
    }

    /// Font files to try, in order (Windows Fonts folder).
    pub fn files(&self) -> &'static [&'static str] {
        match self {
            UiFont::SegoeUi | UiFont::Montserrat | UiFont::MontserratSemibold => &["segoeui.ttf"],
            UiFont::SegoeUiSemibold => &["seguisb.ttf", "segoeui.ttf"],
            UiFont::Verdana => &["verdana.ttf", "segoeui.ttf"],
            UiFont::Tahoma => &["tahoma.ttf", "segoeui.ttf"],
            UiFont::Arial => &["arial.ttf", "segoeui.ttf"],
        }
    }

    /// Fonts shipped inside the program (not part of Windows). For these, `files()`
    /// is the fallback for letters the bundled font doesn't have.
    pub fn bundled(&self) -> Option<&'static [u8]> {
        match self {
            UiFont::Montserrat => Some(include_bytes!("../assets/fonts/Montserrat-Medium.ttf")),
            UiFont::MontserratSemibold => Some(include_bytes!("../assets/fonts/Montserrat-SemiBold.ttf")),
            _ => None,
        }
    }
}

/// How a tab that can't be hidden is shown in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TabLabel {
    #[default]
    Full,
    /// A short word ("Log").
    Short,
    Icon,
}

/// Which tabs the top bar shows. Mods can't be hidden (the main tab); the Log Analyzer and
/// Settings can only get shorter (Settings must stay, or hidden tabs couldn't come back).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TabPrefs {
    #[serde(default)]
    pub hide_workshop: bool,
    #[serde(default)]
    pub hide_sets: bool,
    #[serde(default)]
    pub hide_saves: bool,
    #[serde(default)]
    pub hide_translations: bool,
    #[serde(default)]
    pub hide_optimizer: bool,
    #[serde(default)]
    pub analyzer_label: TabLabel,
    /// The Settings tab as a gear icon only.
    #[serde(default)]
    pub settings_icon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    #[serde(default)]
    pub font: UiFont,
    /// Multiplier for every text size (1.0 = sizes as designed).
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,
    #[serde(default)]
    pub tabs: TabPrefs,
    /// Mods tab: the mod's details in a full-height strip on the right (true) or as a card
    /// under the active list (false, after the strip's close button).
    #[serde(default = "default_true")]
    pub details_side: bool,
}

fn default_text_scale() -> f32 {
    1.1
}

fn default_true() -> bool {
    true
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            font: UiFont::default(),
            text_scale: default_text_scale(),
            tabs: TabPrefs::default(),
            details_side: true,
        }
    }
}

pub const TEXT_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.9..=1.5;

impl UiPrefs {
    pub fn path() -> PathBuf {
        crate::game_install::manager_data_dir().join("ui_prefs.json")
    }

    pub fn load() -> Self {
        let mut prefs: UiPrefs = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        prefs.text_scale = prefs
            .text_scale
            .clamp(*TEXT_SCALE_RANGE.start(), *TEXT_SCALE_RANGE.end());
        prefs
    }

    pub fn save(&self) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(Self::path(), text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_prefs_files_load_with_all_tabs_shown() {
        let old = r#"{ "font": "Montserrat", "text_scale": 1.15 }"#;
        let prefs: UiPrefs = serde_json::from_str(old).unwrap();
        assert_eq!(prefs.font, UiFont::Montserrat);
        assert_eq!(prefs.tabs, TabPrefs::default());
        assert!(!prefs.tabs.hide_sets && prefs.tabs.analyzer_label == TabLabel::Full);
    }

    #[test]
    fn tab_choices_survive_saving() {
        let mut prefs = UiPrefs::default();
        prefs.tabs.hide_translations = true;
        prefs.tabs.analyzer_label = TabLabel::Icon;
        prefs.tabs.settings_icon = true;
        let back: UiPrefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(back, prefs);
    }
}
