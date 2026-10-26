use eframe::egui::{self, Color32, Rounding, Stroke, Visuals};

/// Interface colors (soft lavender on deep violet-black).
/// Field names are kept from the old theme so every screen picks up the new palette:
/// `accent_cyan` is the main accent (lavender), `accent_purple` the filled primary button.
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

pub const THEME: ThemeColors = ThemeColors {
    bg_dark: Color32::from_rgb(0x0E, 0x0B, 0x16),
    bg_surface: Color32::from_rgb(0x15, 0x10, 0x21),
    bg_surface_hover: Color32::from_rgb(0x25, 0x1C, 0x3D),
    bg_card: Color32::from_rgb(0x1B, 0x15, 0x30),
    bg_sidebar: Color32::from_rgb(0x0E, 0x0B, 0x16),
    border_subtle: Color32::from_rgb(0x2E, 0x25, 0x47),
    border_strong: Color32::from_rgb(0x45, 0x38, 0x6B),

    text_primary: Color32::from_rgb(0xF3, 0xF0, 0xFA),
    text_secondary: Color32::from_rgb(0xCF, 0xC7, 0xE6),
    text_muted: Color32::from_rgb(0x8B, 0x82, 0xA8),

    accent_cyan: Color32::from_rgb(0xB6, 0x9C, 0xFF),
    accent_cyan_hover: Color32::from_rgb(0xCD, 0xB8, 0xFF),
    accent_purple: Color32::from_rgb(0x7C, 0x5C, 0xE0),
    accent_purple_hover: Color32::from_rgb(0x8F, 0x72, 0xEC),
    accent_emerald: Color32::from_rgb(0x6E, 0xE7, 0xB7),
    accent_rose: Color32::from_rgb(0xF4, 0x7F, 0xA0),
    accent_amber: Color32::from_rgb(0xFB, 0xBF, 0x77),
    accent_info: Color32::from_rgb(0x8E, 0xC5, 0xFF),
};

/// `color` mixed into `base` at `amount` (0..1) — an opaque "tinted" background.
pub fn mix(base: Color32, color: Color32, amount: f32) -> Color32 {
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(m(base.r(), color.r()), m(base.g(), color.g()), m(base.b(), color.b()))
}

/// Badge / chip background: the color at 18% over a card.
pub fn tint(color: Color32) -> Color32 {
    mix(THEME.bg_card, color, 0.18)
}

pub fn apply_dark_theme(ctx: &egui::Context) {
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
    visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

    // Inactive buttons / inputs. `bg_fill` also paints slider tracks, checkbox boxes and
    // scrollbar handles: they need a visible color on a card (buttons use `weak_bg_fill`).
    visuals.widgets.inactive.bg_fill = THEME.border_strong;
    visuals.widgets.inactive.weak_bg_fill = THEME.bg_card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, THEME.text_secondary);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);

    // Hovered
    visuals.widgets.hovered.bg_fill = THEME.bg_surface_hover;
    visuals.widgets.hovered.weak_bg_fill = THEME.bg_surface_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, THEME.text_primary);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);

    // Pressed
    visuals.widgets.active.bg_fill = THEME.accent_purple;
    visuals.widgets.active.weak_bg_fill = THEME.accent_purple;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, THEME.accent_cyan);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, THEME.text_primary);
    visuals.widgets.active.rounding = Rounding::same(6.0);

    // Open menus / combo boxes
    visuals.widgets.open.bg_fill = THEME.bg_surface_hover;
    visuals.widgets.open.weak_bg_fill = THEME.bg_surface_hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, THEME.border_strong);
    visuals.widgets.open.rounding = Rounding::same(6.0);

    visuals.selection.bg_fill = tint(THEME.accent_cyan);
    visuals.selection.stroke = Stroke::new(1.0_f32, THEME.accent_cyan);

    ctx.set_visuals(visuals);
}
