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

/// Which tabs the side bar shows. Any tab can be hidden but Settings (or hidden tabs
/// couldn't come back). `Default` shows them all (a prefs file from before a tab could be
/// hidden); a first start shows only Mods and Workshop (`first_start`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TabPrefs {
    #[serde(default)]
    pub hide_mods: bool,
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
    pub hide_analyzer: bool,
    /// Names next to the icons in the side bar (off: icons only, names on hover).
    #[serde(default)]
    pub sidebar_labels: bool,
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
    /// Theme and accent palette (F9 of the plan).
    #[serde(default)]
    pub theme: crate::ui::theme::ThemeKind,
    #[serde(default)]
    pub accent: crate::ui::theme::Accent,
}

impl TabPrefs {
    /// The very first start: Mods and Workshop; the rest is turned on in Settings → Tabs.
    pub fn first_start() -> Self {
        Self {
            hide_sets: true,
            hide_saves: true,
            hide_translations: true,
            hide_optimizer: true,
            hide_analyzer: true,
            ..Self::default()
        }
    }
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
            tabs: TabPrefs::first_start(),
            details_side: true,
            theme: Default::default(),
            accent: Default::default(),
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
        assert!(!prefs.tabs.hide_sets && !prefs.tabs.hide_analyzer);
        // Saved before themes: the first theme with its own accent.
        assert_eq!(prefs.theme, crate::ui::theme::ThemeKind::Lavender);
        assert_eq!(prefs.accent, crate::ui::theme::Accent::Theme);
    }

    #[test]
    fn tab_choices_survive_saving() {
        let mut prefs = UiPrefs::default();
        prefs.tabs.hide_translations = false;
        prefs.tabs.hide_mods = true;
        prefs.tabs.sidebar_labels = true;
        let back: UiPrefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(back, prefs);
    }

    #[test]
    fn first_start_shows_only_mods_and_workshop() {
        let tabs = UiPrefs::default().tabs;
        assert!(!tabs.hide_mods && !tabs.hide_workshop);
        assert!(tabs.hide_sets && tabs.hide_saves && tabs.hide_translations);
        assert!(tabs.hide_optimizer && tabs.hide_analyzer);
    }

    #[test]
    fn theme_choice_survives_saving() {
        let prefs = UiPrefs {
            theme: crate::ui::theme::ThemeKind::Mint,
            accent: crate::ui::theme::Accent::Amber,
            ..Default::default()
        };
        let back: UiPrefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!((back.theme, back.accent), (prefs.theme, prefs.accent));
        // The theme was called "Miku" at first: such a file keeps the theme and the font.
        let old: UiPrefs = serde_json::from_str(r#"{ "font": "Montserrat", "theme": "Miku" }"#).unwrap();
        assert_eq!(
            (old.theme, old.font),
            (crate::ui::theme::ThemeKind::Mint, UiFont::Montserrat)
        );
    }
}
