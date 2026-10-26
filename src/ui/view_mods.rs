use eframe::egui::{self, Color32, Rounding, Stroke, Ui, Vec2};
use std::collections::HashSet;

use crate::app::{lookup_mod, passes_category_filter, App};
use crate::categories::ModCategory;
use crate::dependencies::DependencyProblem;
use crate::i18n::{fmt, tr, TranslationKey};
use crate::models::{is_protected_package_id, ModInfo, ModOrigin};
use crate::sorter::ModTier;
use crate::steam::workshop_url;
use super::components::*;
use super::icons::*;
use super::theme::THEME;

pub fn render_mods_view(app: &mut App, ui: &mut Ui) {
    // Any manual change to the active list (enable, disable, move) re-runs the dependency check,
    // so the "not in list" / "order" badges never show stale results.
    let active_before = app.active_mods.clone();
    render_mods_view_inner(app, ui);
    if app.active_mods != active_before {
        app.validate_deps();
    }
}

fn render_mods_view_inner(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    // Toolbar (design: secondary "Auto-Sort", primary "Save Order", amber "● unsaved", library actions, search)
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if secondary_button_icon(ui, ICON_SORT, t.get(TranslationKey::BtnAutoSort)).clicked() {
            app.sort_active_mods();
        }

        if primary_button_icon(ui, ICON_SAVE, t.get(TranslationKey::BtnSaveConfig)).clicked() {
            app.save_config();
        }
        if app.has_unsaved_changes() {
            ui.label(
                egui::RichText::new(crate::i18n::tr!("● не сохранено", "● unsaved"))
                    .color(THEME.accent_amber)
                    .size(12.0),
            )
            .on_hover_text(crate::i18n::tr!(
                "Порядок модов отличается от ModsConfig.xml. Нажмите «Сохранить порядок», чтобы игра его увидела.",
                "The load order differs from ModsConfig.xml. Press \"Save Order\" so the game sees it."
            ));
        }

        let scanning = app.is_rescanning();
        ui.add_enabled_ui(!scanning, |ui| {
            if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::BtnRescanMods)).clicked() {
                app.rescan_mods();
            }
        });
        if scanning {
            ui.spinner();
        }

        if secondary_button_icon(ui, ICON_CHECK_ALL, t.get(TranslationKey::BtnEnableAll)).clicked() {
            app.activate_all_mods();
        }

        if secondary_button_icon(ui, ICON_X_ALL, t.get(TranslationKey::BtnDisableAll)).clicked() {
            app.deactivate_all_mods();
        }

        // Toggle button for Mod Info panel if closed
        if !app.is_details_panel_visible
            && secondary_button_icon(ui, ICON_INFO, app.i18n.get(TranslationKey::ModsBtnInfoPanel)).clicked() {
                app.is_details_panel_visible = true;
                app.is_details_panel_collapsed = false;
            }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let w = ui.available_width().clamp(160.0, 264.0);
            search_input_width(ui, &mut app.mod_search_query, t.get(TranslationKey::SearchPlaceholder), w);
        });
    });

    ui.add_space(8.0);

    // Conflict & Dependency Alert Bar (if any problems detected)
    if !app.dependency_report.is_empty() {
        let missing = app.dependency_report.missing_count();
        let disabled = app.dependency_report.disabled_count();
        let order = app.dependency_report.order_count();
        let incompatible = app.dependency_report.incompatible_count();

        card_container(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_STATUS_WARNING, Vec2::splat(18.0), THEME.accent_amber));
                ui.vertical(|ui| {
                    let mut summary = format!(
                        "{}: {} {}, {} {}, {} {}",
                        t.get(TranslationKey::TabDependencies),
                        missing,
                        t.get(TranslationKey::DepMissing),
                        disabled,
                        t.get(TranslationKey::DepDisabled),
                        order,
                        t.get(TranslationKey::DepWrongOrder)
                    );
                    if incompatible > 0 {
                        summary.push_str(&format!(", {} {}", incompatible, t.get(TranslationKey::DepIncompatible)));
                    }
                    ui.label(
                        egui::RichText::new(summary)
                        .color(THEME.accent_amber)
                        .strong(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, t.get(TranslationKey::BtnAutoFixDependencies)).clicked() {
                        app.auto_fix_dependencies();
                    }
                });
            });
        });
        ui.add_space(4.0);
    }

    // Duplicate copies of the same mod (e.g. Workshop + game Mods folder)
    if !app.duplicate_mods.is_empty() {
        card_container(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_STATUS_DUPLICATE, Vec2::splat(18.0), THEME.accent_amber));
                ui.label(
                    egui::RichText::new(fmt(
                        t.get(TranslationKey::ModsDuplicatesTitle),
                        &[&app.duplicate_mods.len().to_string()],
                    ))
                    .color(THEME.accent_amber)
                    .strong(),
                );
            });
            for dup in &app.duplicate_mods {
                ui.label(
                    egui::RichText::new(format!("{} ({})", dup.name, dup.package_id))
                        .color(THEME.text_primary)
                        .size(12.0),
                );
                for (origin, dir) in &dup.origins {
                    ui.horizontal(|ui| {
                        origin_badge(ui, *origin);
                        if ui
                            .link(egui::RichText::new(dir.display().to_string()).size(12.0))
                            .clicked()
                        {
                            open_external(dir);
                        }
                    });
                }
            }
        });
        ui.add_space(4.0);
    }

    render_version_issues(app, ui);

    render_category_filter(app, ui);
    ui.add_space(12.0);

    // Two panels side by side with a 16 px gap: available mods | active mods (+ details card).
    let gap = 16.0;
    let col_width = ((ui.available_width() - gap) / 2.0).floor().max(280.0);
    let total_height = ui.available_height();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        panel_card(ui, col_width, total_height, |ui| render_inactive_mods_column(app, ui));
        panel_card(ui, col_width, total_height, |ui| {
            // Details card takes exactly its own height at the bottom; the list gets the rest.
            if app.is_details_panel_visible {
                egui::TopBottomPanel::bottom("mod_details_card")
                    .frame(egui::Frame::none().inner_margin(egui::Margin::same(8.0)))
                    .show_separator_line(false)
                    .resizable(false)
                    .show_inside(ui, |ui| render_mod_details_panel(app, ui));
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show_inside(ui, |ui| render_active_mods_column(app, ui));
        });
    });
}

/// Panel card of the design: panel color, subtle border, 6 px corners, fixed size.
fn panel_card(ui: &mut Ui, width: f32, height: f32, add_contents: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
        egui::Frame {
            inner_margin: egui::Margin::ZERO,
            outer_margin: egui::Margin::ZERO,
            rounding: Rounding::same(6.0),
            shadow: egui::epaint::Shadow::NONE,
            fill: THEME.bg_surface,
            stroke: Stroke::new(1.0_f32, THEME.border_subtle),
        }
        .show(ui, |ui| {
            ui.set_width(width - 2.0);
            ui.set_height(height - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui);
        });
    });
}

/// 48 px panel header: uppercase title + muted count on the left, `right` content on the right.
fn panel_header(ui: &mut Ui, title: &str, count: usize, right: impl FnOnce(&mut Ui)) {
    egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 0.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(48.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new(title.to_uppercase()).color(THEME.text_secondary).size(12.0).strong());
            ui.label(egui::RichText::new(count.to_string()).color(THEME.text_muted).size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
        });
    });
}

/// Thin muted line under a list: `left` text and optional `right` text.
fn panel_footer(ui: &mut Ui, top_border: bool, icon: Option<egui::ImageSource<'static>>, left: &str, right: Option<&str>) {
    let rect = ui.available_rect_before_wrap();
    if top_border {
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, THEME.border_subtle));
    }
    egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 0.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(if top_border { 48.0 } else { 28.0 });
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(icon) = icon {
                ui.add(svg_icon(icon, Vec2::splat(16.0), THEME.text_muted));
            }
            ui.label(egui::RichText::new(left).color(THEME.text_muted).size(12.0));
            if let Some(right) = right {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(right).color(THEME.text_muted).size(12.0));
                });
            }
        });
    });
}

/// Mod row frame of the design: 34 px card, the selected one lighter with a lavender bar at the left.
fn mod_row_frame(selected: bool) -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::symmetric(8.0, 5.0),
        outer_margin: egui::Margin::symmetric(0.0, 1.0),
        rounding: Rounding::same(6.0),
        shadow: egui::epaint::Shadow::NONE,
        fill: if selected { THEME.bg_surface_hover } else { THEME.bg_card },
        stroke: Stroke::NONE,
    }
}

fn paint_selection_bar(ui: &Ui, row: egui::Rect) {
    let bar = egui::Rect::from_min_max(
        egui::pos2(row.min.x, row.center().y - 11.0),
        egui::pos2(row.min.x + 3.0, row.center().y + 11.0),
    );
    ui.painter().rect_filled(bar, Rounding::same(1.5), THEME.accent_cyan);
}

/// Mod name in a list row: clickable, takes at most ~60% of the row so badges keep room,
/// and a long name ends with "…" (full name on hover) instead of being cut mid-letter.
fn mod_name_label(ui: &mut Ui, text: egui::RichText) -> egui::Response {
    let max_width = (ui.available_width() * 0.6).max(80.0);
    ui.scope(|ui| {
        ui.set_max_width(max_width);
        ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click()))
    })
    .inner
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Width reserved at the right edge of a row for its buttons (24 px boxes, 4 px gaps, scrollbar room).
const ACTIVE_ROW_BUTTONS_WIDTH: f32 = 3.0 * 24.0 + 2.0 * 4.0 + 8.0 + 4.0;
const INACTIVE_ROW_BUTTONS_WIDTH: f32 = 24.0 + 8.0 + 4.0;

/// Where a row's button slot starts and ends.
struct RowSplit {
    right_x: f32,
    max_x: f32,
}

/// Splits a row: returns a child UI for the content (clipped before the button slot, so a long
/// name or many badges can never run under the buttons) and the position of the slot.
fn split_row(ui: &mut Ui, buttons_width: f32) -> (Ui, RowSplit) {
    let full = ui.available_rect_before_wrap();
    let right_x = (full.max.x - buttons_width).max(full.min.x);
    let left_rect = egui::Rect::from_min_max(full.min, egui::pos2(right_x - 4.0, full.max.y));
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let mut clip = ui.clip_rect();
    clip.max.x = clip.max.x.min(right_x - 4.0);
    left.set_clip_rect(clip);
    (left, RowSplit { right_x, max_x: full.max.x })
}

/// Reserves the content area in the row (never wider than its slot) and returns the rect
/// for the buttons, as tall as the content.
fn finish_row(ui: &mut Ui, left: &Ui, split: &RowSplit) -> egui::Rect {
    let used = left.min_rect();
    let content = egui::Rect::from_min_max(used.min, egui::pos2(split.right_x - 4.0, used.max.y));
    ui.allocate_rect(content, egui::Sense::hover());
    egui::Rect::from_min_max(egui::pos2(split.right_x, used.min.y), egui::pos2(split.max_x, used.max.y))
}

/// Average height of the rows just drawn (without item spacing), for `ScrollArea::show_rows`.
/// Rows get taller with a bigger font, so the height is measured instead of hard-coded.
fn measure_row_height(ui: &Ui, rows_top: f32, rows_count: usize) -> Option<f32> {
    if rows_count == 0 {
        return None;
    }
    let total = ui.cursor().top() - rows_top;
    let per_row = total / rows_count as f32 - ui.spacing().item_spacing.y;
    (per_row > 8.0).then_some(per_row)
}

/// Hard problems of the active list for the installed game (Prepatcher / old Harmony on 1.7)
/// plus a calm one-line summary of mods that don't declare the current version.
fn render_version_issues(app: &App, ui: &mut Ui) {
    let t = app.i18n;
    let game_mm = app.steam_paths.game_major_minor();
    let issues = crate::version_check::active_list_issues(
        &app.active_mods,
        &app.all_mods,
        app.steam_paths.engine,
        game_mm.as_deref(),
    );
    let unsupported = app
        .active_mods
        .iter()
        .filter_map(|id| lookup_mod(&app.mod_index, &app.all_mods, id))
        .filter(|m| crate::version_check::supports_version(m, game_mm.as_deref()) == Some(false))
        .count();

    if !issues.is_empty() {
        card_container(ui, |ui| {
            for issue in &issues {
                let text = match issue {
                    crate::version_check::VersionIssue::PrepatcherOnWaylith => t.get(TranslationKey::VersionPrepatcher),
                    crate::version_check::VersionIssue::HarmonyNotUpdated => t.get(TranslationKey::VersionHarmony),
                };
                ui.horizontal(|ui| {
                    ui.add(svg_icon(ICON_STATUS_ERROR, Vec2::splat(18.0), THEME.accent_rose));
                    ui.label(egui::RichText::new(text).color(THEME.accent_rose).strong());
                });
            }
        });
        ui.add_space(4.0);
    }
    if unsupported > 0 {
        if let Some(mm) = game_mm.as_deref() {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_INFO, Vec2::splat(14.0), THEME.text_muted));
                ui.label(
                    egui::RichText::new(fmt(
                        t.get(TranslationKey::ModsVersionSummary),
                        &[mm, &unsupported.to_string()],
                    ))
                    .color(THEME.text_muted)
                    .size(12.0),
                );
            });
            ui.add_space(2.0);
        }
    }
}

/// Muted "up to 1.6" badge for a mod that doesn't declare the installed game version.
fn version_badge(ui: &mut Ui, m: &ModInfo, game_mm: Option<&str>, t: crate::i18n::I18n) {
    if crate::version_check::supports_version(m, game_mm) != Some(false) {
        return;
    }
    let newest = crate::version_check::newest_supported(m).unwrap_or_default();
    if let Some(resp) = tag_if_fits(ui, &fmt(t.get(TranslationKey::ModsVersionBadge), &[&newest]), THEME.accent_amber) {
        resp.on_hover_text(fmt(t.get(TranslationKey::ModsVersionHint), &[game_mm.unwrap_or("?")]));
    }
}

/// Row of category chips above the lists: "All" + every category that some mod has, with counts.
/// Clicking chips toggles them; a mod is shown if it has any selected category.
fn render_category_filter(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let mut counts: std::collections::HashMap<ModCategory, usize> = std::collections::HashMap::new();
    for m in &app.all_mods {
        for c in &m.categories {
            *counts.entry(*c).or_insert(0) += 1;
        }
    }
    if counts.is_empty() {
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        let all_selected = app.category_filter.is_empty();
        if chip(ui, None, t.get(TranslationKey::CategoriesAll), app.all_mods.len(), all_selected).clicked() {
            app.category_filter.clear();
        }
        for cat in ModCategory::ALL {
            let Some(n) = counts.get(&cat) else {
                continue;
            };
            let selected = app.category_filter.contains(&cat);
            let (_, fg) = cat.colors();
            let fg = Color32::from_rgb(fg[0], fg[1], fg[2]);
            if chip(ui, Some((category_icon(cat), fg)), cat.label(t.language()), *n, selected).clicked() {
                if selected {
                    app.category_filter.remove(&cat);
                } else {
                    app.category_filter.insert(cat);
                }
            }
        }
    });
}

/// Up to `max` category badges (strongest first); the rest are listed in the hover text.
fn category_badges(ui: &mut Ui, cats: &[ModCategory], t: crate::i18n::I18n, max: usize) {
    // In list rows a badge is drawn only if it fits completely; the ones that don't fit
    // go into the "+N" badge (with the full list on hover).
    let mut shown = 0;
    for cat in cats.iter().take(max) {
        let (bg, fg) = cat.colors();
        let drawn = badge_if_fits(
            ui,
            cat.label(t.language()),
            Color32::from_rgb(bg[0], bg[1], bg[2]),
            Color32::from_rgb(fg[0], fg[1], fg[2]),
        );
        if drawn.is_none() {
            break;
        }
        shown += 1;
    }
    if cats.len() > shown {
        let rest: Vec<String> = cats[shown..]
            .iter()
            .map(|c| c.label(t.language()).to_string())
            .collect();
        if let Some(resp) = tag_if_fits(ui, &format!("+{}", cats.len() - shown), THEME.text_muted) {
            resp.on_hover_text(rest.join("\n"));
        }
    }
}

/// "🏷 Categories" submenu in a mod's right-click menu: checkboxes + "detect automatically".
fn category_context_menu(
    ui: &mut Ui,
    t: crate::i18n::I18n,
    cats: &[ModCategory],
    is_manual: bool,
    pkg_id: &str,
    toggle: &mut Option<(String, ModCategory)>,
    reset: &mut Option<String>,
) {
    ui.separator();
    ui.menu_button(t.get(TranslationKey::CategoriesMenu), |ui| {
        for cat in ModCategory::ALL {
            let mut on = cats.contains(&cat);
            let (_, fg) = cat.colors();
            ui.horizontal(|ui| {
                ui.add(svg_icon(category_icon(cat), Vec2::splat(14.0), Color32::from_rgb(fg[0], fg[1], fg[2])));
                if ui.checkbox(&mut on, cat.label(t.language())).changed() {
                    *toggle = Some((pkg_id.to_string(), cat));
                }
            });
        }
        if is_manual {
            ui.separator();
            ui.label(
                egui::RichText::new(t.get(TranslationKey::CategoriesManual))
                    .color(THEME.text_muted)
                    .size(12.0),
            );
            if ui.button(t.get(TranslationKey::CategoriesReset)).clicked() {
                *reset = Some(pkg_id.to_string());
                ui.close_menu();
            }
        }
    });
}

fn apply_category_actions(app: &mut App, toggle: Option<(String, ModCategory)>, reset: Option<String>) {
    if let Some((pkg_id, cat)) = toggle {
        app.toggle_mod_category(&pkg_id, cat);
    }
    if let Some(pkg_id) = reset {
        app.reset_mod_categories(&pkg_id);
    }
}

/// Helper function to draw a distinctive mod icon badge or real image
/// "Steam" (Workshop) or "Local" badge with its icon.
fn origin_badge(ui: &mut Ui, origin: ModOrigin) {
    let (icon, text, color) = match origin {
        ModOrigin::Workshop => (ICON_STATUS_WORKSHOP, "Steam", THEME.accent_info),
        ModOrigin::Local => (ICON_STATUS_LOCAL, "Local", THEME.text_muted),
    };
    icon_badge(ui, icon, text, crate::ui::theme::tint(color), color);
}

/// Icon, background and foreground for a mod without a preview image:
/// shield for protected content, otherwise the icon of its strongest category.
fn mod_fallback_icon(
    is_protected: bool,
    category: Option<ModCategory>,
) -> (egui::ImageSource<'static>, Color32, Color32) {
    if is_protected {
        return (ICON_SHIELD, crate::ui::theme::tint(THEME.accent_amber), THEME.accent_amber);
    }
    match category {
        Some(cat) => {
            let (bg, fg) = cat.colors();
            (
                category_icon(cat),
                Color32::from_rgb(bg[0], bg[1], bg[2]),
                Color32::from_rgb(fg[0], fg[1], fg[2]),
            )
        }
        None => (ICON_MODS, crate::ui::theme::tint(THEME.accent_cyan), THEME.accent_cyan),
    }
}

/// 20 px mod icon: the real preview (rounded corners), or a category icon on its tinted square.
fn render_mod_icon_pill(
    ui: &mut Ui,
    category: Option<ModCategory>,
    is_protected: bool,
    texture: Option<&egui::TextureHandle>,
) {
    if let Some(tex) = texture {
        ui.add(egui::Image::new(tex).rounding(Rounding::same(4.0)).fit_to_exact_size(egui::vec2(20.0, 20.0)));
        return;
    }
    let (icon, bg_col, fg_col) = mod_fallback_icon(is_protected, category);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(6.0), bg_col);
    egui::Image::new(icon)
        .texture_options(egui::TextureOptions::NEAREST)
        .tint(fg_col)
        .paint_at(ui, egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0)));
}

/// Right column: active mods in load order.
fn render_active_mods_column(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let game_mm = app.steam_paths.game_major_minor();

    // Header: title, count; on the right — search and move up / down for the selected mod.
    let active_count = app.active_mods.len();
    let mut move_selected: i32 = 0;
    let search_hint = app.i18n.get(TranslationKey::ModsSearchActive);
    panel_header(ui, t.get(TranslationKey::ActiveModsTitle), active_count, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        search_input_width(ui, &mut app.active_search_query, search_hint, 192.0);
        if boxed_icon_button(ui, ICON_ARROW_DOWN).on_hover_text(t.get(TranslationKey::ModsCtxMoveDown)).clicked() {
            move_selected = 1;
        }
        if boxed_icon_button(ui, ICON_ARROW_UP).on_hover_text(t.get(TranslationKey::ModsCtxMoveUp)).clicked() {
            move_selected = -1;
        }
    });
    if move_selected != 0 {
        // The selection may come from the left list, where IDs keep the case of
        // About.xml, while ModsConfig stores them in lower case: compare ignoring case.
        let selected_pos = app
            .selected_mod_id
            .as_deref()
            .and_then(|sel| app.active_mods.iter().position(|id| id.eq_ignore_ascii_case(sel)));
        if let Some(pos) = selected_pos {
            if move_selected > 0 && pos + 1 < app.active_mods.len() {
                app.active_mods.swap(pos, pos + 1);
            } else if move_selected < 0 && pos > 0 {
                app.active_mods.swap(pos, pos - 1);
            }
        }
    }

    let query_lower = app.active_search_query.trim().to_lowercase();
    let mut move_up_idx = None;
    let mut move_down_idx = None;
    let mut deactivate_idx = None;
    let mut enable_dep_id = None;
    let mut deactivate_selected = false;
    let mut toggle_ignore_id = None;
    let mut cat_toggle: Option<(String, ModCategory)> = None;
    let mut cat_reset: Option<String> = None;

    // Rows that pass the search and category filter (indices into active_mods), computed once.
    let visible_rows: Vec<usize> = app
        .active_mods
        .iter()
        .enumerate()
        .filter(|(_, pkg_id)| {
            let mod_opt = lookup_mod(&app.mod_index, &app.all_mods, pkg_id);
            let display_name = mod_opt.map(|m| m.name.as_str()).unwrap_or(pkg_id.as_str());
            (query_lower.is_empty()
                || display_name.to_lowercase().contains(&query_lower)
                || pkg_id.to_lowercase().contains(&query_lower))
                && passes_category_filter(&app.category_filter, mod_opt)
        })
        .map(|(i, _)| i)
        .collect();
    // Visible IDs for Shift-range selection
    let visible_active_ids: Vec<String> = visible_rows.iter().map(|&i| app.active_mods[i].clone()).collect();

    // First dependency problem of each mod, by lowercase id: one lookup per row (M1).
    let mut dep_by_mod: std::collections::HashMap<String, DependencyProblem> = std::collections::HashMap::new();
    for p in &app.dependency_report.problems {
        dep_by_mod
            .entry(p.dependent_id().to_lowercase())
            .or_insert_with(|| p.clone());
    }

    let mut missing_icons = Vec::new();
    let row_height = app.active_row_height;
    let mut measured_row_height: Option<f32> = None;
    let mut shown_range = 0..0;
    let list_height = (ui.available_height() - 28.0).max(60.0);

    // Only rows inside the visible area are built (M1): big modpacks stay smooth.
    egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 0.0)).show(ui, |ui| {
    egui::ScrollArea::vertical()
        .id_salt("active_mods_scroll_right")
        .auto_shrink([false, false])
        .max_height(list_height)
        .show_rows(ui, row_height, visible_rows.len(), |ui, row_range| {
            let rows_top = ui.cursor().top();
            let rows_count = row_range.len();
            shown_range = row_range.clone();
            for &idx in &visible_rows[row_range] {
                let pkg_id = &app.active_mods[idx];
                let mod_info_opt = lookup_mod(&app.mod_index, &app.all_mods, pkg_id);
                let display_name = mod_info_opt
                    .map(|m| m.name.as_str())
                    .unwrap_or(pkg_id.as_str());

                let is_selected = app.selected_mod_ids.iter().any(|s| s.eq_ignore_ascii_case(pkg_id))
                    || app.selected_mod_id.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(pkg_id));
                let is_protected = is_protected_package_id(pkg_id);

                // Dependency warning for this mod
                let dep_problem = dep_by_mod.get(&pkg_id.to_lowercase());
                let incompatible = matches!(dep_problem, Some(DependencyProblem::Incompatible { .. }));

                let row = mod_row_frame(is_selected).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // Buttons get a fixed slot at the right edge; long names and badges are
                        // clipped before it instead of pushing the buttons over them.
                        let (mut left_ui, split) = split_row(ui, ACTIVE_ROW_BUTTONS_WIDTH);
                        {
                        let ui = &mut left_ui;
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // Index in a fixed-width column, so "9." and "10." rows stay aligned.
                        ui.add_sized(
                            Vec2::new(24.0, 20.0),
                            egui::Label::new(egui::RichText::new(format!("{}.", idx + 1)).color(THEME.text_muted).size(12.0)),
                        );

                        // Mod Icon badge
                        let tex_opt = app.mod_textures.get(pkg_id);
                        if tex_opt.is_none() {
                            missing_icons.push((pkg_id.clone(), mod_info_opt.and_then(|m| m.preview_path.clone())));
                        }
                        let texture = tex_opt.and_then(|t| t.as_ref());
                        render_mod_icon_pill(
                            ui,
                            mod_info_opt.and_then(|m| m.categories.first().copied()),
                            is_protected,
                            texture,
                        );

                        // Mod Name (selectable)
                        let name_text = egui::RichText::new(display_name)
                            .color(if is_protected {
                                THEME.accent_amber
                            } else if incompatible {
                                THEME.accent_rose
                            } else {
                                THEME.text_primary
                            })
                            .size(13.0);
                        let label_resp = mod_name_label(ui, if is_selected { name_text.strong() } else { name_text });

                        // Multi-selection: Ctrl+click, Shift+click, normal click
                        if label_resp.clicked() {
                            let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd);
                            let shift = ui.input(|i| i.modifiers.shift);

                            if ctrl {
                                // Toggle this mod in multi-selection
                                if app.selected_mod_ids.contains(pkg_id) {
                                    app.selected_mod_ids.remove(pkg_id);
                                } else {
                                    app.selected_mod_ids.insert(pkg_id.clone());
                                }
                            } else if shift {
                                // Range select from last_clicked to current
                                if let Some(ref last_id) = app.last_clicked_mod_id.clone() {
                                    let last_pos = visible_active_ids.iter().position(|id| id == last_id);
                                    let cur_pos = visible_active_ids.iter().position(|id| id == pkg_id);
                                    if let (Some(start), Some(end)) = (last_pos, cur_pos) {
                                        let range = if start <= end { start..=end } else { end..=start };
                                        for i in range {
                                            app.selected_mod_ids.insert(visible_active_ids[i].clone());
                                        }
                                    }
                                }
                            } else {
                                // Normal click — select only this mod
                                app.selected_mod_ids.clear();
                                app.selected_mod_ids.insert(pkg_id.clone());
                            }

                            app.selected_mod_id = Some(pkg_id.clone());
                            app.last_clicked_mod_id = Some(pkg_id.clone());
                            app.is_details_panel_visible = true;
                        }

                        // Double-click: deactivate mod
                        if label_resp.double_clicked() && !is_protected {
                            deactivate_idx = Some(idx);
                        }

                        // Right-click context menu
                        let dir_path_ctx = mod_info_opt.map(|m| m.dir_path.clone());
                        let steam_id_ctx = mod_info_opt.and_then(|m| m.steam_item_id.clone());
                        label_resp.context_menu(|ui| {
                            if !is_protected {
                                if ui.button(app.i18n.get(TranslationKey::ModsCtxDeactivate)).clicked() {
                                    deactivate_idx = Some(idx);
                                    ui.close_menu();
                                }
                                // Deactivate all selected
                                if app.selected_mod_ids.len() > 1
                                    && ui.button(app.i18n.get(TranslationKey::ModsCtxDeactivateSelected).replace("{}", &app.selected_mod_ids.len().to_string())).clicked() {
                                        deactivate_selected = true;
                                        ui.close_menu();
                                    }
                                ui.separator();
                            }
                            if idx > 0
                                && ui.button(app.i18n.get(TranslationKey::ModsCtxMoveUp)).clicked() {
                                    move_up_idx = Some(idx);
                                    ui.close_menu();
                                }
                            if idx < app.active_mods.len().saturating_sub(1)
                                && ui.button(app.i18n.get(TranslationKey::ModsCtxMoveDown)).clicked() {
                                    move_down_idx = Some(idx);
                                    ui.close_menu();
                                }
                            ui.separator();
                            if let Some(ref dir) = dir_path_ctx {
                                if ui.button(app.i18n.get(TranslationKey::ModsCtxOpenFolder)).clicked() {
                                    open_external(dir);
                                    ui.close_menu();
                                }
                            }
                            if let Some(ref steam_id) = steam_id_ctx {
                                if ui.button(app.i18n.get(TranslationKey::ModsCtxOpenWorkshop)).clicked() {
                                    open_external(workshop_url(steam_id));
                                    ui.close_menu();
                                }
                            }
                            if !is_protected_package_id(pkg_id) && !pkg_id.to_lowercase().starts_with("ludeon.rimworld") {
                                ui.separator();
                                let is_ignored = app.opt_ignore_list.contains(&pkg_id.to_lowercase());
                                let ignore_text = if is_ignored { app.i18n.get(TranslationKey::ModsCtxOptSkip) } else { app.i18n.get(TranslationKey::ModsCtxOptAllow) };
                                if ui.button(ignore_text).clicked() {
                                    toggle_ignore_id = Some(pkg_id.clone());
                                    ui.close_menu();
                                }
                            }
                            if let Some(m) = mod_info_opt.filter(|m| !m.package_id.to_lowercase().starts_with("ludeon.rimworld")) {
                                category_context_menu(
                                    ui,
                                    t,
                                    &m.categories,
                                    app.category_overrides.contains_key(&m.package_id.to_lowercase()),
                                    &m.package_id,
                                    &mut cat_toggle,
                                    &mut cat_reset,
                                );
                            }
                        });

                        // Badges: Core, categories, version, "not on disk"
                        if is_protected {
                            tag_if_fits(ui, "Core", THEME.accent_amber);
                        }
                        if let Some(m) = mod_info_opt {
                            if !is_protected {
                                category_badges(ui, &m.categories, t, 2);
                            }
                            version_badge(ui, m, game_mm.as_deref(), t);
                        }
                        // Listed in ModsConfig.xml but not found by the scanner: kept on save, not dropped.
                        if mod_info_opt.is_none() {
                            tag_if_fits(ui, app.i18n.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                        }

                        // Dependency problems
                        if let Some(prob) = dep_problem {
                            match prob {
                                DependencyProblem::Disabled { required_id, .. } => {
                                    tag_if_fits(ui, app.i18n.get(TranslationKey::ModsDepNotInList), THEME.accent_amber);
                                    if fits(ui, 60.0)
                                        && small_button(ui, app.i18n.get(TranslationKey::ModsDepEnable), ButtonStyle::Success).clicked()
                                    {
                                        enable_dep_id = Some(required_id.clone());
                                    }
                                }
                                DependencyProblem::Missing { .. } => {
                                    tag_if_fits(ui, app.i18n.get(TranslationKey::ModsDepNotFound), THEME.accent_rose);
                                }
                                DependencyProblem::WrongOrder { .. } => {
                                    tag_if_fits(ui, app.i18n.get(TranslationKey::ModsDepOrder), THEME.accent_amber);
                                }
                                DependencyProblem::Incompatible { other_name, .. } => {
                                    if let Some(resp) = tag_if_fits(ui, app.i18n.get(TranslationKey::ModsDepIncompatible), THEME.accent_rose) {
                                        resp.on_hover_text(other_name.as_str());
                                    }
                                }
                            }
                        }

                        }
                        let right_rect = finish_row(ui, &left_ui, &split);

                        // Right action buttons (deactivate, move)
                        ui.allocate_new_ui(
                            egui::UiBuilder::new()
                                .max_rect(right_rect)
                                .layout(egui::Layout::right_to_left(egui::Align::Center)),
                            |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_space(8.0); // room for the scrollbar
                            // Fixed slots, so ↑ ↓ ✕ stay in the same columns on every row.
                            if is_protected {
                                ui.add_space(24.0 + 4.0);
                            } else if boxed_icon_button(ui, ICON_CLOSE).on_hover_text(t.get(TranslationKey::BtnDeactivate)).clicked() {
                                deactivate_idx = Some(idx);
                            }
                            if idx + 1 >= app.active_mods.len() {
                                ui.add_space(24.0 + 4.0);
                            } else if boxed_icon_button(ui, ICON_ARROW_DOWN).on_hover_text(t.get(TranslationKey::ModsCtxMoveDown)).clicked() {
                                move_down_idx = Some(idx);
                            }
                            if idx == 0 {
                                ui.add_space(24.0);
                            } else if boxed_icon_button(ui, ICON_ARROW_UP).on_hover_text(t.get(TranslationKey::ModsCtxMoveUp)).clicked() {
                                move_up_idx = Some(idx);
                            }
                        });
                    });
                });
                if is_selected {
                    paint_selection_bar(ui, row.response.rect);
                }
            }
            measured_row_height = measure_row_height(ui, rows_top, rows_count);
        });
    });
    if let Some(h) = measured_row_height {
        app.active_row_height = h;
    }

    // Footer: visible range and a reminder of how the order works.
    let showing = if visible_rows.is_empty() {
        tr!("Нет модов", "No mods").to_string()
    } else {
        tr!(
            "Показаны {}–{} из {}",
            "Showing {}–{} of {}",
            shown_range.start + 1,
            shown_range.end.min(visible_rows.len()),
            visible_rows.len()
        )
    };
    panel_footer(ui, false, None, &showing, Some(tr!("Загрузка сверху вниз", "Loads from top to bottom")));

    // Request missing icons
    for (pkg_id, preview_opt) in missing_icons {
        app.mod_textures.insert(pkg_id.clone(), None);
        if let Some(path) = preview_opt {
            let _ = app.icon_tx.send((pkg_id, path));
        }
    }

    if let Some(pkg_id) = toggle_ignore_id {
        app.toggle_optimizer_ignore(&pkg_id);
    }
    apply_category_actions(app, cat_toggle, cat_reset);

    if let Some(req_id) = enable_dep_id {
        app.activate_mod(&req_id);
    }
    if let Some(idx) = move_up_idx {
        if idx > 0 {
            app.active_mods.swap(idx, idx - 1);
        }
    }
    if let Some(idx) = move_down_idx {
        if idx + 1 < app.active_mods.len() {
            app.active_mods.swap(idx, idx + 1);
        }
    }
    // Deactivate all selected mods (from context menu)
    if deactivate_selected {
        // Protected mods (Core) stay active even when they are part of the selection.
        let to_remove: HashSet<String> = app.selected_mod_ids.iter().map(|s| s.to_lowercase()).collect();
        app.active_mods
            .retain(|id| is_protected_package_id(id) || !to_remove.contains(&id.to_lowercase()));
        app.selected_mod_ids.clear();
    } else if let Some(idx) = deactivate_idx {
        app.active_mods.remove(idx);
    }
}

/// Renders the Left Column: Inactive / Available Mods
fn render_inactive_mods_column(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let game_mm = app.steam_paths.game_major_minor();
    // "x_steam" in ModsConfig refers to the Workshop copy of "x": it is active too.
    let active_set: HashSet<String> = app
        .active_mods
        .iter()
        .map(|s| {
            let lower = s.to_lowercase();
            lower.strip_suffix("_steam").map(str::to_string).unwrap_or(lower)
        })
        .collect();

    let inactive_mods: Vec<&ModInfo> = app
        .all_mods
        .iter()
        .filter(|m| !active_set.contains(&m.package_id.to_lowercase()))
        .collect();

    let total_text = app.i18n.get(TranslationKey::ModsTotal).replace("{}", &app.all_mods.len().to_string());
    panel_header(ui, t.get(TranslationKey::AvailableModsTitle), inactive_mods.len(), |ui| {
        ui.label(egui::RichText::new(total_text).color(THEME.text_muted).size(12.0));
    });

    let query_lower = app.mod_search_query.trim().to_lowercase();
    let mut activate_id = None;
    let mut activate_selected = false;
    let mut toggle_ignore_id = None;
    let mut cat_toggle: Option<(String, ModCategory)> = None;
    let mut cat_reset: Option<String> = None;

    // Rows that pass the search and category filter, computed once.
    let visible_mods: Vec<&ModInfo> = inactive_mods
        .iter()
        .copied()
        .filter(|m| {
            (query_lower.is_empty()
                || m.name_lower.contains(&query_lower)
                || m.package_id.to_lowercase().contains(&query_lower)
                || m.author.to_lowercase().contains(&query_lower))
                && passes_category_filter(&app.category_filter, Some(m))
        })
        .collect();
    // Visible IDs for Shift-range selection
    let visible_inactive_ids: Vec<String> = visible_mods.iter().map(|m| m.package_id.clone()).collect();

    let mut missing_icons = Vec::new();
    let row_height = app.inactive_row_height;
    let mut measured_row_height: Option<f32> = None;
    let list_height = (ui.available_height() - 48.0).max(60.0);

    // Only rows inside the visible area are built (M1).
    egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 0.0)).show(ui, |ui| {
    egui::ScrollArea::vertical()
        .id_salt("inactive_mods_scroll_left")
        .auto_shrink([false, false])
        .max_height(list_height)
        .min_scrolled_height(list_height)
        .show_rows(ui, row_height, visible_mods.len(), |ui, row_range| {
            let rows_top = ui.cursor().top();
            let rows_count = row_range.len();
            for m in visible_mods.iter().skip(row_range.start).take(rows_count) {

                let is_selected = app.selected_mod_ids.contains(&m.package_id) || app.selected_mod_id.as_deref() == Some(&m.package_id);

                let row = mod_row_frame(is_selected).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // Fixed slot for the "+" button at the right edge (see the active list).
                        let (mut left_ui, split) = split_row(ui, INACTIVE_ROW_BUTTONS_WIDTH);
                        {
                        let ui = &mut left_ui;
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // Mod Icon badge
                        let tex_opt = app.mod_textures.get(&m.package_id);
                        if tex_opt.is_none() {
                            missing_icons.push((m.package_id.clone(), m.preview_path.clone()));
                        }
                        let texture = tex_opt.and_then(|t| t.as_ref());
                        render_mod_icon_pill(ui, m.categories.first().copied(), false, texture);

                        let name_text = egui::RichText::new(&m.name).color(THEME.text_primary).size(13.0);
                        let label_resp = mod_name_label(ui, if is_selected { name_text.strong() } else { name_text });

                        // Multi-selection: Ctrl+click, Shift+click, normal click
                        if label_resp.clicked() {
                            let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd);
                            let shift = ui.input(|i| i.modifiers.shift);

                            if ctrl {
                                if app.selected_mod_ids.contains(&m.package_id) {
                                    app.selected_mod_ids.remove(&m.package_id);
                                } else {
                                    app.selected_mod_ids.insert(m.package_id.clone());
                                }
                            } else if shift {
                                if let Some(ref last_id) = app.last_clicked_mod_id.clone() {
                                    let last_pos = visible_inactive_ids.iter().position(|id| id == last_id);
                                    let cur_pos = visible_inactive_ids.iter().position(|id| id == &m.package_id);
                                    if let (Some(start), Some(end)) = (last_pos, cur_pos) {
                                        let range = if start <= end { start..=end } else { end..=start };
                                        for i in range {
                                            app.selected_mod_ids.insert(visible_inactive_ids[i].clone());
                                        }
                                    }
                                }
                            } else {
                                app.selected_mod_ids.clear();
                                app.selected_mod_ids.insert(m.package_id.clone());
                            }

                            app.selected_mod_id = Some(m.package_id.clone());
                            app.last_clicked_mod_id = Some(m.package_id.clone());
                            app.is_details_panel_visible = true;
                        }

                        // Double-click: activate mod
                        if label_resp.double_clicked() {
                            activate_id = Some(m.package_id.clone());
                        }

                        // Right-click context menu
                        let dir_path_ctx = m.dir_path.clone();
                        let steam_id_ctx = m.steam_item_id.clone();
                        let pkg_id_ctx = m.package_id.clone();
                        label_resp.context_menu(|ui| {
                            if ui.button(app.i18n.get(TranslationKey::ModsCtxActivate)).clicked() {
                                activate_id = Some(pkg_id_ctx.clone());
                                ui.close_menu();
                            }
                            if app.selected_mod_ids.len() > 1
                                && ui.button(app.i18n.get(TranslationKey::ModsCtxActivateSelected).replace("{}", &app.selected_mod_ids.len().to_string())).clicked() {
                                    activate_selected = true;
                                    ui.close_menu();
                                }
                            ui.separator();
                            if ui.button(app.i18n.get(TranslationKey::ModsCtxOpenFolder)).clicked() {
                                open_external(&dir_path_ctx);
                                ui.close_menu();
                            }
                            if let Some(ref steam_id) = steam_id_ctx {
                                if ui.button(app.i18n.get(TranslationKey::ModsCtxOpenWorkshop)).clicked() {
                                    open_external(workshop_url(steam_id));
                                    ui.close_menu();
                                }
                            }
                            
                            if !is_protected_package_id(&pkg_id_ctx) && !pkg_id_ctx.to_lowercase().starts_with("ludeon.rimworld") {
                                ui.separator();
                                let is_ignored = app.opt_ignore_list.contains(&pkg_id_ctx.to_lowercase());
                                let ignore_text = if is_ignored { app.i18n.get(TranslationKey::ModsCtxOptSkip) } else { app.i18n.get(TranslationKey::ModsCtxOptAllow) };
                                if ui.button(ignore_text).clicked() {
                                    toggle_ignore_id = Some(pkg_id_ctx.clone());
                                    ui.close_menu();
                                }
                            }
                            if !pkg_id_ctx.to_lowercase().starts_with("ludeon.rimworld") {
                                category_context_menu(
                                    ui,
                                    t,
                                    &m.categories,
                                    app.category_overrides.contains_key(&pkg_id_ctx.to_lowercase()),
                                    &pkg_id_ctx,
                                    &mut cat_toggle,
                                    &mut cat_reset,
                                );
                            }
                        });

                        category_badges(ui, &m.categories, t, 2);
                        version_badge(ui, m, game_mm.as_deref(), t);
                        if m.origin == ModOrigin::Workshop {
                            tag_if_fits(ui, "Steam", THEME.accent_info);
                        }
                        }
                        let right_rect = finish_row(ui, &left_ui, &split);

                        ui.allocate_new_ui(
                            egui::UiBuilder::new()
                                .max_rect(right_rect)
                                .layout(egui::Layout::right_to_left(egui::Align::Center)),
                            |ui| {
                            ui.add_space(8.0); // room for the scrollbar
                            if boxed_icon_button(ui, ICON_PLUS).on_hover_text(t.get(TranslationKey::BtnActivate)).clicked() {
                                activate_id = Some(m.package_id.clone());
                            }
                        });
                    });
                });
                if is_selected {
                    paint_selection_bar(ui, row.response.rect);
                }
            }
            measured_row_height = measure_row_height(ui, rows_top, rows_count);
        });
    });
    if let Some(h) = measured_row_height {
        app.inactive_row_height = h;
    }

    panel_footer(
        ui,
        true,
        Some(ICON_PLUS),
        tr!(
            "Нажмите +, чтобы добавить мод в порядок загрузки.",
            "Use + to add a mod to the active load order."
        ),
        None,
    );

    for (pkg_id, preview_opt) in missing_icons {
        app.mod_textures.insert(pkg_id.clone(), None);
        if let Some(path) = preview_opt {
            let _ = app.icon_tx.send((pkg_id, path));
        }
    }

    if let Some(pkg_id) = toggle_ignore_id {
        app.toggle_optimizer_ignore(&pkg_id);
    }
    apply_category_actions(app, cat_toggle, cat_reset);

    // Activate all selected (from context menu)
    if activate_selected {
        let ids: Vec<String> = app.selected_mod_ids.drain().collect();
        for id in ids {
            app.activate_mod(&id);
        }
    } else if let Some(pkg_id) = activate_id {
        app.activate_mod(&pkg_id);
    }
}

/// Mod details panel at the bottom of the right column.
fn render_mod_details_panel(app: &mut App, ui: &mut Ui) {
    let selected_opt = app.selected_mod_id.as_ref().and_then(|sel_id| {
        app.all_mods.iter().find(|m| m.package_id.eq_ignore_ascii_case(sel_id)).cloned()
    });

    let frame = egui::Frame {
        inner_margin: egui::Margin::same(12.0),
        outer_margin: egui::Margin::ZERO,
        rounding: Rounding::same(6.0),
        fill: THEME.bg_dark,
        stroke: Stroke::new(1.0_f32, THEME.border_subtle),
        shadow: egui::epaint::Shadow::NONE,
    };

    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        // Header: icon + title (click collapses), close button on the right
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let title = if selected_opt.is_some() {
                app.i18n.get(TranslationKey::ModsDetailsTitle)
            } else {
                app.i18n.get(TranslationKey::ModsSummaryHeader)
            };
            ui.add(svg_icon(
                if selected_opt.is_some() { ICON_INFO } else { ICON_CHECK_ALL },
                Vec2::splat(16.0),
                THEME.accent_cyan,
            ));
            let title_resp = ui
                .add(
                    egui::Label::new(egui::RichText::new(title).color(THEME.text_primary).size(13.0).strong())
                        .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if title_resp.clicked() {
                app.is_details_panel_collapsed = !app.is_details_panel_collapsed;
            }

            // Close button [✕] to hide panel
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, ICON_CLOSE, Vec2::new(14.0, 14.0), THEME.text_muted).on_hover_text(app.i18n.get(TranslationKey::ModsHideInfoPanel)).clicked() {
                    app.is_details_panel_visible = false;
                }
            });
        });

        // If collapsed: only show one-line summary
        if app.is_details_panel_collapsed {
            return;
        }

        ui.add_space(4.0);

        if let Some(m) = selected_opt {
            let is_protected = is_protected_package_id(&m.package_id);

            ui.horizontal(|ui| {
                // Large Preview Image
                let tex_opt = app.mod_textures.get(&m.package_id);
                if tex_opt.is_none() {
                    if let Some(ref path) = m.preview_path {
                        let _ = app.icon_tx.send((m.package_id.clone(), path.clone()));
                        app.mod_textures.insert(m.package_id.clone(), None);
                    }
                }
                
                if let Some(Some(tex)) = app.mod_textures.get(&m.package_id) {
                    // Show actual image
                    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(64.0, 64.0)));
                } else {
                    // Fallback block if no image or loading
                    let (icon, bg_col, fg_col) =
                        mod_fallback_icon(is_protected, m.categories.first().copied());

                    let frame = egui::Frame {
                        inner_margin: egui::Margin::ZERO,
                        outer_margin: egui::Margin::ZERO,
                        rounding: Rounding::same(6.0),
                        fill: bg_col,
                        stroke: Stroke::new(1.0_f32, fg_col.linear_multiply(0.4)),
                        shadow: egui::epaint::Shadow::NONE,
                    };
                    frame.show(ui, |ui| {
                        ui.allocate_ui(egui::vec2(64.0, 64.0), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.add(svg_icon(icon, Vec2::splat(32.0), fg_col));
                            });
                        });
                    });
                }

                // Vertical column for text details
                ui.vertical(|ui| {
                    // Mod Title + Badges
                    ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&m.name)
                        .color(THEME.text_primary)
                        .size(14.0)
                        .strong(),
                );

                if is_protected {
                    icon_badge(ui, ICON_SHIELD, app.i18n.get(TranslationKey::ModsBadgeProtected), Color32::from_rgb(107, 33, 168).linear_multiply(0.4), Color32::from_rgb(192, 132, 252));
                }
                origin_badge(ui, m.origin);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(ref steam_id) = m.steam_item_id {
                        if ui.small_button("Steam").clicked() {
                            open_external(workshop_url(steam_id));
                        }
                    }
                    if secondary_button_icon(ui, ICON_FOLDER, app.i18n.get(TranslationKey::ModsBtnFolder)).clicked() {
                        open_external(&m.dir_path);
                    }
                });
            });

            // All categories of the mod
            if !m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
                ui.horizontal_wrapped(|ui| {
                    if m.categories.is_empty() {
                        ui.label(
                            egui::RichText::new(app.i18n.get(TranslationKey::CategoriesNone))
                                .color(THEME.text_muted)
                                .size(12.0),
                        );
                    } else {
                        category_badges(ui, &m.categories, app.i18n, usize::MAX);
                    }
                    if app.category_overrides.contains_key(&m.package_id.to_lowercase()) {
                        ui.label(
                            egui::RichText::new(format!("({})", app.i18n.get(TranslationKey::CategoriesManual)))
                                .color(THEME.text_muted)
                                .size(12.0),
                        );
                    }
                });
            }

            // Package ID & Author
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Package ID: {}", m.package_id))
                        .color(Color32::from_rgb(245, 158, 11))
                        .monospace()
                        .size(12.0),
                );

                if !m.author.is_empty() {
                    ui.label(
                        egui::RichText::new(app.i18n.get(TranslationKey::ModsAuthor).replace("{}", &m.author))
                            .color(THEME.text_muted)
                            .size(12.0),
                    );
                }
            });

            // Dependencies list
            if !m.dependencies.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(app.i18n.get(TranslationKey::ModsDependencies)).color(THEME.text_muted).size(12.0));
                    for dep in &m.dependencies {
                        badge(ui, dep, THEME.bg_dark, THEME.text_secondary);
                    }
                });
            }

            // Description box
            if !m.description.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("mod_desc_scroll")
                    .max_height(60.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&m.description)
                                .color(THEME.text_secondary)
                                .size(12.0),
                        );
                    });
            }
                }); // close vertical
            }); // close horizontal
        } else {
            // No mod selected: counts + reminder (design "Mod list summary")
            let available = app.all_mods.len().saturating_sub(app.active_mods.len());
            let incompatible = app.dependency_report.incompatible_count();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.label(
                    egui::RichText::new(tr!("Активно: {}", "{} active", app.active_mods.len()))
                        .color(THEME.text_secondary)
                        .size(12.0),
                );
                ui.label(egui::RichText::new(tr!("Доступно: {}", "{} available", available)).color(THEME.text_secondary).size(12.0));
                if incompatible > 0 {
                    ui.label(
                        egui::RichText::new(tr!("Несовместимых: {}", "{} incompatible", incompatible))
                            .color(THEME.accent_rose)
                            .size(12.0),
                    );
                }
            });
            ui.label(
                egui::RichText::new(tr!(
                    "Проверьте совместимость и сохраните порядок перед игрой. Нажмите на мод, чтобы увидеть подробности.",
                    "Review compatibility and save your order before playing. Click a mod for details."
                ))
                .color(THEME.text_muted)
                .size(12.0),
            );
        }
    });
}

#[allow(dead_code)]
fn render_tier_badge(ui: &mut Ui, tier: ModTier) {
    let (label, bg, fg) = match tier {
        ModTier::Tier0Prepatcher => ("T0 Prepatcher", Color32::from_rgb(88, 28, 135), Color32::from_rgb(216, 180, 254)),
        ModTier::Tier1CoreOfficial => ("T1 Core/DLC", Color32::from_rgb(180, 83, 9), Color32::from_rgb(253, 230, 138)),
        ModTier::Tier2Frameworks => ("T2 Framework", Color32::from_rgb(30, 58, 138), Color32::from_rgb(191, 219, 254)),
        ModTier::Tier3Overhauls => ("T3 Overhaul", Color32::from_rgb(127, 29, 29), Color32::from_rgb(254, 202, 202)),
        ModTier::Tier4FactionsRaces => ("T4 Faction", Color32::from_rgb(19, 78, 74), Color32::from_rgb(153, 246, 228)),
        ModTier::Tier5ContentItems => ("T5 Items", Color32::from_rgb(31, 41, 55), Color32::from_rgb(209, 213, 219)),
        ModTier::Tier6BiomesAnimals => ("T6 Biomes", Color32::from_rgb(20, 83, 45), Color32::from_rgb(187, 247, 208)),
        ModTier::Tier7UiCamera => ("T7 UI", Color32::from_rgb(67, 56, 202), Color32::from_rgb(224, 231, 255)),
        ModTier::Tier8QolBehaviors => ("T8 QoL", Color32::from_rgb(133, 77, 14), Color32::from_rgb(254, 240, 138)),
        ModTier::Tier9Patches => ("T9 Patch", Color32::from_rgb(76, 29, 149), Color32::from_rgb(233, 213, 255)),
        ModTier::Tier10Translations => ("T10 Trans", Color32::from_rgb(131, 24, 67), Color32::from_rgb(251, 207, 232)),
        ModTier::Tier11Optimizers => ("T11 Opt", Color32::from_rgb(6, 78, 59), Color32::from_rgb(167, 243, 208)),
    };
    badge(ui, label, bg, fg);
}
