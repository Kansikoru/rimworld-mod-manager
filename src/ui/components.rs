use eframe::egui::{self, Color32, CornerRadius, ImageSource, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::icons::*;
use super::theme::{mix, tint, THEME};

/// Opens a folder in Explorer or a link in the default browser. One way for the
/// whole app; doesn't wait for the program it starts.
pub fn open_external(target: impl AsRef<std::ffi::OsStr>) {
    let _ = open::that_detached(target);
}

fn workshop_request_id() -> egui::Id {
    egui::Id::unique("open_workshop_item")
}

/// Asks to show a Workshop mod in the program's own Workshop tab, not in the browser. Kept
/// in egui's memory, so any view can ask while it is reading the app's lists; the app opens
/// it on the next frame (`App::open_workshop_items`).
pub fn open_in_workshop_tab(ctx: &egui::Context, item_id: &str) {
    open_many_in_workshop_tab(ctx, vec![item_id.to_string()]);
}

/// Several mods at once: the tab shows just them, the first one picked.
pub fn open_many_in_workshop_tab(ctx: &egui::Context, item_ids: Vec<String>) {
    if item_ids.is_empty() {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(workshop_request_id(), item_ids));
    ctx.request_repaint();
}

pub fn take_workshop_request(ctx: &egui::Context) -> Option<Vec<String>> {
    ctx.data_mut(|d| d.remove_temp::<Vec<String>>(workshop_request_id()))
}

fn changelog_request_id() -> egui::Id {
    egui::Id::unique("open_changelog")
}

/// Asks for the "What changed" window of a Workshop mod, the same way as
/// `open_in_workshop_tab` (`App::open_changelog` runs on the next frame).
pub fn open_changelog_window(ctx: &egui::Context, item_id: &str) {
    ctx.data_mut(|d| d.insert_temp(changelog_request_id(), item_id.to_string()));
    ctx.request_repaint();
}

pub fn take_changelog_request(ctx: &egui::Context) -> Option<String> {
    ctx.data_mut(|d| d.remove_temp::<String>(changelog_request_id()))
}

/// Unix time as a local date and time: "26.10.2026 14:32" (Russian) or "2026-10-26 14:32".
pub fn local_date_time(unix_secs: u64) -> String {
    let Some(utc) = chrono::DateTime::from_timestamp(unix_secs as i64, 0) else {
        return String::new();
    };
    let format = crate::i18n::tr!("%d.%m.%Y %H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M");
    utc.with_timezone(&chrono::Local).format(format).to_string()
}

/// Unix time as a local date: "26.10.2026" (Russian) or "2026-10-26".
pub fn local_date(unix_secs: u64) -> String {
    let Some(utc) = chrono::DateTime::from_timestamp(unix_secs as i64, 0) else {
        return String::new();
    };
    let format = crate::i18n::tr!("%d.%m.%Y", "%Y-%m-%d", "%Y-%m-%d");
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
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), fg);
    let (rect, response) = ui.allocate_exact_size(galley.size() + BADGE_PAD * 2.0, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect(rect, CornerRadius::same(6), bg, Stroke::NONE, egui::StrokeKind::Middle);
        ui.painter().galley(rect.min + BADGE_PAD, galley, fg);
    }
    response
}

/// Badge: `color` text on the same color at 18%.
pub fn tag(ui: &mut Ui, text: &str, color: Color32) -> Response {
    badge(ui, text, tint(color), color)
}

/// Height of a `badge` (and of an `icon_badge`) with the current font.
pub fn badge_height(ui: &Ui) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap("Ag".to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    galley.size().y.max(BADGE_ICON) + BADGE_PAD.y * 2.0
}

/// Width a `badge` with this text takes.
pub fn badge_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    galley.size().x + BADGE_PAD.x * 2.0
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

/// Badge pill with an SVG icon (tinted with `fg`) in front of the text.
pub fn icon_badge(ui: &mut Ui, icon: ImageSource<'static>, text: &str, bg: Color32, fg: Color32) -> Response {
    let gap = if text.is_empty() { 0.0 } else { 5.0 };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), fg);
    let content_h = galley.size().y.max(BADGE_ICON);
    let size = Vec2::new(
        BADGE_PAD.x * 2.0 + BADGE_ICON + gap + galley.size().x,
        BADGE_PAD.y * 2.0 + content_h,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect(rect, CornerRadius::same(6), bg, Stroke::NONE, egui::StrokeKind::Middle);
        let icon_center = Pos2::new(rect.min.x + BADGE_PAD.x + BADGE_ICON / 2.0, rect.center().y);
        paint_icon(ui, icon, icon_center, BADGE_ICON, fg);
        let text_pos = Pos2::new(
            rect.min.x + BADGE_PAD.x + BADGE_ICON + gap,
            rect.center().y - galley.size().y / 2.0,
        );
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
    let icon_w = if has_icon {
        16.0 + if text.is_empty() { 0.0 } else { 8.0 }
    } else {
        0.0
    };
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
        ButtonStyle::Primary => (
            THEME.accent_purple,
            THEME.accent_purple_hover,
            THEME.accent_purple,
            THEME.text_primary,
            THEME.text_primary,
        ),
        ButtonStyle::Secondary => (
            THEME.bg_card,
            THEME.bg_surface_hover,
            THEME.border_strong,
            THEME.text_secondary,
            THEME.text_primary,
        ),
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
    let icon_w = if icon.is_some() {
        16.0 + if text.is_empty() { 0.0 } else { 8.0 }
    } else {
        0.0
    };
    let size = Vec2::new(pad * 2.0 + icon_w + galley.size().x, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && response.hovered();
        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let bg = if hovered { hover_fill } else { fill };
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            dim(bg),
            Stroke::new(1.0_f32, dim(stroke)),
            egui::StrokeKind::Middle,
        );
        let color = dim(if hovered { hover_fg } else { fg });
        let mut x = rect.min.x + pad;
        if let Some(icon) = icon {
            paint_icon(ui, icon, Pos2::new(x + 8.0, rect.center().y), 16.0, color);
            x += icon_w;
        }
        ui.painter().galley(
            Pos2::new(snap(ui, x), snap(ui, rect.center().y - galley.size().y / 2.0)),
            galley,
            color,
        );
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
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), THEME.bg_surface_hover);
        }
        paint_icon(
            ui,
            icon,
            rect.center(),
            size.x,
            if hovered { THEME.text_primary } else { tint_color },
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 24×24 square button with a border and a 16 px icon (row actions: ↑ ↓ ✕ +).
pub fn boxed_icon_button(ui: &mut Ui, icon: ImageSource<'static>) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if hovered { THEME.bg_surface_hover } else { THEME.bg_card };
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            fill,
            Stroke::new(1.0_f32, THEME.border_strong),
            egui::StrokeKind::Middle,
        );
        let color = if hovered {
            THEME.text_primary
        } else {
            THEME.text_secondary
        };
        paint_icon(ui, icon, rect.center(), 16.0, color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Top-bar tab: text only, 40 px tall; the active one is lavender on the hover color with a
/// 2 px lavender line at the bottom.
/// Below this width, pages with a list and a details panel put them one above the other.
pub const NARROW_TWO_COLUMNS: f32 = 940.0;

/// D1: under "subscribe to the missing mods": how many missing mods have no Workshop link,
/// and (without the Workshop ID database yet) a button to load it. Returns whether the
/// button was clicked.
pub fn workshop_links_hint(ui: &mut Ui, t: crate::i18n::I18n, no_link: usize, have_index: bool, busy: bool) -> bool {
    use crate::i18n::{fmt, TranslationKey};
    if no_link == 0 {
        return false;
    }
    let mut clicked = false;
    if have_index {
        ui.add(
            egui::Label::new(
                egui::RichText::new(fmt(t.get(TranslationKey::WorkshopDbNoLink), &[&no_link.to_string()]))
                    .color(THEME.text_muted)
                    .size(12.0),
            )
            .wrap(),
        );
    } else {
        ui.add(
            egui::Label::new(
                egui::RichText::new(fmt(t.get(TranslationKey::WorkshopDbHint), &[&no_link.to_string()]))
                    .color(THEME.accent_amber)
                    .size(12.0),
            )
            .wrap(),
        );
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                clicked = secondary_button_icon(ui, crate::ui::icons::ICON_STEAM, t.get(TranslationKey::WorkshopDbBtn))
                    .on_hover_text(t.get(TranslationKey::WorkshopDbBtnHint))
                    .clicked();
            });
            if busy {
                ui.spinner();
            }
        });
    }
    clicked
}

/// A dialog's content width and its largest height for the current window: as wide as
/// wanted if the window allows, never taller than the window (the rest scrolls).
pub fn dialog_size(ctx: &egui::Context, wanted_width: f32) -> (f32, f32) {
    let screen = ctx.content_rect();
    (
        wanted_width.min(screen.width() - 64.0).max(240.0),
        (screen.height() - 96.0).max(200.0),
    )
}

/// Filter chip: label + muted count (none for 0); selected = accent on an accent tint.
pub fn chip(
    ui: &mut Ui,
    icon: Option<(ImageSource<'static>, Color32)>,
    label: &str,
    count: usize,
    selected: bool,
) -> Response {
    let label_font = egui::FontId::proportional(12.0);
    let (fill, stroke, label_color, count_color) = if selected {
        (
            tint(THEME.accent_cyan),
            THEME.border_strong,
            THEME.accent_cyan,
            THEME.accent_cyan,
        )
    } else {
        (
            THEME.bg_surface,
            THEME.border_subtle,
            THEME.text_secondary,
            THEME.text_muted,
        )
    };
    let label_g = ui
        .painter()
        .layout_no_wrap(label.to_string(), label_font.clone(), label_color);
    // A zero count isn't drawn: such a chip is a plain on/off option.
    let count_text = if count > 0 { count.to_string() } else { String::new() };
    let count_g = ui.painter().layout_no_wrap(count_text, label_font, count_color);
    let pad = 8.0;
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let count_w = if count > 0 { 6.0 + count_g.size().x } else { 0.0 };
    let size = Vec2::new(pad * 2.0 + icon_w + label_g.size().x + count_w, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() && !selected {
            THEME.bg_surface_hover
        } else {
            fill
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            fill,
            Stroke::new(1.0_f32, stroke),
            egui::StrokeKind::Middle,
        );
        let mut x = rect.min.x + pad;
        if let Some((icon, color)) = icon {
            paint_icon(
                ui,
                icon,
                Pos2::new(x + 7.0, rect.center().y),
                14.0,
                if selected { THEME.accent_cyan } else { color },
            );
            x += icon_w;
        }
        let y = snap(ui, rect.center().y - label_g.size().y / 2.0);
        let lw = label_g.size().x;
        ui.painter().galley(Pos2::new(snap(ui, x), y), label_g, label_color);
        ui.painter()
            .galley(Pos2::new(snap(ui, x + lw + 6.0), y), count_g, count_color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------- inputs and containers

/// Search field: dark field, border, search icon, clear button.
pub fn search_input(ui: &mut Ui, query: &mut String, placeholder: &str) {
    search_input_width(ui, query, placeholder, ui.available_width().min(320.0));
}
/// `search_input` with an explicit width. Returns whether Enter was pressed in the field.
pub fn search_input_width(ui: &mut Ui, query: &mut String, placeholder: &str, width: f32) -> bool {
    let mut entered = false;
    egui::Frame {
        inner_margin: egui::Margin::symmetric(8, 0),
        outer_margin: egui::Margin::ZERO,
        corner_radius: CornerRadius::same(6),
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
            let field = ui.add(
                egui::TextEdit::singleline(query)
                    .hint_text(egui::RichText::new(placeholder).color(THEME.text_muted).size(12.0))
                    .frame(egui::Frame::NONE)
                    .font(egui::FontId::proportional(12.0))
                    .desired_width((ui.available_width() - clear_w).max(40.0)),
            );
            entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !query.is_empty() && icon_button(ui, ICON_CLOSE, Vec2::new(12.0, 12.0), THEME.text_muted).clicked() {
                query.clear();
            }
        });
    });
    entered
}

/// The soft shadow under raised surfaces (cards, panels): it lifts them off the window.
pub fn surface_shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 3],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(90),
    }
}

/// A raised surface: lighter than the window, a visible edge, a soft shadow and room for it.
pub fn raised_frame(radius: u8) -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::same(12),
        outer_margin: egui::Margin::ZERO,
        corner_radius: CornerRadius::same(radius),
        shadow: surface_shadow(),
        fill: THEME.bg_surface,
        stroke: Stroke::new(1.0_f32, mix(THEME.border_subtle, THEME.border_strong, 0.5)),
    }
}

/// A thin light line along the top edge of a raised surface, like light falling on it.
pub fn paint_top_highlight(ui: &Ui, rect: Rect, radius: u8) {
    let r = radius as f32;
    let y = rect.top() + 1.0;
    ui.painter().hline(
        (rect.left() + r)..=(rect.right() - r),
        y,
        Stroke::new(1.0, Color32::from_white_alpha(14)),
    );
}

/// Card: a raised surface with 8 px corners.
pub fn card_container<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame {
        outer_margin: egui::Margin::symmetric(0, 5),
        ..raised_frame(8)
    };
    let response = frame.show(ui, add_contents);
    let rect = response.response.rect.shrink2(Vec2::new(0.0, 5.0));
    paint_top_highlight(ui, rect, 8);
    response.inner
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

/// A segmented choice: options side by side in one rounded box, the chosen one on an
/// accent-tinted segment. Returns whether the choice changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    let font = egui::FontId::proportional(13.0);
    let pad = Vec2::new(12.0, 5.0);
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap(label.to_string(), font.clone(), THEME.text_primary)
        })
        .collect();
    let height = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max) + pad.y * 2.0;
    let width: f32 = galleys.iter().map(|g| g.size().x + pad.x * 2.0).sum::<f32>() + 6.0;
    let (outer, outer_resp) = ui.allocate_exact_size(Vec2::new(width, height + 6.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(outer, CornerRadius::same(8), THEME.bg_dark);
    painter.rect_stroke(
        outer,
        CornerRadius::same(8),
        Stroke::new(1.0, THEME.border_subtle),
        egui::StrokeKind::Inside,
    );
    let mut x = outer.left() + 3.0;
    for (i, ((option, _), galley)) in options.iter().zip(galleys).enumerate() {
        let w = galley.size().x + pad.x * 2.0;
        let rect = Rect::from_min_size(Pos2::new(x, outer.top() + 3.0), Vec2::new(w, height));
        x += w;
        let response = ui
            .interact(rect, outer_resp.id.with(i), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let selected = *value == *option;
        if selected {
            painter.rect_filled(rect, CornerRadius::same(6), crate::ui::theme::tint(THEME.accent_cyan));
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(6), THEME.bg_surface_hover);
        }
        let color = if selected {
            THEME.accent_cyan
        } else if response.hovered() {
            THEME.text_primary
        } else {
            THEME.text_secondary
        };
        let pos = Pos2::new(
            snap(ui, rect.center().x - galley.size().x / 2.0),
            snap(ui, rect.center().y - galley.size().y / 2.0),
        );
        painter.galley(pos, galley, color);
        if response.clicked() && !selected {
            *value = *option;
            changed = true;
        }
    }
    changed
}

/// An item of a side navigation list: icon and name, a count on the right, the chosen one on
/// an accent-tinted pill with a bar at its left edge. `compact` leaves only the centered icon
/// (name on hover, the count as a dot). An `alert` count is drawn in the accent color.
pub fn side_nav_item(
    ui: &mut Ui,
    icon: ImageSource<'static>,
    label: &str,
    selected: bool,
    compact: bool,
    count: Option<(usize, bool)>,
) -> Response {
    let size = Vec2::new(ui.available_width(), 38.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), crate::ui::theme::tint(THEME.accent_cyan));
            let bar = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 9.0), Vec2::new(3.0, 18.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), THEME.accent_cyan);
        } else if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), THEME.bg_surface_hover);
        }
        let color = if selected {
            THEME.accent_cyan
        } else if hovered {
            THEME.text_primary
        } else {
            THEME.text_secondary
        };
        let icon_x = if compact { rect.center().x } else { rect.left() + 22.0 };
        let icon_rect = Rect::from_center_size(Pos2::new(icon_x, rect.center().y), Vec2::splat(18.0));
        egui::Image::new(icon).tint(color).paint_at(ui, icon_rect);
        if compact {
            if let Some((_, true)) = count {
                ui.painter()
                    .circle_filled(icon_rect.right_top() + Vec2::new(1.0, 1.0), 3.5, THEME.accent_amber);
            }
        } else {
            // The name is cut with "…" before the count (or the right edge), never past it.
            let count_w = count.map_or(0.0, |(n, _)| {
                let w = ui
                    .painter()
                    .layout_no_wrap(n.to_string(), egui::FontId::proportional(11.0), Color32::WHITE)
                    .size()
                    .x;
                (w + 12.0).max(22.0) + 14.0
            });
            let max_w = (rect.right() - 8.0 - count_w - (rect.left() + 42.0)).max(20.0);
            let mut job = egui::text::LayoutJob::simple_singleline(
                label.to_string(),
                egui::FontId::proportional(14.0),
                if selected { THEME.text_primary } else { color },
            );
            job.wrap.max_width = max_w;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(
                Pos2::new(rect.left() + 42.0, rect.center().y - galley.size().y / 2.0),
                galley,
                THEME.text_primary,
            );
            if let Some((n, alert)) = count {
                let (bg, fg) = if alert {
                    (crate::ui::theme::tint(THEME.accent_amber), THEME.accent_amber)
                } else {
                    (THEME.bg_dark, THEME.text_muted)
                };
                let galley = ui
                    .painter()
                    .layout_no_wrap(n.to_string(), egui::FontId::proportional(11.0), fg);
                let pill = Rect::from_center_size(
                    Pos2::new(rect.right() - 10.0 - (galley.size().x + 12.0) / 2.0, rect.center().y),
                    Vec2::new((galley.size().x + 12.0).max(22.0), 18.0),
                );
                ui.painter().rect_filled(pill, CornerRadius::same(9), bg);
                let pos = Pos2::new(
                    snap(ui, pill.center().x - galley.size().x / 2.0),
                    snap(ui, pill.center().y - galley.size().y / 2.0),
                );
                ui.painter().galley(pos, galley, fg);
            }
        }
    }
    // The name on hover too: in the compact bar it's the only place, a long one may be cut.
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// A small muted heading over a group of side navigation items.
pub fn side_nav_group(ui: &mut Ui, text: &str, compact: bool) {
    if compact {
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
    } else {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(text).color(THEME.text_muted).size(11.0).strong());
        });
        ui.add_space(2.0);
    }
}

/// A chip that opens a menu: its name, "· N" when N of its options are chosen, and a "▾".
/// Drawn as chosen while any option is.
pub fn menu_chip(ui: &mut Ui, label: &str, chosen: usize) -> Response {
    let text = if chosen > 0 {
        format!("{label} · {chosen}")
    } else {
        label.to_string()
    };
    let selected = chosen > 0;
    let font = egui::FontId::proportional(12.0);
    let color = if selected {
        THEME.accent_cyan
    } else {
        THEME.text_secondary
    };
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let size = Vec2::new(galley.size().x + 16.0 + 14.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        // A small chevron at the right edge.
        let c = Pos2::new(rect.right() - 13.0, rect.center().y + 1.0);
        let chevron = [
            c + Vec2::new(-3.5, -2.0),
            c + Vec2::new(0.0, 1.5),
            c + Vec2::new(3.5, -2.0),
        ];
        let fill = if selected {
            tint(THEME.accent_cyan)
        } else if response.hovered() {
            THEME.bg_surface_hover
        } else {
            THEME.bg_surface
        };
        let stroke = if selected {
            THEME.border_strong
        } else {
            THEME.border_subtle
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            fill,
            Stroke::new(1.0_f32, stroke),
            egui::StrokeKind::Middle,
        );
        let pos = Pos2::new(
            snap(ui, rect.min.x + 8.0),
            snap(ui, rect.center().y - galley.size().y / 2.0),
        );
        ui.painter().galley(pos, galley, color);
        ui.painter()
            .add(egui::Shape::line(chevron.to_vec(), Stroke::new(1.5, color)));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A path as shown on screen. In the screenshot mode (`RWMM_SCREENSHOT`) the user's own
/// folder becomes `C:\Users\Player`, so a picture for a mod page doesn't show whose PC it is.
pub fn shown_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if std::env::var_os("RWMM_SCREENSHOT").is_none() {
        return text;
    }
    match std::env::var("USERPROFILE") {
        Ok(home) if !home.is_empty() && text.to_lowercase().starts_with(&home.to_lowercase()) => {
            format!(r"C:\Users\Player{}", &text[home.len()..])
        }
        _ => text,
    }
}
