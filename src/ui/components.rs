use eframe::egui::{self, Color32, Pos2, Response, Rounding, Sense, Stroke, Ui, Vec2};
use super::theme::THEME;

/// Renders a modern badge pill (e.g. "Workshop", "DLC", "RU", "Tier 2")
pub fn badge(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) -> Response {
    let padding = Vec2::new(7.0, 3.0);
    let font_id = egui::FontId::proportional(12.0);

    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, fg);
    let desired_size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            Rounding::same(4.0),
            bg,
            Stroke::NONE,
        );
        let text_pos = Pos2::new(
            rect.min.x + padding.x,
            rect.min.y + padding.y,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
    response
}

/// Width a `badge` with this text takes.
pub fn badge_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    galley.size().x + 7.0 * 2.0
}

/// Width an `icon_badge` with this text takes.
pub fn icon_badge_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    let gap = if text.is_empty() { 0.0 } else { 5.0 };
    6.0 * 2.0 + 14.0 + gap + galley.size().x
}

/// Whether something this wide still fits in the row (plus the spacing before it).
pub fn fits(ui: &Ui, width: f32) -> bool {
    ui.available_width() >= width + ui.spacing().item_spacing.x
}

/// `badge` only if it fits completely; a half-cut badge looks broken.
pub fn badge_if_fits(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) -> Option<Response> {
    fits(ui, badge_width(ui, text)).then(|| badge(ui, text, bg, fg))
}

/// `icon_badge` only if it fits completely.
pub fn icon_badge_if_fits(
    ui: &mut Ui,
    icon: egui::ImageSource<'static>,
    text: &str,
    bg: Color32,
    fg: Color32,
) -> Option<Response> {
    fits(ui, icon_badge_width(ui, text)).then(|| icon_badge(ui, icon, text, bg, fg))
}

/// Badge pill with an SVG icon (tinted with `fg`) in front of the text.
pub fn icon_badge(
    ui: &mut Ui,
    icon: egui::ImageSource<'static>,
    text: &str,
    bg: Color32,
    fg: Color32,
) -> Response {
    let padding = Vec2::new(6.0, 3.0);
    let icon_size = 14.0;
    let gap = if text.is_empty() { 0.0 } else { 5.0 };
    let font_id = egui::FontId::proportional(12.0);

    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, fg);
    let content_h = galley.size().y.max(icon_size);
    let desired_size = Vec2::new(
        padding.x * 2.0 + icon_size + gap + galley.size().x,
        padding.y * 2.0 + content_h,
    );

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, Rounding::same(4.0), bg, Stroke::NONE);
        // Snap the icon to whole physical pixels: a half-pixel offset blurs thin lines.
        let ppp = ui.ctx().pixels_per_point();
        let snap = |v: f32| (v * ppp).round() / ppp;
        let icon_rect = egui::Rect::from_min_size(
            Pos2::new(snap(rect.min.x + padding.x), snap(rect.center().y - icon_size / 2.0)),
            Vec2::splat(icon_size),
        );
        egui::Image::new(icon)
            .texture_options(egui::TextureOptions::NEAREST)
            .tint(fg)
            .paint_at(ui, icon_rect);
        let text_pos = Pos2::new(
            icon_rect.max.x + gap,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
    response
}

/// Primary button with Cyan / Sky blue glow
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let padding = Vec2::new(14.0, 7.0);
    let font_id = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, Color32::WHITE);
    let desired_size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    if ui.is_rect_visible(rect) {
        let (bg, border) = if response.is_pointer_button_down_on() {
            (THEME.accent_cyan, THEME.accent_cyan_hover)
        } else if response.hovered() {
            (THEME.accent_cyan_hover, Color32::WHITE)
        } else {
            (THEME.accent_cyan, Color32::TRANSPARENT)
        };

        ui.painter().rect(
            rect,
            Rounding::same(6.0),
            bg,
            Stroke::new(1.0_f32, border),
        );
        let text_pos = Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, Color32::WHITE);
    }
    response
}

/// Danger / Red button (for deletes, unsubscribes)
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    let padding = Vec2::new(12.0, 6.0);
    let font_id = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, Color32::WHITE);
    let desired_size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.is_pointer_button_down_on() {
            Color32::from_rgb(190, 24, 60)
        } else if response.hovered() {
            Color32::from_rgb(225, 29, 72)
        } else {
            Color32::from_rgb(159, 18, 57)
        };

        ui.painter().rect(
            rect,
            Rounding::same(6.0),
            bg,
            Stroke::new(1.0_f32, Color32::from_rgb(244, 63, 94)),
        );
        let text_pos = Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, Color32::WHITE);
    }
    response
}

/// Success / Green button
pub fn success_button(ui: &mut Ui, text: &str) -> Response {
    let padding = Vec2::new(14.0, 7.0);
    let font_id = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, Color32::WHITE);
    let desired_size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            Color32::from_rgb(22, 163, 74)
        } else {
            Color32::from_rgb(21, 128, 61)
        };

        ui.painter().rect(
            rect,
            Rounding::same(6.0),
            bg,
            Stroke::new(1.0_f32, Color32::from_rgb(34, 197, 94)),
        );
        let text_pos = Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, Color32::WHITE);
    }
    response
}

/// Secondary button
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let padding = Vec2::new(12.0, 6.0);
    let font_id = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, THEME.text_primary);
    let desired_size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    if ui.is_rect_visible(rect) {
        let (bg, border) = if response.hovered() {
            (THEME.bg_surface_hover, THEME.border_strong)
        } else {
            (THEME.bg_card, THEME.border_subtle)
        };

        ui.painter().rect(
            rect,
            Rounding::same(6.0),
            bg,
            Stroke::new(1.0_f32, border),
        );
        let text_pos = Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, THEME.text_primary);
    }
    response
}

/// Search box with icon and clear button
pub fn search_input(ui: &mut Ui, query: &mut String, placeholder: &str) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(query)
                .hint_text(placeholder)
                .desired_width(ui.available_width().min(320.0))
                .margin(Vec2::new(8.0, 6.0)),
        );
        if !query.is_empty() && icon_button(ui, ICON_CLOSE, Vec2::new(12.0, 12.0), THEME.text_muted).clicked() {
            query.clear();
        }
    });
}

/// Card container with subtle border and dark background
pub fn card_container<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame {
        inner_margin: egui::Margin::same(12.0),
        outer_margin: egui::Margin::symmetric(0.0, 4.0),
        rounding: Rounding::same(10.0),
        shadow: egui::epaint::Shadow::NONE,
        fill: THEME.bg_card,
        stroke: Stroke::new(1.0_f32, THEME.border_subtle),
    };
    frame.show(ui, add_contents).inner
}

/// Header banner with title, subtitle, and badge
pub fn section_header(ui: &mut Ui, title: &str, subtitle: &str, badge_text: Option<(&str, Color32)>) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new(title)
                    .color(THEME.text_primary)
                    .size(18.0)
                    .strong(),
            );
            if let Some((b_text, b_color)) = badge_text {
                badge(ui, b_text, b_color.linear_multiply(0.2), b_color);
            }
        });
        if !subtitle.is_empty() {
            ui.label(
                egui::RichText::new(subtitle)
                    .color(THEME.text_muted)
                    .size(12.0),
            );
        }
        ui.add_space(4.0);
    });
}

use eframe::egui::ImageSource;
use super::icons::*;

/// Primary button with SVG icon (Cyan / Sky blue)
pub fn primary_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    let btn = egui::Button::image_and_text(
        svg_icon(icon, Vec2::new(16.0, 16.0), Color32::WHITE),
        egui::RichText::new(text).color(Color32::WHITE).size(13.0).strong(),
    )
    .fill(THEME.accent_cyan)
    .stroke(Stroke::new(1.0_f32, Color32::TRANSPARENT))
    .rounding(Rounding::same(6.0));
    ui.add(btn)
}

/// Secondary button with SVG icon (dark card background, subtle border)
pub fn secondary_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    let btn = egui::Button::image_and_text(
        svg_icon(icon, Vec2::new(16.0, 16.0), THEME.text_primary),
        egui::RichText::new(text).color(THEME.text_primary).size(13.0),
    )
    .fill(THEME.bg_card)
    .stroke(Stroke::new(1.0_f32, THEME.border_subtle))
    .rounding(Rounding::same(6.0));
    ui.add(btn)
}

/// Danger button with SVG icon (Red)
pub fn danger_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    let btn = egui::Button::image_and_text(
        svg_icon(icon, Vec2::new(16.0, 16.0), Color32::WHITE),
        egui::RichText::new(text).color(Color32::WHITE).size(13.0),
    )
    .fill(Color32::from_rgb(159, 18, 57))
    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(244, 63, 94)))
    .rounding(Rounding::same(6.0));
    ui.add(btn)
}

/// Success button with SVG icon (Green)
pub fn success_button_icon(ui: &mut Ui, icon: ImageSource<'static>, text: &str) -> Response {
    let btn = egui::Button::image_and_text(
        svg_icon(icon, Vec2::new(16.0, 16.0), Color32::WHITE),
        egui::RichText::new(text).color(Color32::WHITE).size(13.0),
    )
    .fill(Color32::from_rgb(21, 128, 61))
    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(34, 197, 94)))
    .rounding(Rounding::same(6.0));
    ui.add(btn)
}

/// Small icon-only button. Allocates exactly `size` + padding and draws the icon 1:1 on whole
/// pixels (egui's own image button shrinks the image to the row height, which loses thin lines).
pub fn icon_button(ui: &mut Ui, icon: ImageSource<'static>, size: Vec2, tint: Color32) -> Response {
    let pad = 3.0;
    let size = size.round();
    let (rect, response) = ui.allocate_exact_size(size + Vec2::splat(pad * 2.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if hovered {
            ui.painter().rect_filled(rect, Rounding::same(4.0), THEME.bg_surface_hover);
        }
        let ppp = ui.ctx().pixels_per_point();
        let snap = |v: f32| (v * ppp).round() / ppp;
        let icon_rect = egui::Rect::from_min_size(
            Pos2::new(snap(rect.min.x + pad), snap(rect.min.y + pad)),
            size,
        );
        egui::Image::new(icon)
            .texture_options(egui::TextureOptions::NEAREST)
            .tint(if hovered { THEME.text_primary } else { tint })
            .paint_at(ui, icon_rect);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
