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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    #[serde(default)]
    pub font: UiFont,
    /// Multiplier for every text size (1.0 = sizes as designed).
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,
}

fn default_text_scale() -> f32 {
    1.1
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            font: UiFont::default(),
            text_scale: default_text_scale(),
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
