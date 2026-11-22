use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Interface colors of a theme (F9 of the plan: themes and accent palettes).
/// Field names are kept from the first theme so every screen picks up any palette:
/// `accent_cyan` is the main accent, `accent_purple` the filled primary button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeColors {
    /// Window background.
    pub bg_dark: Color32,
    /// Panels and bars.
    pub bg_surface: Color32,
    pub bg_surface_hover: Color32,
    /// Cards, list rows, secondary buttons.
    pub bg_card: Color32,
    pub bg_sidebar: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,

    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,

    /// Main accent: soft lavender.
    pub accent_cyan: Color32,
    pub accent_cyan_hover: Color32,
    /// Filled primary buttons: deep violet.
    pub accent_purple: Color32,
    pub accent_purple_hover: Color32,
    /// Success: soft mint.
    pub accent_emerald: Color32,
    /// Danger: soft rose.
    pub accent_rose: Color32,
    /// Warning: soft amber.
    pub accent_amber: Color32,
    /// Info: soft sky.
    pub accent_info: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The first theme: soft lavender on deep violet-black.
const LAVENDER: ThemeColors = ThemeColors {
    bg_dark: rgb(0x0E0B16),
    bg_surface: rgb(0x151021),
    bg_surface_hover: rgb(0x251C3D),
    bg_card: rgb(0x1B1530),
    bg_sidebar: rgb(0x0E0B16),
    border_subtle: rgb(0x2E2547),
    border_strong: rgb(0x45386B),
    text_primary: rgb(0xF3F0FA),
    text_secondary: rgb(0xCFC7E6),
    text_muted: rgb(0x8B82A8),
    accent_cyan: rgb(0xB69CFF),
    accent_cyan_hover: rgb(0xCDB8FF),
    accent_purple: rgb(0x7C5CE0),
    accent_purple_hover: rgb(0x8F72EC),
    accent_emerald: rgb(0x6EE7B7),
    accent_rose: rgb(0xF47FA0),
    accent_amber: rgb(0xFBBF77),
    accent_info: rgb(0x8EC5FF),
};

/// Neutral dark grey with a calm blue accent.
const DARK: ThemeColors = ThemeColors {
    bg_dark: rgb(0x121212),
    bg_surface: rgb(0x1A1A1A),
    bg_surface_hover: rgb(0x2A2A2A),
    bg_card: rgb(0x1F1F1F),
    bg_sidebar: rgb(0x121212),
    border_subtle: rgb(0x2C2C2C),
    border_strong: rgb(0x454545),
    text_primary: rgb(0xF2F2F2),
    text_secondary: rgb(0xCFCFCF),
    text_muted: rgb(0x8C8C8C),
    accent_cyan: rgb(0x8AB4F8),
    accent_cyan_hover: rgb(0xA8C7FA),
    accent_purple: rgb(0x3B6FD8),
    accent_purple_hover: rgb(0x4C7FE6),
    accent_emerald: rgb(0x6EE7B7),
    accent_rose: rgb(0xF47F8E),
    accent_amber: rgb(0xF6C177),
    accent_info: rgb(0x8EC5FF),
};

/// Mint teal and pink on teal-black.
const MINT: ThemeColors = ThemeColors {
    bg_dark: rgb(0x0C1416),
    bg_surface: rgb(0x121E21),
    bg_surface_hover: rgb(0x1E3236),
    bg_card: rgb(0x16262A),
    bg_sidebar: rgb(0x0C1416),
    border_subtle: rgb(0x23393D),
    border_strong: rgb(0x33565C),
    text_primary: rgb(0xEAF7F6),
    text_secondary: rgb(0xBFE3E0),
    text_muted: rgb(0x7FA5A2),
    accent_cyan: rgb(0x39C5BB),
    accent_cyan_hover: rgb(0x62D8CF),
    accent_purple: rgb(0xD12A7E),
    accent_purple_hover: rgb(0xE24593),
    accent_emerald: rgb(0x7FE0A8),
    accent_rose: rgb(0xFF7AA2),
    accent_amber: rgb(0xF7C26B),
    accent_info: rgb(0x7FD3F7),
};

/// The game's own look: slate windows, warm off-white text, gold highlights.
const RIMWORLD: ThemeColors = ThemeColors {
    bg_dark: rgb(0x16181A),
    bg_surface: rgb(0x1E2124),
    bg_surface_hover: rgb(0x2E3338),
    bg_card: rgb(0x24282C),
    bg_sidebar: rgb(0x16181A),
    border_subtle: rgb(0x33383D),
    border_strong: rgb(0x4B5259),
    text_primary: rgb(0xEAE5D7),
    text_secondary: rgb(0xCBC4B2),
    text_muted: rgb(0x938D7E),
    accent_cyan: rgb(0xE0C068),
    accent_cyan_hover: rgb(0xEDD08A),
    accent_purple: rgb(0x8A6B2B),
    accent_purple_hover: rgb(0xA07F33),
    accent_emerald: rgb(0x8CC47A),
    accent_rose: rgb(0xE0664F),
    accent_amber: rgb(0xEAA640),
    accent_info: rgb(0x86B4E0),
};

/// The base look of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeKind {
    #[default]
    Lavender,
    Dark,
    /// Called "Miku" at first (a saved choice still loads).
    #[serde(alias = "Miku")]
    Mint,
    RimWorld,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 4] = [
        ThemeKind::Lavender,
        ThemeKind::Dark,
        ThemeKind::Mint,
        ThemeKind::RimWorld,
    ];

    pub fn colors(self) -> ThemeColors {
        match self {
            ThemeKind::Lavender => LAVENDER,
            ThemeKind::Dark => DARK,
            ThemeKind::Mint => MINT,
            ThemeKind::RimWorld => RIMWORLD,
        }
    }

    pub fn key(self) -> crate::i18n::TranslationKey {
        use crate::i18n::TranslationKey as K;
        match self {
            ThemeKind::Lavender => K::ThemeLavender,
            ThemeKind::Dark => K::ThemeDark,
            ThemeKind::Mint => K::ThemeMint,
            ThemeKind::RimWorld => K::ThemeRimWorld,
        }
    }
}

/// The accent of buttons, selection and links: the theme's own, or one of six palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Theme,
    Lavender,
    Ocean,
    Emerald,
    Amber,
    Rose,
    Graphite,
}

impl Accent {
    pub const ALL: [Accent; 7] = [
        Accent::Theme,
        Accent::Lavender,
        Accent::Ocean,
        Accent::Emerald,
        Accent::Amber,
        Accent::Rose,
        Accent::Graphite,
    ];

    /// Main accent, its hover, filled button, its hover; `None`: the theme's own.
    fn colors(self) -> Option<[Color32; 4]> {
        let hex = match self {
            Accent::Theme => return None,
            Accent::Lavender => [0xB69CFF, 0xCDB8FF, 0x7C5CE0, 0x8F72EC],
            Accent::Ocean => [0x7CC7FF, 0xA3D8FF, 0x2F7FD0, 0x3B90E0],
            Accent::Emerald => [0x6EE7B7, 0x96F0CE, 0x178A5F, 0x1F9D6E],
            Accent::Amber => [0xFBBF77, 0xFCD29E, 0xB8731B, 0xCC8424],
            Accent::Rose => [0xF7A1B8, 0xFAC0D0, 0xC9406A, 0xD9527C],
            Accent::Graphite => [0xC8CCD4, 0xDDE0E6, 0x5A606B, 0x6B7280],
        };
        Some(hex.map(rgb))
    }

    /// The color shown on the palette's button (the theme's own one for `Theme`).
    pub fn swatch(self, theme: ThemeKind) -> Color32 {
        self.colors().map_or(theme.colors().accent_cyan, |c| c[0])
    }

    pub fn key(self) -> crate::i18n::TranslationKey {
        use crate::i18n::TranslationKey as K;
        match self {
            Accent::Theme => K::AccentTheme,
            Accent::Lavender => K::AccentLavender,
            Accent::Ocean => K::AccentOcean,
            Accent::Emerald => K::AccentEmerald,
            Accent::Amber => K::AccentAmber,
            Accent::Rose => K::AccentRose,
            Accent::Graphite => K::AccentGraphite,
        }
    }
}

/// A theme with an accent palette on top.
pub fn build(theme: ThemeKind, accent: Accent) -> ThemeColors {
    let mut c = theme.colors();
    if let Some([main, main_hover, fill, fill_hover]) = accent.colors() {
        c.accent_cyan = main;
        c.accent_cyan_hover = main_hover;
        c.accent_purple = fill;
        c.accent_purple_hover = fill_hover;
    }
    c
}

/// Every theme × accent, built once; `CURRENT` says which one is shown.
fn table() -> &'static [ThemeColors] {
    static TABLE: OnceLock<Vec<ThemeColors>> = OnceLock::new();
    TABLE.get_or_init(|| {
        ThemeKind::ALL
            .iter()
            .flat_map(|t| Accent::ALL.iter().map(|a| build(*t, *a)))
            .collect()
    })
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn index(theme: ThemeKind, accent: Accent) -> usize {
    let t = ThemeKind::ALL.iter().position(|x| *x == theme).unwrap_or(0);
    let a = Accent::ALL.iter().position(|x| *x == accent).unwrap_or(0);
    t * Accent::ALL.len() + a
}

/// Chooses the colors every screen reads through `THEME` (then `apply_theme`).
pub fn set_theme(theme: ThemeKind, accent: Accent) {
    CURRENT.store(index(theme, accent), Ordering::Relaxed);
}

/// The colors of the chosen theme: `THEME.accent_cyan` and so on, as before.
pub struct CurrentTheme;

impl std::ops::Deref for CurrentTheme {
    type Target = ThemeColors;
    fn deref(&self) -> &ThemeColors {
        let table = table();
        &table[CURRENT.load(Ordering::Relaxed).min(table.len() - 1)]
    }
}

pub static THEME: CurrentTheme = CurrentTheme;

/// `color` mixed into `base` at `amount` (0..1) — an opaque "tinted" background.
pub fn mix(base: Color32, color: Color32, amount: f32) -> Color32 {
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(m(base.r(), color.r()), m(base.g(), color.g()), m(base.b(), color.b()))
}

/// Badge / chip background: the color at 18% over a card.
pub fn tint(color: Color32) -> Color32 {
    mix(THEME.bg_card, color, 0.18)
}

/// Gives egui's own widgets the colors of the chosen theme (at start and after a change).
pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    visuals.override_text_color = Some(THEME.text_primary);
    visuals.panel_fill = THEME.bg_dark;
    visuals.window_fill = THEME.bg_surface;
    visuals.window_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.extreme_bg_color = THEME.bg_dark;
    visuals.faint_bg_color = THEME.bg_card;
    visuals.hyperlink_color = THEME.accent_cyan;

    // Normal widgets
    visuals.widgets.noninteractive.bg_fill = THEME.bg_surface;
    visuals.widgets.noninteractive.weak_bg_fill = THEME.bg_surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, THEME.border_subtle);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, THEME.text_secondary);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);

    // Inactive buttons / inputs. `bg_fill` also paints slider tracks, checkbox boxes and
    // scrollbar handles: they need a visible color on a card (buttons use `weak_bg_fill`).
    visuals.widgets.inactive.bg_fill = THEME.border_strong;
    visuals.widgets.inactive.weak_bg_fill = THEME.bg_card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, THEME.text_secondary);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);

    // Hovered
    visuals.widgets.hovered.bg_fill = THEME.bg_surface_hover;
    visuals.widgets.hovered.weak_bg_fill = THEME.bg_surface_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, THEME.text_primary);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);

    // Pressed
    visuals.widgets.active.bg_fill = THEME.accent_purple;
    visuals.widgets.active.weak_bg_fill = THEME.accent_purple;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, THEME.accent_cyan);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, THEME.text_primary);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);

    // Open menus / combo boxes
    visuals.widgets.open.bg_fill = THEME.bg_surface_hover;
    visuals.widgets.open.weak_bg_fill = THEME.bg_surface_hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.widgets.open.corner_radius = CornerRadius::same(6);

    visuals.selection.bg_fill = tint(THEME.accent_cyan);
    visuals.selection.stroke = Stroke::new(1.0_f32, THEME.accent_cyan);

    ctx.set_visuals(visuals);

    // egui 0.37 made buttons roomier (padding 8x2, height 20); the layout of every tab is
    // made for the earlier 4x1 and 18 px.
    ctx.global_style_mut(|style| {
        style.spacing.button_padding = egui::vec2(4.0, 1.0);
        style.spacing.interact_size = egui::vec2(40.0, 18.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_and_accent_is_built_and_chosen() {
        assert_eq!(table().len(), ThemeKind::ALL.len() * Accent::ALL.len());
        // The theme's own accent keeps the theme as it is; a palette changes only the accent.
        for theme in ThemeKind::ALL {
            assert_eq!(build(theme, Accent::Theme), theme.colors());
            let ocean = build(theme, Accent::Ocean);
            assert_eq!(ocean.bg_card, theme.colors().bg_card);
            assert_eq!(ocean.accent_cyan, rgb(0x7CC7FF));
        }
        assert_eq!(Accent::Theme.swatch(ThemeKind::Mint), rgb(0x39C5BB));

        set_theme(ThemeKind::RimWorld, Accent::Rose);
        assert_eq!(THEME.bg_dark, RIMWORLD.bg_dark);
        assert_eq!(THEME.accent_purple, rgb(0xC9406A));
        set_theme(ThemeKind::Lavender, Accent::Theme);
        assert_eq!(*THEME, LAVENDER, "the first theme is the default");
    }

    #[test]
    fn text_stays_readable_on_every_theme() {
        // Relative luminance contrast (WCAG): body text at least 7:1 on the window and the
        // cards, muted text at least 3:1, the filled button's text at least 3:1.
        fn lum(c: Color32) -> f64 {
            let ch = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        }
        fn contrast(a: Color32, b: Color32) -> f64 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        for theme in ThemeKind::ALL {
            for accent in Accent::ALL {
                let c = build(theme, accent);
                let name = format!("{theme:?}/{accent:?}");
                assert!(contrast(c.text_primary, c.bg_dark) >= 7.0, "{name}");
                assert!(contrast(c.text_primary, c.bg_card) >= 7.0, "{name}");
                assert!(contrast(c.text_muted, c.bg_card) >= 3.0, "{name}");
                assert!(contrast(c.accent_cyan, c.bg_card) >= 3.0, "{name}: accent");
                assert!(
                    contrast(c.text_primary, c.accent_purple) >= 3.0,
                    "{name}: filled button"
                );
            }
        }
    }
}
