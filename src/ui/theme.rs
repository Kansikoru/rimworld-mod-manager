use eframe::egui::{self, Color32, Rounding, Stroke, Visuals};

pub struct ThemeColors {
    pub bg_dark: Color32,
    pub bg_surface: Color32,
    pub bg_surface_hover: Color32,
    pub bg_card: Color32,
    pub bg_sidebar: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,

    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,

    pub accent_cyan: Color32,
    pub accent_cyan_hover: Color32,
    pub accent_purple: Color32,
    pub accent_purple_hover: Color32,
    pub accent_emerald: Color32,
    pub accent_rose: Color32,
    pub accent_amber: Color32,
}

pub const THEME: ThemeColors = ThemeColors {
    bg_dark: Color32::from_rgb(7, 10, 18),           // #070A12 - Ultra deep space background
    bg_surface: Color32::from_rgb(11, 15, 25),       // #0B0F19 - Charcoal-navy surface
    bg_surface_hover: Color32::from_rgb(20, 28, 46), // #141C2E - Hover state
    bg_card: Color32::from_rgb(14, 20, 34),          // #0E1422 - Mod card item
    bg_sidebar: Color32::from_rgb(6, 9, 15),         // #06090F - Deepest sidebar
    border_subtle: Color32::from_rgb(24, 34, 52),    // #182234 - Subtle border
    border_strong: Color32::from_rgb(42, 60, 92),    // #2A3C5C - Border focus

    text_primary: Color32::from_rgb(241, 245, 249),  // #F1F5F9 - Crisp primary text
    text_secondary: Color32::from_rgb(203, 213, 225),// #CBD5E1 - Secondary text
    text_muted: Color32::from_rgb(100, 116, 139),    // #64748B - Muted labels

    accent_cyan: Color32::from_rgb(56, 189, 248),    // #38BDF8 - Sky-cyan
    accent_cyan_hover: Color32::from_rgb(125, 211, 252), // #7DD3FC - Hover
    accent_purple: Color32::from_rgb(168, 85, 247),  // #A855F7 - Purple
    accent_purple_hover: Color32::from_rgb(192, 132, 252),
    accent_emerald: Color32::from_rgb(34, 197, 94),  // #22C55E - Green
    accent_rose: Color32::from_rgb(244, 63, 94),     // #F43F5E - Rose
    accent_amber: Color32::from_rgb(245, 158, 11),   // #F59E0B - Amber/Gold
};

pub fn apply_dark_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    visuals.override_text_color = Some(THEME.text_primary);
    visuals.panel_fill = THEME.bg_surface;
    visuals.window_fill = THEME.bg_surface;
    visuals.extreme_bg_color = THEME.bg_dark;

    // Normal widgets
    visuals.widgets.noninteractive.bg_fill = THEME.bg_surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, THEME.border_subtle);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, THEME.text_secondary);
    visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

    // Inactive button/inputs
    visuals.widgets.inactive.bg_fill = THEME.bg_card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, THEME.border_subtle);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, THEME.text_primary);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);

    // Hovered widgets
    visuals.widgets.hovered.bg_fill = THEME.bg_surface_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, THEME.accent_cyan_hover);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);

    // Active (pressed) widgets
    visuals.widgets.active.bg_fill = THEME.accent_cyan;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, THEME.accent_cyan_hover);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.rounding = Rounding::same(6.0);

    visuals.selection.bg_fill = THEME.accent_cyan.linear_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, THEME.accent_cyan);

    ctx.set_visuals(visuals);
}
