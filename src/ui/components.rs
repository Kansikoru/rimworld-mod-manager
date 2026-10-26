use eframe::egui::{self, Color32, ImageSource, Pos2, Rect, Response, Rounding, Sense, Stroke, Ui, Vec2};

use super::icons::*;
use super::theme::{mix, tint, THEME};

/// Opens a folder in Explorer or a link in the default browser. One way for the
/// whole app; doesn't wait for the program it starts.
pub fn open_external(target: impl AsRef<std::ffi::OsStr>) {
    let _ = open::that_detached(target);
}

/// Unix time as a local date and time: "26.10.2026 14:32" (Russian) or "2026-10-26 14:32".
pub fn local_date_time(unix_secs: u64) -> String {
    let Some(utc) = chrono::DateTime::from_timestamp(unix_secs as i64, 0) else {
        return String::new();
    };
    let format = crate::i18n::tr!("%d.%m.%Y %H:%M", "%Y-%m-%d %H:%M");
    utc.with_timezone(&chrono::Local).format(format).to_string()
}

/// Snaps a coordinate to whole physical pixels: thin icon lines stay sharp.
fn snap(ui: &Ui, v: f32) -> f32 {
    let ppp = ui.ctx().pixels_per_point();
    (v * ppp).round() / ppp
}

/// Paints an SVG icon 1:1 on whole pixels, tinted.
fn paint_icon(ui: &Ui, icon: ImageSource<'static>, center: Pos2, size: f32, tint_color: Color32) {
    let rect = Rect::from_min_size(
        Pos2::new(snap(ui, center.x - size / 2.0), snap(ui, center.y - size / 2.0)),
        Vec2::splat(size),
    );
    egui::Image::new(icon)
        .texture_options(egui::TextureOptions::NEAREST)
        .tint(tint_color)
        .paint_at(ui, rect);
}

// ---------------------------------------------------------------- badges

const BADGE_PAD: Vec2 = Vec2::new(6.0, 4.0);
const BADGE_ICON: f32 = 14.0;

/// Small pill with text (category, version, status). `bg` is usually `tint(fg)`.
pub fn badge(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), fg);
    let (rect, response) = ui.allocate_exact_size(galley.size() + BADGE_PAD * 2.0, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, Rounding::same(6.0), bg, Stroke::NONE);
        ui.painter().galley(rect.min + BADGE_PAD, galley, fg);
    }
    response
}

/// Badge: `color` text on the same color at 18%.
pub fn tag(ui: &mut Ui, text: &str, color: Color32) -> Response {
    badge(ui, text, tint(color), color)
}

/// Width a `badge` with this text takes.
pub fn badge_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    galley.size().x + BADGE_PAD.x * 2.0
}

/// Width an `icon_badge` with this text takes.
pub fn icon_badge_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    let gap = if text.is_empty() { 0.0 } else { 5.0 };
    BADGE_PAD.x * 2.0 + BADGE_ICON + gap + galley.size().x
}

/// Whether something this wide still fits in the row (plus the spacing before it).
pub fn fits(ui: &Ui, width: f32) -> bool {
    ui.available_width() >= width + ui.spacing().item_spacing.x
}

/// `badge` only if it fits completely; a half-cut badge looks broken.
pub fn badge_if_fits(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) -> Option<Response> {
    fits(ui, badge_width(ui, text)).then(|| badge(ui, text, bg, fg))
}

/// `tag` only if it fits completely.
pub fn tag_if_fits(ui: &mut Ui, text: &str, color: Color32) -> Option<Response> {
    badge_if_fits(ui, text, tint(color), color)
}

/// `icon_badge` only if it fits completely.
pub fn icon_badge_if_fits(
    ui: &mut Ui,
    icon: ImageSource<'static>,
    text: &str,
    bg: Color32,
    fg: Color32,
) -> Option<Response> {
    fits(ui, icon_badge_width(ui, text)).then(|| icon_badge(ui, icon, text, bg, fg))
}

/// `icon_badge` with `color` on 18% of it, only if it fits.
pub fn icon_tag_if_fits(ui: &mut Ui, icon: ImageSource<'static>, text: &str, color: Color32) -> Option<Response> {
    icon_badge_if_fits(ui, icon, text, tint(color), color)
}

/// Badge pill with an SVG icon (tinted with `fg`) in front of the text.
pub fn icon_badge(ui: &mut Ui, icon: ImageSource<'static>, text: &str, bg: Color32, fg: Color32) -> Response {
    let gap = if text.is_empty() { 0.0 } else { 5.0 };
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), fg);
    let content_h = galley.size().y.max(BADGE_ICON);
    let size = Vec2::new(BADGE_PAD.x * 2.0 + BADGE_ICON + gap + galley.size().x, BADGE_PAD.y * 2.0 + content_h);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, Rounding::same(6.0), bg, Stroke::NONE);
        let icon_center = Pos2::new(rect.min.x + BADGE_PAD.x + BADGE_ICON / 2.0, rect.center().y);
        paint_icon(ui, icon, icon_center, BADGE_ICON, fg);
        let text_pos = Pos2::new(rect.min.x + BADGE_PAD.x + BADGE_ICON + gap, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley, fg);
    }
    response
}

// ---------------------------------------------------------------- buttons

/// Button styles.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Filled deep violet (the main action, e.g. "Save Order").
    Primary,
    /// Dark card with a border.
    Secondary,
    /// Mint on 18% mint ("Play", "Enable").
    Success,
    /// Rose on 18% rose (delete).
    Danger,
}

/// One painter for every button: 32 px tall, 12 px side padding, 16 px icon, 12 px semibold text.
pub fn styled_button(ui: &mut Ui, icon: Option<ImageSource<'static>>, text: &str, style: ButtonStyle) -> Response {
    styled_button_sized(ui, icon, text, style, 32.0, 12.0)
}

/// Width a `styled_button` takes (icon-only when `text` is empty).
pub fn styled_button_width(ui: &Ui, has_icon: bool, text: &str) -> f32 {
    let text_w = if text.is_empty() {
        0.0
    } else {
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE)
            .size()
            .x
    };
    let icon_w = if has_icon { 16.0 + if text.is_empty() { 0.0 } else { 8.0 } } else { 0.0 };
    12.0 * 2.0 + icon_w + text_w
}

/// Compact 24 px button for list rows ("Enable" next to a missing dependency).
pub fn small_button(ui: &mut Ui, text: &str, style: ButtonStyle) -> Response {
    styled_button_sized(ui, None, text, style, 24.0, 8.0)
}

fn styled_button_sized(
    ui: &mut Ui,
    icon: Option<ImageSource<'static>>,
    text: &str,
    style: ButtonStyle,
    height: f32,
    pad: f32,
) -> Response {
    let font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
    let (fill, hover_fill, stroke, fg, hover_fg) = match style {
        ButtonStyle::Primary => (THEME.accent_purple, THEME.accent_purple_hover, THEME.accent_purple, THEME.text_primary, THEME.text_primary),
        ButtonStyle::Secondary => (THEME.bg_card, THEME.bg_surface_hover, THEME.border_strong, THEME.text_secondary, THEME.text_primary),
        ButtonStyle::Success => (
            tint(THEME.accent_emerald),
            mix(THEME.bg_card, THEME.accent_emerald, 0.28),
            tint(THEME.accent_emerald),
            THEME.accent_emerald,
            THEME.accent_emerald,
        ),
        ButtonStyle::Danger => (
            tint(THEME.accent_rose),
            mix(THEME.bg_card, THEME.accent_rose, 0.28),
            mix(THEME.bg_card, THEME.accent_rose, 0.4),
            THEME.accent_rose,
            THEME.accent_rose,
        ),
    };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let icon_w = if icon.is_some() { 16.0 + if text.is_empty() { 0.0 } else { 8.0 } } else { 0.0 };
    let size = Vec2::new(pad * 2.0 + icon_w + galley.size().x, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && response.hovered();
        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let bg = if hovered { hover_fill } else { fill };
        ui.painter().rect(rect, Rounding::same(6.0), dim(bg), Stroke::new(1.0_f32, dim(stroke)));
        let color = dim(if hovered { hover_fg } else { fg });
        let mut x = rect.min.x + pad;
        if let Some(icon) = icon {
            paint_icon(ui, icon, Pos2::new(x + 8.0, rect.center().y), 16.0, color);
            x += icon_w;
        }
        ui.painter().galley(Pos2::new(snap(ui, x), snap(ui, rect.center().y - galley.size().y / 2.0)), galley, color);
    }
    if ui.is_enabled() {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, None, text, ButtonStyle::Primary)
}

pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, None, text, ButtonStyle::Secondary)
}

pub fn success_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, None, text, ButtonStyle::Success)
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, None, text, ButtonStyle::Danger)
}

pub fn primary_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    styled_button(ui, Some(icon), text, ButtonStyle::Primary)
}

pub fn secondary_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    styled_button(ui, Some(icon), text, ButtonStyle::Secondary)
}

pub fn danger_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    styled_button(ui, Some(icon), text, ButtonStyle::Danger)
}

pub fn success_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    styled_button(ui, Some(icon), text, ButtonStyle::Success)
}

/// Small icon-only button without a frame (icon 1:1 on whole pixels; a light square on hover).
pub fn icon_button(ui: &mut Ui, icon: ImageSource<'static>, size: Vec2, tint_color: Color32) -> Response {
    let pad = 3.0;
    let size = size.round();
    let (rect, response) = ui.allocate_exact_size(size + Vec2::splat(pad * 2.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if hovered {
            ui.painter().rect_filled(rect, Rounding::same(4.0), THEME.bg_surface_hover);
        }
        paint_icon(ui, icon, rect.center(), size.x, if hovered { THEME.text_primary } else { tint_color });
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 24×24 square button with a border and a 16 px icon (row actions: ↑ ↓ ✕ +).
pub fn boxed_icon_button(ui: &mut Ui, icon: ImageSource<'static>) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if hovered { THEME.bg_surface_hover } else { THEME.bg_card };
        ui.painter().rect(rect, Rounding::same(6.0), fill, Stroke::new(1.0_f32, THEME.border_strong));
        let color = if hovered { THEME.text_primary } else { THEME.text_secondary };
        paint_icon(ui, icon, rect.center(), 16.0, color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Top-bar tab: text only, 40 px tall; the active one is lavender on the hover color with a
/// 2 px lavender line at the bottom.
pub fn nav_tab(ui: &mut Ui, label: &str, active: bool) -> Response {
    let color = if active { THEME.accent_cyan } else { THEME.text_secondary };
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), color);
    let size = Vec2::new(galley.size().x + 24.0, 40.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if active || hovered {
            ui.painter().rect_filled(rect, Rounding::same(6.0), THEME.bg_surface_hover);
        }
        let text_color = if hovered && !active { THEME.text_primary } else { color };
        let pos = Pos2::new(snap(ui, rect.min.x + 12.0), snap(ui, rect.center().y - galley.size().y / 2.0));
        ui.painter().galley(pos, galley, text_color);
        if active {
            let line = Rect::from_min_max(
                Pos2::new(rect.min.x + 12.0, rect.max.y - 2.0),
                Pos2::new(rect.max.x - 12.0, rect.max.y),
            );
            ui.painter().rect_filled(line, Rounding::same(1.0), THEME.accent_cyan);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Filter chip: label + muted count; selected = lavender on 18% lavender.
pub fn chip(ui: &mut Ui, icon: Option<(ImageSource<'static>, Color32)>, label: &str, count: usize, selected: bool) -> Response {
    let label_font = egui::FontId::proportional(12.0);
    let (fill, stroke, label_color, count_color) = if selected {
        (tint(THEME.accent_cyan), THEME.border_strong, THEME.accent_cyan, THEME.accent_cyan)
    } else {
        (THEME.bg_surface, THEME.border_subtle, THEME.text_secondary, THEME.text_muted)
    };
    let label_g = ui.painter().layout_no_wrap(label.to_string(), label_font.clone(), label_color);
    let count_g = ui.painter().layout_no_wrap(count.to_string(), label_font, count_color);
    let pad = 8.0;
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let size = Vec2::new(pad * 2.0 + icon_w + label_g.size().x + 6.0 + count_g.size().x, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() && !selected { THEME.bg_surface_hover } else { fill };
        ui.painter().rect(rect, Rounding::same(6.0), fill, Stroke::new(1.0_f32, stroke));
        let mut x = rect.min.x + pad;
        if let Some((icon, color)) = icon {
            paint_icon(ui, icon, Pos2::new(x + 7.0, rect.center().y), 14.0, if selected { THEME.accent_cyan } else { color });
            x += icon_w;
        }
        let y = snap(ui, rect.center().y - label_g.size().y / 2.0);
        let lw = label_g.size().x;
        ui.painter().galley(Pos2::new(snap(ui, x), y), label_g, label_color);
        ui.painter().galley(Pos2::new(snap(ui, x + lw + 6.0), y), count_g, count_color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------- inputs and containers

/// Search field: dark field, border, search icon, clear button.
pub fn search_input(ui: &mut Ui, query: &mut String, placeholder: &str) {
    search_input_width(ui, query, placeholder, ui.available_width().min(320.0));
}

/// `search_input` with an explicit width.
pub fn search_input_width(ui: &mut Ui, query: &mut String, placeholder: &str, width: f32) {
    egui::Frame {
        inner_margin: egui::Margin::symmetric(8.0, 0.0),
        outer_margin: egui::Margin::ZERO,
        rounding: Rounding::same(6.0),
        shadow: egui::epaint::Shadow::NONE,
        fill: THEME.bg_dark,
        stroke: Stroke::new(1.0_f32, THEME.border_subtle),
    }
    .show(ui, |ui| {
        ui.set_width((width - 16.0).max(80.0));
        ui.set_height(30.0);
        // Always icon → text → clear button, even inside a right-to-left row
        // (a plain `horizontal` would inherit that order and put "×" on the left).
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(svg_icon(ICON_SEARCH, Vec2::splat(16.0), THEME.text_muted));
            let clear_w = if query.is_empty() { 0.0 } else { 26.0 };
            ui.add(
                egui::TextEdit::singleline(query)
                    .hint_text(egui::RichText::new(placeholder).color(THEME.text_muted).size(12.0))
                    .frame(false)
                    .font(egui::FontId::proportional(12.0))
                    .desired_width((ui.available_width() - clear_w).max(40.0)),
            );
            if !query.is_empty() && icon_button(ui, ICON_CLOSE, Vec2::new(12.0, 12.0), THEME.text_muted).clicked() {
                query.clear();
            }
        });
    });
}

/// Card: panel color, subtle border, 6 px corners.
pub fn card_container<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame {
        inner_margin: egui::Margin::same(12.0),
        outer_margin: egui::Margin::symmetric(0.0, 4.0),
        rounding: Rounding::same(6.0),
        shadow: egui::epaint::Shadow::NONE,
        fill: THEME.bg_surface,
        stroke: Stroke::new(1.0_f32, THEME.border_subtle),
    };
    frame.show(ui, add_contents).inner
}

/// Page title with an optional badge and a muted subtitle.
pub fn section_header(ui: &mut Ui, title: &str, subtitle: &str, badge_text: Option<(&str, Color32)>) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new(title).color(THEME.text_primary).size(18.0).strong());
            if let Some((b_text, b_color)) = badge_text {
                tag(ui, b_text, b_color);
            }
        });
        if !subtitle.is_empty() {
            ui.label(egui::RichText::new(subtitle).color(THEME.text_muted).size(12.0));
        }
        ui.add_space(4.0);
    });
}
