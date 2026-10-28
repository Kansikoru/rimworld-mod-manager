use eframe::egui::{self, Color32, Rounding, Stroke, Ui, Vec2};
use std::collections::HashSet;

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::{is_official_content, lookup_mod, passes_category_filter, App, DropTarget, ModDrag};
use crate::categories::ModCategory;
use crate::dependencies::DependencyProblem;
use crate::i18n::{fmt, tr, TranslationKey};
use crate::models::{is_protected_package_id, ModInfo, ModOrigin};
use crate::sorter::ModTier;
use crate::steam::workshop_url;

pub fn render_mods_view(app: &mut App, ui: &mut Ui) {
    // Any manual change to the active list (enable, disable, move) re-runs the dependency check,
    // so the "not in list" / "order" badges never show stale results.
    let active_before = app.active_mods.clone();
    // The lists set the drop target again every frame while something is dragged.
    app.drop_target = None;
    render_mods_view_inner(app, ui);
    handle_mod_drag(app, ui.ctx());
    if app.active_mods != active_before {
        app.validate_deps();
    }
}

/// Drag and drop of mods (F1): the label at the cursor, Esc to cancel, drop on release.
fn handle_mod_drag(app: &mut App, ctx: &egui::Context) {
    if app.mod_drag.is_none() {
        return;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.mod_drag = None;
        app.drop_target = None;
        return;
    }
    if ctx.input(|i| !i.pointer.primary_down()) {
        app.finish_mod_drag();
        return;
    }
    let Some(drag) = app.mod_drag.as_ref() else {
        return;
    };

    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    if let Some(pos) = ctx.pointer_interact_pos() {
        let hint = match app.drop_target {
            Some(DropTarget::Available) => Some(tr!("отпустите, чтобы отключить", "release to disable")),
            Some(DropTarget::Active(_)) if !drag.from_active => {
                Some(tr!("отпустите, чтобы включить сюда", "release to enable here"))
            }
            None => Some(tr!("Esc — отмена", "Esc to cancel")),
            _ => None,
        };
        egui::Area::new(egui::Id::new("mod_drag_label"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + Vec2::new(16.0, 12.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame {
                    inner_margin: egui::Margin::symmetric(10.0, 6.0),
                    rounding: Rounding::same(6.0),
                    fill: THEME.bg_surface_hover,
                    stroke: Stroke::new(1.0_f32, THEME.accent_cyan),
                    ..Default::default()
                }
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&drag.label)
                            .color(THEME.text_primary)
                            .size(13.0)
                            .strong(),
                    );
                    if let Some(hint) = hint {
                        ui.label(egui::RichText::new(hint).color(THEME.text_muted).size(11.0));
                    }
                });
            });
    }
    ctx.request_repaint();
}

/// Label that follows the cursor: "Harmony", or "Harmony +2" for a group.
fn drag_label(first_name: &str, count: usize) -> String {
    if count > 1 {
        format!("{first_name} +{}", count - 1)
    } else {
        first_name.to_string()
    }
}

/// Invisible drag handle over a whole row. Registered before the row content, so the name
/// (click, double click, menu) and the buttons stay on top and keep working.
fn row_drag_handle(ui: &mut Ui, id: egui::Id, row_height: f32) -> egui::Response {
    let rect = egui::Rect::from_min_size(
        egui::pos2(ui.max_rect().left(), ui.cursor().top()),
        Vec2::new(ui.available_width(), row_height),
    );
    ui.interact(rect, id, egui::Sense::drag())
}

/// Dragged rows are drawn faded, so it is clear what is being moved.
fn fade_row(ui: &Ui, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, Rounding::same(6.0), THEME.bg_surface.gamma_multiply(0.65));
}

fn render_mods_view_inner(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    // Toolbar: secondary "Auto-Sort", primary "Save Order", amber "● unsaved", library actions, search.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        const GAP: f32 = 8.0;
        const SEARCH_MIN: f32 = 160.0;
        ui.spacing_mut().item_spacing.x = GAP;

        // What fits in a narrow window: first the library buttons lose their labels
        // (the label stays as a tooltip), then the search field becomes a magnifier button.
        let unsaved = app.has_unsaved_changes();
        let unsaved_text = crate::i18n::tr!("● не сохранено", "● unsaved");
        let scanning = app.is_rescanning();
        let mut library: Vec<(egui::ImageSource<'static>, &str)> = vec![
            (ICON_REFRESH, t.get(TranslationKey::BtnRescanMods)),
            (ICON_CHECK_ALL, t.get(TranslationKey::BtnEnableAll)),
            (ICON_X_ALL, t.get(TranslationKey::BtnDisableAll)),
            (ICON_STEAM, t.get(TranslationKey::UpdatesBtn)),
        ];
        let checking_updates = app.checking_mod_updates();
        if !app.is_details_panel_visible {
            library.push((ICON_INFO, t.get(TranslationKey::ModsBtnInfoPanel)));
        }
        let mut fixed = styled_button_width(ui, true, t.get(TranslationKey::BtnAutoSort))
            + styled_button_width(ui, true, t.get(TranslationKey::BtnSaveConfig))
            + 2.0 * GAP;
        if unsaved {
            fixed += ui
                .painter()
                .layout_no_wrap(
                    unsaved_text.to_string(),
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                )
                .size()
                .x
                + GAP;
        }
        if scanning {
            fixed += 16.0 + GAP;
        }
        let full: f32 = library
            .iter()
            .map(|(_, text)| styled_button_width(ui, true, text) + GAP)
            .sum();
        let icons_only = library.len() as f32 * (styled_button_width(ui, true, "") + GAP);
        let avail = ui.available_width();
        let compact = fixed + full + SEARCH_MIN > avail;
        let used = fixed + if compact { icons_only } else { full };
        let search_as_button = avail - used < SEARCH_MIN;
        // Narrowest: Auto-Sort and Save Order become icons too (their names on hover).
        let main_icons = compact && used + styled_button_width(ui, true, "") > avail;
        let sort_label = t.get(TranslationKey::BtnAutoSort);
        let save_label = t.get(TranslationKey::BtnSaveConfig);

        let sort = secondary_button_icon(ui, ICON_SORT, if main_icons { "" } else { sort_label });
        if if main_icons {
            sort.on_hover_text(sort_label)
        } else {
            sort
        }
        .clicked()
        {
            app.sort_active_mods();
        }

        let save = primary_button_icon(ui, ICON_SAVE, if main_icons { "" } else { save_label });
        if if main_icons {
            save.on_hover_text(save_label)
        } else {
            save
        }
        .clicked()
        {
            app.save_config();
        }
        if unsaved {
            ui.label(egui::RichText::new(unsaved_text).color(THEME.accent_amber).size(12.0))
                .on_hover_text(crate::i18n::tr!(
                    "Порядок модов отличается от ModsConfig.xml. Нажмите «Сохранить порядок», чтобы игра его увидела.",
                    "The load order differs from ModsConfig.xml. Press \"Save Order\" so the game sees it."
                ));
        }

        for (i, (icon, text)) in library.iter().enumerate() {
            let label = if compact { "" } else { *text };
            let busy = (i == 0 && scanning) || (i == 3 && checking_updates);
            let clicked = ui
                .add_enabled_ui(!busy, |ui| {
                    let resp = secondary_button_icon(ui, icon.clone(), label);
                    let hint = if i == 3 {
                        t.get(TranslationKey::UpdatesBtnHint)
                    } else {
                        *text
                    };
                    if compact || i == 3 {
                        resp.on_hover_text(hint)
                    } else {
                        resp
                    }
                    .clicked()
                })
                .inner;
            if busy {
                ui.spinner();
            }
            if clicked {
                match i {
                    0 => app.rescan_mods(),
                    1 => app.activate_all_mods(),
                    2 => app.deactivate_all_mods(),
                    3 => app.check_mod_updates(),
                    _ => {
                        app.is_details_panel_visible = true;
                        app.is_details_panel_collapsed = false;
                    }
                }
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let placeholder = t.get(TranslationKey::SearchPlaceholder);
            if !search_as_button {
                let w = ui.available_width().clamp(SEARCH_MIN, 264.0);
                search_input_width(ui, &mut app.mod_search_query, placeholder, w);
                return;
            }
            // Narrow window: a magnifier opens the search field below it; lit while a search is active.
            let popup_id = ui.make_persistent_id("mods_search_popup");
            let style = if app.mod_search_query.trim().is_empty() {
                ButtonStyle::Secondary
            } else {
                ButtonStyle::Primary
            };
            let resp = styled_button(ui, Some(ICON_SEARCH), "", style).on_hover_text(placeholder);
            if resp.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup_id));
            }
            egui::popup_below_widget(
                ui,
                popup_id,
                &resp,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_min_width(240.0);
                    search_input_width(ui, &mut app.mod_search_query, placeholder, 240.0);
                },
            );
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
                        summary.push_str(&format!(
                            ", {} {}",
                            incompatible,
                            t.get(TranslationKey::DepIncompatible)
                        ));
                    }
                    ui.label(egui::RichText::new(summary).color(THEME.accent_amber).strong());
                });
                // Auto-fix only when it has something to do: incompatible mods are the
                // player's choice (buttons below).
                if app.dependency_report.auto_fixable() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if secondary_button(ui, t.get(TranslationKey::BtnAutoFixDependencies)).clicked() {
                            app.auto_fix_dependencies();
                        }
                    });
                }
            });
            // Each incompatible pair, with a button to turn off either mod.
            let mut turn_off: Option<String> = None;
            for ((a_id, a_name), (b_id, b_name)) in app.dependency_report.incompatible_pairs().into_iter().take(5) {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                    ui.label(
                        egui::RichText::new(fmt(t.get(TranslationKey::DepIncompatiblePair), &[&a_name, &b_name]))
                            .color(THEME.text_secondary)
                            .size(12.0),
                    );
                    for (id, name) in [(&a_id, &a_name), (&b_id, &b_name)] {
                        if small_button(
                            ui,
                            &fmt(t.get(TranslationKey::DepTurnOff), &[&short_name(name, 28)]),
                            ButtonStyle::Secondary,
                        )
                        .on_hover_text(id.as_str())
                        .clicked()
                        {
                            turn_off = Some(id.clone());
                        }
                    }
                });
            }
            if let Some(id) = turn_off {
                app.deactivate_mod(&id);
            }
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

    // Narrow window: one list at a time, with a switch between them; the "Filters" button
    // shares its row.
    if ui.available_width() < NARROW_MODS {
        let active: std::collections::HashSet<String> = app.active_mods.iter().map(|s| s.to_lowercase()).collect();
        let available = app
            .all_mods
            .iter()
            .filter(|m| !active.contains(&m.package_id.to_lowercase()))
            .count();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let active_count = app.active_mods.len();
            if chip(
                ui,
                None,
                t.get(TranslationKey::ModsShowAvailable),
                available,
                !app.mods_show_active,
            )
            .clicked()
            {
                app.mods_show_active = false;
            }
            if chip(
                ui,
                None,
                t.get(TranslationKey::ModsShowActive),
                active_count,
                app.mods_show_active,
            )
            .clicked()
            {
                app.mods_show_active = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                filters_toggle(app, ui)
            });
        });
        ui.add_space(6.0);
        render_category_filter(app, ui, true);
        ui.add_space(6.0);
        let width = ui.available_width();
        let height = ui.available_height();
        if app.mods_show_active {
            // A short window gives the space to the list; the details card opens from
            // the toolbar's info button.
            let details_fit = height > 420.0;
            panel_card(ui, width, height, |ui| render_active_panel(app, ui, details_fit));
        } else {
            panel_card(ui, width, height, |ui| render_inactive_mods_column(app, ui));
        }
        return;
    }

    render_category_filter(app, ui, false);
    ui.add_space(12.0);

    // Two panels side by side with a 16 px gap: available mods | active mods (+ details card).
    let gap = 16.0;
    let col_width = ((ui.available_width() - gap) / 2.0).floor().max(280.0);
    let total_height = ui.available_height();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        panel_card(ui, col_width, total_height, |ui| render_inactive_mods_column(app, ui));
        panel_card(ui, col_width, total_height, |ui| render_active_panel(app, ui, true));
    });
}

/// Below this width the Mods tab shows one list at a time (two lists with their headers,
/// badges and buttons need about 470 px each).
const NARROW_MODS: f32 = 940.0;
/// Below this width the category chips fold behind a "Filters" button.
const NARROW_FILTERS: f32 = 900.0;

/// The active list with the details card at its bottom. In a short window (`details_fit`
/// false) the card gets at most 40% of the height and scrolls, so the list stays usable.
fn render_active_panel(app: &mut App, ui: &mut Ui, details_fit: bool) {
    // Details card takes exactly its own height at the bottom; the list gets the rest.
    // In a short window the summary (no mod selected) isn't worth the space.
    let show_card = app.is_details_panel_visible && (details_fit || app.selected_mod_id.is_some());
    if show_card {
        let panel = egui::TopBottomPanel::bottom("mod_details_card")
            .frame(egui::Frame::none().inner_margin(egui::Margin::same(8.0)))
            .show_separator_line(false)
            .resizable(false);
        if details_fit {
            panel.show_inside(ui, |ui| render_mod_details_panel(app, ui));
        } else {
            let max_h = (ui.available_height() * 0.4).max(90.0);
            panel.max_height(max_h).show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("mod_details_scroll")
                    // Leave the panel's own margins out, so the card never covers the list.
                    .max_height(max_h - 24.0)
                    .show(ui, |ui| render_mod_details_panel(app, ui));
            });
        }
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::none())
        .show_inside(ui, |ui| render_active_mods_column(app, ui));
}

/// Panel card: panel color, subtle border, 6 px corners, fixed size.
fn panel_card(ui: &mut Ui, width: f32, height: f32, add_contents: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
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
        },
    );
}

/// 48 px panel header: uppercase title + muted count on the left, `right` content on the right.
fn panel_header(ui: &mut Ui, title: &str, count: usize, right: impl FnOnce(&mut Ui)) {
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(16.0, 0.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(48.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    egui::RichText::new(title.to_uppercase())
                        .color(THEME.text_secondary)
                        .size(12.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(count.to_string())
                        .color(THEME.text_muted)
                        .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
            });
        });
}

/// Thin muted line under a list: `left` text and optional `right` text.
fn panel_footer(
    ui: &mut Ui,
    top_border: bool,
    icon: Option<egui::ImageSource<'static>>,
    left: &str,
    right: Option<&str>,
) {
    let rect = ui.available_rect_before_wrap();
    if top_border {
        ui.painter()
            .hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, THEME.border_subtle));
    }
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(16.0, 0.0))
        .show(ui, |ui| {
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

/// Mod row frame: 34 px card, the selected one lighter with a lavender bar at the left.
/// A list row: lighter when selected, green when related to the selected mod.
fn mod_row_frame(selected: bool, related: bool) -> egui::Frame {
    let (fill, stroke) = if selected {
        (THEME.bg_surface_hover, Stroke::NONE)
    } else if related {
        (
            super::theme::mix(THEME.bg_card, THEME.accent_emerald, 0.14),
            Stroke::new(1.0_f32, super::theme::mix(THEME.bg_card, THEME.accent_emerald, 0.6)),
        )
    } else {
        (THEME.bg_card, Stroke::NONE)
    };
    egui::Frame {
        inner_margin: egui::Margin::symmetric(8.0, 5.0),
        outer_margin: egui::Margin::symmetric(0.0, 1.0),
        rounding: Rounding::same(6.0),
        shadow: egui::epaint::Shadow::NONE,
        fill,
        stroke,
    }
}

/// Green badge on a mod related to the selected one: what it is to it.
fn relation_badge(ui: &mut Ui, relation: Option<&crate::relations::Relation>, t: crate::i18n::I18n) {
    use crate::relations::Relation;
    let Some(r) = relation else { return };
    let (label, hint) = match r {
        Relation::Translation => (TranslationKey::RelTranslation, TranslationKey::RelTranslationHint),
        Relation::Addon => (TranslationKey::RelAddon, TranslationKey::RelAddonHint),
        Relation::Original => (TranslationKey::RelOriginal, TranslationKey::RelOriginalHint),
        Relation::Dependency => (TranslationKey::RelDependency, TranslationKey::RelDependencyHint),
        Relation::Dependent => (TranslationKey::RelDependent, TranslationKey::RelDependentHint),
    };
    if let Some(resp) = tag_if_fits(ui, t.get(label), THEME.accent_emerald) {
        resp.on_hover_text(t.get(hint));
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
    (
        left,
        RowSplit {
            right_x,
            max_x: full.max.x,
        },
    )
}

/// Reserves the content area in the row (never wider than its slot) and returns the rect
/// for the buttons, as tall as the content.
fn finish_row(ui: &mut Ui, left: &Ui, split: &RowSplit) -> egui::Rect {
    let used = left.min_rect();
    let content = egui::Rect::from_min_max(used.min, egui::pos2(split.right_x - 4.0, used.max.y));
    ui.allocate_rect(content, egui::Sense::hover());
    egui::Rect::from_min_max(
        egui::pos2(split.right_x, used.min.y),
        egui::pos2(split.max_x, used.max.y),
    )
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
fn render_version_issues(app: &mut App, ui: &mut Ui) {
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

    // Optimizer DDS stored the wrong way up for this game version: upside down in the game
    // until flipped or rolled back on the Optimizer tab.
    let leftover = app.opt_wrong_orientation;
    if leftover > 0 {
        let mut open = false;
        card_container(ui, |ui| {
            // Button at the right edge first, so the text gets the rest of the width and wraps in it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                open = secondary_button(ui, t.get(TranslationKey::ModsOpenOptimizer)).clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(svg_icon(ICON_STATUS_ERROR, Vec2::splat(18.0), THEME.accent_rose));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                t.get(TranslationKey::ModsLeftoverDds)
                                    .replace("{}", &leftover.to_string()),
                            )
                            .color(THEME.accent_rose)
                            .strong(),
                        )
                        .wrap(),
                    );
                });
            });
        });
        ui.add_space(4.0);
        if open {
            app.current_tab = crate::app::AppTab::Optimizer;
        }
    }

    if !issues.is_empty() {
        card_container(ui, |ui| {
            for issue in &issues {
                let text = match issue {
                    crate::version_check::VersionIssue::PrepatcherOnWaylith => t.get(TranslationKey::VersionPrepatcher),
                    crate::version_check::VersionIssue::HarmonyNotUpdated => t.get(TranslationKey::VersionHarmony),
                };
                ui.horizontal(|ui| {
                    ui.add(svg_icon(ICON_STATUS_ERROR, Vec2::splat(18.0), THEME.accent_rose));
                    ui.add(egui::Label::new(egui::RichText::new(text).color(THEME.accent_rose).strong()).wrap());
                });
            }
        });
        ui.add_space(4.0);
    }
    if unsupported > 0 {
        if let Some(mm) = game_mm.as_deref() {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_INFO, Vec2::splat(14.0), THEME.text_muted));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(fmt(
                            t.get(TranslationKey::ModsVersionSummary),
                            &[mm, &unsupported.to_string()],
                        ))
                        .color(THEME.text_muted)
                        .size(12.0),
                    )
                    .wrap(),
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
    if let Some(resp) = tag_if_fits(
        ui,
        &fmt(t.get(TranslationKey::ModsVersionBadge), &[&newest]),
        THEME.accent_amber,
    ) {
        resp.on_hover_text(fmt(t.get(TranslationKey::ModsVersionHint), &[game_mm.unwrap_or("?")]));
    }
}

/// Sky-blue "update" badge: the Workshop has a newer version of the mod.
fn update_badge(
    ui: &mut Ui,
    update: Option<&crate::mod_updates::ModUpdate>,
    t: crate::i18n::I18n,
    clickable: bool,
) -> bool {
    let Some(u) = update else { return false };
    let text = if u.newer_supports_game {
        t.get(TranslationKey::UpdatesBadgeVersion)
    } else {
        t.get(TranslationKey::UpdatesBadge)
    };
    let Some(resp) = tag_if_fits(ui, text, THEME.accent_info) else {
        return false;
    };
    let mut hint = fmt(
        t.get(TranslationKey::UpdatesBadgeHint),
        &[&local_date_time(u.remote_time)],
    );
    if u.estimated {
        hint.push('\n');
        hint.push_str(t.get(TranslationKey::UpdatesEstimated));
    }
    if clickable {
        hint.push('\n');
        hint.push_str(t.get(TranslationKey::UpdatesClickHint));
        resp.interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hint)
            .clicked()
    } else {
        resp.on_hover_text(hint);
        false
    }
}

/// The "Filters" button of a narrow window: opens or folds the chips; lit when a filter is on.
fn filters_toggle(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let selected = app.category_filter.len() + usize::from(app.filter_updates) + usize::from(app.filter_no_version);
    let label = if selected > 0 {
        format!("{} · {}", t.get(TranslationKey::ModsFiltersBtn), selected)
    } else {
        t.get(TranslationKey::ModsFiltersBtn).to_string()
    };
    let style = if selected > 0 || app.show_filter_chips {
        ButtonStyle::Primary
    } else {
        ButtonStyle::Secondary
    };
    if styled_button(ui, None, &label, style)
        .on_hover_text(t.get(TranslationKey::ModsFiltersHint))
        .clicked()
    {
        app.show_filter_chips = !app.show_filter_chips;
    }
}

/// T2: the modpack translation search found a translation for this mod (not subscribed yet).
/// Returns whether it was clicked (opens the results on the Translations tab).
fn translation_badge(
    ui: &mut Ui,
    title: Option<&String>,
    lang: crate::models::TranslationTargetLang,
    t: crate::i18n::I18n,
) -> bool {
    let Some(title) = title else { return false };
    let Some(resp) = tag_if_fits(
        ui,
        &fmt(t.get(TranslationKey::TransHintBadge), &[lang.code()]),
        THEME.accent_cyan,
    ) else {
        return false;
    };
    resp.interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(fmt(t.get(TranslationKey::TransHintHover), &[lang.label(), title]))
        .clicked()
}

/// A mod name cut to `max` characters with "…" (for button labels).
fn short_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        format!(
            "{}…",
            name.chars().take(max.saturating_sub(1)).collect::<String>().trim_end()
        )
    }
}

/// F10: badge for a mod that produced errors in the last run's Player.log.
/// Clicking it opens the Log Analyzer tab.
fn log_error_badge(ui: &mut Ui, culprit: Option<&crate::analyzer::CrashCulprit>, t: crate::i18n::I18n) -> bool {
    let Some(c) = culprit else { return false };
    let color = match c.severity {
        crate::analyzer::ErrorSeverity::Warning => THEME.accent_amber,
        _ => THEME.accent_rose,
    };
    let Some(resp) = tag_if_fits(ui, t.get(TranslationKey::ModsLogErrorBadge), color) else {
        return false;
    };
    let sev = c.severity.badge_text(t.language() == crate::i18n::Language::Russian);
    let hint = fmt(
        t.get(TranslationKey::ModsLogErrorHint),
        &[sev, &c.occurrences.to_string(), &c.headline],
    );
    resp.interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hint)
        .clicked()
}

/// Row of category chips above the lists: "All" + every category that some mod has, with counts.
/// Clicking chips toggles them; a mod is shown if it has any selected category.
fn render_category_filter(app: &mut App, ui: &mut Ui, toggle_drawn: bool) {
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

    let game_mm = app.steam_paths.game_major_minor();
    let no_version = app
        .all_mods
        .iter()
        .filter(|m| crate::version_check::supports_version(m, game_mm.as_deref()) == Some(false))
        .count();
    let updates = app.mod_updates.as_ref().map(|r| r.outdated_count()).unwrap_or(0);

    // Narrow window: the chips take many rows, so they fold behind a "Filters" button
    // (drawn here, or already drawn by the caller next to the list switch).
    if toggle_drawn {
        if !app.show_filter_chips {
            return;
        }
    } else if ui.available_width() < NARROW_FILTERS {
        filters_toggle(app, ui);
        if !app.show_filter_chips {
            return;
        }
        ui.add_space(6.0);
    }

    let mut update_all = false;
    // Mods SteamCMD can update (the Steam client updates its own subscriptions).
    let updatable = if app.steamcmd_active() {
        app.steamcmd_updatable_ids().len()
    } else {
        0
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        let all_selected = app.category_filter.is_empty() && !app.filter_updates && !app.filter_no_version;
        if chip(
            ui,
            None,
            t.get(TranslationKey::CategoriesAll),
            app.all_mods.len(),
            all_selected,
        )
        .clicked()
        {
            app.category_filter.clear();
            app.filter_updates = false;
            app.filter_no_version = false;
        }
        // Status filters: a newer Workshop version, no support for the installed game version.
        if updates > 0 {
            if chip(
                ui,
                Some((ICON_STEAM, THEME.accent_info)),
                t.get(TranslationKey::UpdatesChip),
                updates,
                app.filter_updates,
            )
            .on_hover_text(t.get(TranslationKey::UpdatesChipHint))
            .clicked()
            {
                app.filter_updates = !app.filter_updates;
            }
            if updatable > 0 {
                let label = fmt(t.get(TranslationKey::UpdatesBtnUpdateAll), &[&updatable.to_string()]);
                if ui
                    .add_enabled_ui(!app.steam_busy(), |ui| small_button(ui, &label, ButtonStyle::Primary))
                    .inner
                    .clicked()
                {
                    update_all = true;
                }
            }
        }
        if no_version > 0 {
            let label = fmt(
                t.get(TranslationKey::NoVersionChip),
                &[game_mm.as_deref().unwrap_or("?")],
            );
            if chip(
                ui,
                Some((ICON_STATUS_WARNING, THEME.accent_amber)),
                &label,
                no_version,
                app.filter_no_version,
            )
            .on_hover_text(t.get(TranslationKey::NoVersionChipHint))
            .clicked()
            {
                app.filter_no_version = !app.filter_no_version;
            }
        }
        for cat in ModCategory::ALL {
            let Some(n) = counts.get(&cat) else {
                continue;
            };
            let selected = app.category_filter.contains(&cat);
            let (_, fg) = cat.colors();
            let fg = Color32::from_rgb(fg[0], fg[1], fg[2]);
            if chip(
                ui,
                Some((category_icon(cat), fg)),
                cat.label(t.language()),
                *n,
                selected,
            )
            .clicked()
            {
                if selected {
                    app.category_filter.remove(&cat);
                } else {
                    app.category_filter.insert(cat);
                }
            }
        }
    });
    if update_all {
        app.update_outdated_mods();
    }
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
                ui.add(svg_icon(
                    category_icon(cat),
                    Vec2::splat(14.0),
                    Color32::from_rgb(fg[0], fg[1], fg[2]),
                ));
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

/// Small icon before a mod name: Steam Workshop or the game's Mods folder (hover says which).
fn origin_icon(ui: &mut Ui, origin: ModOrigin, t: crate::i18n::I18n) {
    let (icon, color, key) = match origin {
        ModOrigin::Workshop => (ICON_STATUS_WORKSHOP, THEME.accent_info, TranslationKey::ModsOriginSteam),
        ModOrigin::Local => (ICON_STATUS_LOCAL, THEME.text_muted, TranslationKey::ModsOriginLocal),
    };
    ui.add(svg_icon(icon, Vec2::splat(14.0), color))
        .on_hover_text(t.get(key));
}

/// Icon, background and foreground for a mod without a preview image:
/// shield for protected content, otherwise the icon of its strongest category.
fn mod_fallback_icon(
    is_protected: bool,
    category: Option<ModCategory>,
) -> (egui::ImageSource<'static>, Color32, Color32) {
    if is_protected {
        return (
            ICON_SHIELD,
            crate::ui::theme::tint(THEME.accent_amber),
            THEME.accent_amber,
        );
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
        ui.add(
            egui::Image::new(tex)
                .rounding(Rounding::same(4.0))
                .fit_to_exact_size(egui::vec2(20.0, 20.0)),
        );
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
    let update_map = app.mod_updates.as_ref().map(|r| r.mods.clone()).unwrap_or_default();
    let log_culprits = app.log_culprit_map();
    // Mods related to the selected one: highlighted green, with what they are to it.
    let related = app.related_to_selected();
    let mut open_analyzer = false;
    // T2: mods the modpack translation search found a translation for.
    let trans_hints = app.translation_hints();
    let trans_lang = app.batch_lang;
    let mut open_translations = false;
    let game_data = app.steam_paths.game_data_path.clone();

    // Header: title, count; on the right — search and move up / down for the selected mod.
    let active_count = app.active_mods.len();
    let mut move_selected: i32 = 0;
    let mut open_history = false;
    let search_hint = app.i18n.get(TranslationKey::ModsSearchActive);
    panel_header(ui, t.get(TranslationKey::ActiveModsTitle), active_count, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        search_input_width(ui, &mut app.active_search_query, search_hint, 192.0);
        if boxed_icon_button(ui, ICON_ARROW_DOWN)
            .on_hover_text(t.get(TranslationKey::ModsCtxMoveDown))
            .clicked()
        {
            move_selected = 1;
        }
        if boxed_icon_button(ui, ICON_ARROW_UP)
            .on_hover_text(t.get(TranslationKey::ModsCtxMoveUp))
            .clicked()
        {
            move_selected = -1;
        }
        if boxed_icon_button(ui, ICON_HISTORY)
            .on_hover_text(tr!(
                "История: вернуть порядок модов из прошлых сохранений",
                "History: bring back the load order from earlier saves"
            ))
            .clicked()
        {
            open_history = true;
        }
    });
    if open_history {
        app.open_history();
    }
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
    let mut download_dep_ws_id = None;
    let mut deactivate_selected = false;
    let mut toggle_ignore_id = None;
    let mut cat_toggle: Option<(String, ModCategory)> = None;
    let mut cat_reset: Option<String> = None;
    let steamcmd_active = app.steamcmd_active();
    let steam_busy = app.steam_busy();
    let workshop_index = app.workshop_index.clone();

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
                && app.passes_extra_filters(mod_opt, game_mm.as_deref())
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

    // Drag and drop (F1): rows drawn this frame (index in active_mods, rect) and a drag that starts now.
    let mut drawn_rows: Vec<(usize, egui::Rect)> = Vec::new();
    let mut start_drag: Option<ModDrag> = None;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("active_mods_scroll_right")
        .auto_shrink([false, false])
        .max_height(list_height)
        .drag_to_scroll(false);
    if let Some(offset) = app.active_scroll_to.take() {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let mut scroll_offset = app.active_scroll_offset;

    // Only rows inside the visible area are built (M1): big modpacks stay smooth.
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(8.0, 0.0))
        .show(ui, |ui| {
            let output = scroll.show_rows(ui, row_height, visible_rows.len(), |ui, row_range| {
                let rows_top = ui.cursor().top();
                let rows_count = row_range.len();
                shown_range = row_range.clone();
                for &idx in &visible_rows[row_range] {
                    let pkg_id = &app.active_mods[idx];
                    let drag_resp = row_drag_handle(
                        ui,
                        egui::Id::new(("active_row_drag", pkg_id.to_lowercase())),
                        row_height,
                    );
                    let is_dragged = app
                        .mod_drag
                        .as_ref()
                        .is_some_and(|d| d.from_active && d.ids.iter().any(|id| id.eq_ignore_ascii_case(pkg_id)));
                    let mod_info_opt = lookup_mod(&app.mod_index, &app.all_mods, pkg_id);
                    let display_name = mod_info_opt.map(|m| m.name.as_str()).unwrap_or(pkg_id.as_str());

                    let is_selected = app.selected_mod_ids.iter().any(|s| s.eq_ignore_ascii_case(pkg_id))
                        || app
                            .selected_mod_id
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(pkg_id));
                    let is_protected = is_protected_package_id(pkg_id);

                    // The installed mod's ID in lower case: an `<id>_steam` entry finds its
                    // warning, and its preview is loaded once for every list (R11, R12).
                    let mod_key = mod_info_opt.map_or_else(|| pkg_id.to_lowercase(), |m| m.package_id.to_lowercase());

                    // Dependency warning for this mod
                    let dep_problem = dep_by_mod.get(&mod_key);
                    let incompatible = matches!(dep_problem, Some(DependencyProblem::Incompatible { .. }));

                    let relation = related.get(&mod_key);
                    let row = mod_row_frame(is_selected, relation.is_some()).show(ui, |ui| {
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
                                    egui::Label::new(
                                        egui::RichText::new(format!("{}.", idx + 1))
                                            .color(THEME.text_muted)
                                            .size(12.0),
                                    ),
                                );

                                // Mod Icon badge
                                let tex_opt = app.mod_textures.get(&mod_key);
                                if tex_opt.is_none() {
                                    missing_icons
                                        .push((mod_key.clone(), mod_info_opt.and_then(|m| m.preview_path.clone())));
                                }
                                let texture = tex_opt.and_then(|t| t.as_ref());
                                render_mod_icon_pill(
                                    ui,
                                    mod_info_opt.and_then(|m| m.categories.first().copied()),
                                    is_protected,
                                    texture,
                                );
                                if let Some(m) = mod_info_opt.filter(|m| !is_official_content(m, game_data.as_deref()))
                                {
                                    origin_icon(ui, m.origin, t);
                                }

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
                                let label_resp =
                                    mod_name_label(ui, if is_selected { name_text.strong() } else { name_text });

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
                                            && ui
                                                .button(
                                                    app.i18n
                                                        .get(TranslationKey::ModsCtxDeactivateSelected)
                                                        .replace("{}", &app.selected_mod_ids.len().to_string()),
                                                )
                                                .clicked()
                                        {
                                            deactivate_selected = true;
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                    }
                                    if idx > 0 && ui.button(app.i18n.get(TranslationKey::ModsCtxMoveUp)).clicked() {
                                        move_up_idx = Some(idx);
                                        ui.close_menu();
                                    }
                                    if idx < app.active_mods.len().saturating_sub(1)
                                        && ui.button(app.i18n.get(TranslationKey::ModsCtxMoveDown)).clicked()
                                    {
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
                                    if !is_protected_package_id(pkg_id)
                                        && !pkg_id.to_lowercase().starts_with("ludeon.rimworld")
                                    {
                                        ui.separator();
                                        let is_ignored = app.opt_ignore_list.contains(&pkg_id.to_lowercase());
                                        let ignore_text = if is_ignored {
                                            app.i18n.get(TranslationKey::ModsCtxOptSkip)
                                        } else {
                                            app.i18n.get(TranslationKey::ModsCtxOptAllow)
                                        };
                                        if ui.button(ignore_text).clicked() {
                                            toggle_ignore_id = Some(pkg_id.clone());
                                            ui.close_menu();
                                        }
                                    }
                                    if let Some(m) = mod_info_opt
                                        .filter(|m| !m.package_id.to_lowercase().starts_with("ludeon.rimworld"))
                                    {
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
                                relation_badge(ui, relation, t);
                                if let Some(m) = mod_info_opt {
                                    if !is_protected {
                                        category_badges(ui, &m.categories, t, 2);
                                    }
                                    if log_error_badge(ui, log_culprits.get(&m.package_id.to_lowercase()), t) {
                                        open_analyzer = true;
                                    }
                                    version_badge(ui, m, game_mm.as_deref(), t);
                                    if let Some(u) = update_map.get(&m.package_id.to_lowercase()) {
                                        if update_badge(
                                            ui,
                                            Some(u),
                                            t,
                                            steamcmd_active && m.origin != ModOrigin::Workshop,
                                        ) {
                                            download_dep_ws_id = Some(u.workshop_id.clone());
                                        }
                                    }
                                    if translation_badge(
                                        ui,
                                        trans_hints.get(&m.package_id.to_lowercase()),
                                        trans_lang,
                                        t,
                                    ) {
                                        open_translations = true;
                                    }
                                }
                                // Listed in ModsConfig.xml but not found by the scanner: kept on save, not dropped.
                                if mod_info_opt.is_none() {
                                    tag_if_fits(ui, app.i18n.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                                }

                                // Dependency problems
                                if let Some(prob) = dep_problem {
                                    match prob {
                                        DependencyProblem::Disabled { required_id, .. } => {
                                            tag_if_fits(
                                                ui,
                                                app.i18n.get(TranslationKey::ModsDepNotInList),
                                                THEME.accent_amber,
                                            );
                                            if fits(ui, 60.0)
                                                && small_button(
                                                    ui,
                                                    app.i18n.get(TranslationKey::ModsDepEnable),
                                                    ButtonStyle::Success,
                                                )
                                                .clicked()
                                            {
                                                enable_dep_id = Some(required_id.clone());
                                            }
                                        }
                                        DependencyProblem::Missing { required_id, .. } => {
                                            let dep_name = workshop_index
                                                .as_ref()
                                                .and_then(|idx| idx.name_for_package(required_id))
                                                .unwrap_or(required_id.as_str());
                                            if let Some(resp) = tag_if_fits(
                                                ui,
                                                app.i18n.get(TranslationKey::ModsDepNotFound),
                                                THEME.accent_rose,
                                            ) {
                                                resp.on_hover_text(format!("{dep_name} ({required_id})"));
                                            }
                                            if let Some(ws_id) = workshop_index
                                                .as_ref()
                                                .and_then(|idx| idx.lookup(required_id, None))
                                                .map(str::to_string)
                                            {
                                                if fits(ui, 72.0)
                                                    && ui
                                                        .add_enabled_ui(!steam_busy, |ui| {
                                                            small_button(
                                                                ui,
                                                                app.i18n.get(TranslationKey::ModsDepDownload),
                                                                ButtonStyle::Primary,
                                                            )
                                                        })
                                                        .inner
                                                        .on_hover_text(format!("{dep_name} (#{ws_id})"))
                                                        .clicked()
                                                {
                                                    download_dep_ws_id = Some(ws_id);
                                                }
                                            }
                                        }
                                        DependencyProblem::WrongOrder { .. } => {
                                            tag_if_fits(
                                                ui,
                                                app.i18n.get(TranslationKey::ModsDepOrder),
                                                THEME.accent_amber,
                                            );
                                        }
                                        DependencyProblem::Incompatible { other_name, .. } => {
                                            if let Some(resp) = tag_if_fits(
                                                ui,
                                                app.i18n.get(TranslationKey::ModsDepIncompatible),
                                                THEME.accent_rose,
                                            ) {
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
                                    } else if boxed_icon_button(ui, ICON_CLOSE)
                                        .on_hover_text(t.get(TranslationKey::BtnDeactivate))
                                        .clicked()
                                    {
                                        deactivate_idx = Some(idx);
                                    }
                                    if idx + 1 >= app.active_mods.len() {
                                        ui.add_space(24.0 + 4.0);
                                    } else if boxed_icon_button(ui, ICON_ARROW_DOWN)
                                        .on_hover_text(t.get(TranslationKey::ModsCtxMoveDown))
                                        .clicked()
                                    {
                                        move_down_idx = Some(idx);
                                    }
                                    if idx == 0 {
                                        ui.add_space(24.0);
                                    } else if boxed_icon_button(ui, ICON_ARROW_UP)
                                        .on_hover_text(t.get(TranslationKey::ModsCtxMoveUp))
                                        .clicked()
                                    {
                                        move_up_idx = Some(idx);
                                    }
                                },
                            );
                        });
                    });
                    if is_selected {
                        paint_selection_bar(ui, row.response.rect);
                    }
                    if is_dragged {
                        fade_row(ui, row.response.rect);
                    }
                    drawn_rows.push((idx, row.response.rect));

                    if drag_resp.drag_started() {
                        // A selected row drags the whole selection (in load order), any other row only itself.
                        let in_selection = app.selected_mod_ids.iter().any(|s| s.eq_ignore_ascii_case(pkg_id));
                        let ids: Vec<String> = if in_selection && app.selected_mod_ids.len() > 1 {
                            app.active_mods
                                .iter()
                                .filter(|a| app.selected_mod_ids.iter().any(|s| s.eq_ignore_ascii_case(a)))
                                .cloned()
                                .collect()
                        } else {
                            vec![pkg_id.clone()]
                        };
                        start_drag = Some(ModDrag {
                            label: drag_label(display_name, ids.len()),
                            ids,
                            from_active: true,
                        });
                    }
                }
                measured_row_height = measure_row_height(ui, rows_top, rows_count);

                if app.mod_drag.is_some() {
                    let view = ui.clip_rect();
                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                        let over_x = pos.x >= view.left() && pos.x <= view.right();
                        if over_x && pos.y >= view.top() && pos.y <= view.bottom() {
                            // Insert before the first row whose middle is below the cursor.
                            let (to, line_y) = drawn_rows
                                .iter()
                                .find(|(_, r)| pos.y < r.center().y)
                                .map(|(i, r)| (*i, r.top()))
                                .or_else(|| drawn_rows.last().map(|(i, r)| (i + 1, r.bottom())))
                                .unwrap_or((app.active_mods.len(), view.top() + 2.0));
                            app.drop_target = Some(DropTarget::Active(to));
                            let x = view.x_range().shrink(4.0);
                            let painter = ui.painter().with_clip_rect(view);
                            painter.hline(x, line_y, Stroke::new(2.0_f32, THEME.accent_cyan));
                            painter.circle_filled(egui::pos2(x.min + 3.0, line_y), 4.0, THEME.accent_cyan);
                        }
                        // Near the top or bottom edge the list scrolls by itself, faster the closer the cursor.
                        const EDGE: f32 = 36.0;
                        let speed = if pos.y < view.top() + EDGE {
                            -(view.top() + EDGE - pos.y)
                        } else if pos.y > view.bottom() - EDGE {
                            pos.y - (view.bottom() - EDGE)
                        } else {
                            0.0
                        };
                        let near = pos.y > view.top() - 80.0 && pos.y < view.bottom() + 80.0;
                        if over_x && near && speed != 0.0 {
                            let step = (speed * 0.4).clamp(-20.0, 20.0);
                            app.active_scroll_to = Some((app.active_scroll_offset + step).max(0.0));
                        }
                    }
                }
            });
            scroll_offset = output.state.offset.y;
        });
    app.active_scroll_offset = scroll_offset;
    if let Some(h) = measured_row_height {
        app.active_row_height = h;
    }
    if start_drag.is_some() {
        app.mod_drag = start_drag;
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
    panel_footer(
        ui,
        false,
        None,
        &showing,
        Some(tr!("Загрузка сверху вниз", "Loads from top to bottom")),
    );

    // Request missing icons
    for (pkg_id, preview_opt) in missing_icons {
        app.mod_textures.insert(pkg_id.clone(), None);
        if let Some(path) = preview_opt {
            let _ = app.icon_tx.send((pkg_id, path));
        }
    }
    if open_translations {
        app.open_batch_translations();
    }
    if open_analyzer {
        app.current_tab = crate::app::AppTab::Analyzer;
    }

    if let Some(pkg_id) = toggle_ignore_id {
        app.toggle_optimizer_ignore(&pkg_id);
    }
    if let Some(ws_id) = download_dep_ws_id {
        app.subscribe_steam_mod(&ws_id);
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
    let game_data = app.steam_paths.game_data_path.clone();
    // Whole panel: drop zone for disabling dragged active mods.
    let panel_rect = ui.max_rect();
    let game_mm = app.steam_paths.game_major_minor();
    let update_map = app.mod_updates.as_ref().map(|r| r.mods.clone()).unwrap_or_default();
    let log_culprits = app.log_culprit_map();
    // Mods related to the selected one: highlighted green, with what they are to it.
    let related = app.related_to_selected();
    let mut open_analyzer = false;
    // T2: mods the modpack translation search found a translation for.
    let trans_hints = app.translation_hints();
    let trans_lang = app.batch_lang;
    let mut open_translations = false;
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

    let total_text = app
        .i18n
        .get(TranslationKey::ModsTotal)
        .replace("{}", &app.all_mods.len().to_string());
    panel_header(
        ui,
        t.get(TranslationKey::AvailableModsTitle),
        inactive_mods.len(),
        |ui| {
            ui.label(egui::RichText::new(total_text).color(THEME.text_muted).size(12.0));
        },
    );

    let query_lower = app.mod_search_query.trim().to_lowercase();
    let mut activate_id = None;
    let mut activate_selected = false;
    let mut toggle_ignore_id = None;
    let mut cat_toggle: Option<(String, ModCategory)> = None;
    let mut cat_reset: Option<String> = None;
    let mut download_dep_ws_id: Option<String> = None;
    let steamcmd_active = app.steamcmd_active();

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
                && app.passes_extra_filters(Some(m), game_mm.as_deref())
        })
        .collect();
    // Visible IDs for Shift-range selection
    let visible_inactive_ids: Vec<String> = visible_mods.iter().map(|m| m.package_id.clone()).collect();

    let mut missing_icons = Vec::new();
    let row_height = app.inactive_row_height;
    let mut measured_row_height: Option<f32> = None;
    let list_height = (ui.available_height() - 48.0).max(60.0);
    let mut start_drag: Option<ModDrag> = None;

    // Only rows inside the visible area are built (M1).
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(8.0, 0.0))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("inactive_mods_scroll_left")
                .auto_shrink([false, false])
                .drag_to_scroll(false)
                .max_height(list_height)
                .min_scrolled_height(list_height)
                .show_rows(ui, row_height, visible_mods.len(), |ui, row_range| {
                    let rows_top = ui.cursor().top();
                    let rows_count = row_range.len();
                    for m in visible_mods.iter().skip(row_range.start).take(rows_count) {
                        let drag_resp = row_drag_handle(
                            ui,
                            egui::Id::new(("inactive_row_drag", m.package_id.to_lowercase())),
                            row_height,
                        );
                        let is_dragged = app.mod_drag.as_ref().is_some_and(|d| {
                            !d.from_active && d.ids.iter().any(|id| id.eq_ignore_ascii_case(&m.package_id))
                        });
                        let is_selected = app.selected_mod_ids.contains(&m.package_id)
                            || app.selected_mod_id.as_deref() == Some(&m.package_id);

                        let relation = related.get(&m.package_id.to_lowercase());
                        let row = mod_row_frame(is_selected, relation.is_some()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                // Fixed slot for the "+" button at the right edge (see the active list).
                                let (mut left_ui, split) = split_row(ui, INACTIVE_ROW_BUTTONS_WIDTH);
                                {
                                    let ui = &mut left_ui;
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    // Mod Icon badge
                                    let tex_key = m.package_id.to_lowercase();
                                    let tex_opt = app.mod_textures.get(&tex_key);
                                    if tex_opt.is_none() {
                                        missing_icons.push((tex_key, m.preview_path.clone()));
                                    }
                                    let texture = tex_opt.and_then(|t| t.as_ref());
                                    render_mod_icon_pill(ui, m.categories.first().copied(), false, texture);
                                    if !is_official_content(m, game_data.as_deref()) {
                                        origin_icon(ui, m.origin, t);
                                    }

                                    let name_text = egui::RichText::new(&m.name).color(THEME.text_primary).size(13.0);
                                    let label_resp =
                                        mod_name_label(ui, if is_selected { name_text.strong() } else { name_text });

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
                                                let cur_pos =
                                                    visible_inactive_ids.iter().position(|id| id == &m.package_id);
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
                                            && ui
                                                .button(
                                                    app.i18n
                                                        .get(TranslationKey::ModsCtxActivateSelected)
                                                        .replace("{}", &app.selected_mod_ids.len().to_string()),
                                                )
                                                .clicked()
                                        {
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

                                        if !is_protected_package_id(&pkg_id_ctx)
                                            && !pkg_id_ctx.to_lowercase().starts_with("ludeon.rimworld")
                                        {
                                            ui.separator();
                                            let is_ignored = app.opt_ignore_list.contains(&pkg_id_ctx.to_lowercase());
                                            let ignore_text = if is_ignored {
                                                app.i18n.get(TranslationKey::ModsCtxOptSkip)
                                            } else {
                                                app.i18n.get(TranslationKey::ModsCtxOptAllow)
                                            };
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

                                    relation_badge(ui, relation, t);
                                    category_badges(ui, &m.categories, t, 2);
                                    if log_error_badge(ui, log_culprits.get(&m.package_id.to_lowercase()), t) {
                                        open_analyzer = true;
                                    }
                                    version_badge(ui, m, game_mm.as_deref(), t);
                                    if let Some(u) = update_map.get(&m.package_id.to_lowercase()) {
                                        if update_badge(
                                            ui,
                                            Some(u),
                                            t,
                                            steamcmd_active && m.origin != ModOrigin::Workshop,
                                        ) {
                                            download_dep_ws_id = Some(u.workshop_id.clone());
                                        }
                                    }
                                    if translation_badge(
                                        ui,
                                        trans_hints.get(&m.package_id.to_lowercase()),
                                        trans_lang,
                                        t,
                                    ) {
                                        open_translations = true;
                                    }
                                }
                                let right_rect = finish_row(ui, &left_ui, &split);

                                ui.allocate_new_ui(
                                    egui::UiBuilder::new()
                                        .max_rect(right_rect)
                                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                                    |ui| {
                                        ui.add_space(8.0); // room for the scrollbar
                                        if boxed_icon_button(ui, ICON_PLUS)
                                            .on_hover_text(t.get(TranslationKey::BtnActivate))
                                            .clicked()
                                        {
                                            activate_id = Some(m.package_id.clone());
                                        }
                                    },
                                );
                            });
                        });
                        if is_selected {
                            paint_selection_bar(ui, row.response.rect);
                        }
                        if is_dragged {
                            fade_row(ui, row.response.rect);
                        }
                        if drag_resp.drag_started() {
                            let ids: Vec<String> =
                                if app.selected_mod_ids.contains(&m.package_id) && app.selected_mod_ids.len() > 1 {
                                    visible_inactive_ids
                                        .iter()
                                        .filter(|id| app.selected_mod_ids.contains(*id))
                                        .cloned()
                                        .collect()
                                } else {
                                    vec![m.package_id.clone()]
                                };
                            start_drag = Some(ModDrag {
                                label: drag_label(&m.name, ids.len()),
                                ids,
                                from_active: false,
                            });
                        }
                    }
                    measured_row_height = measure_row_height(ui, rows_top, rows_count);
                });
        });
    if let Some(h) = measured_row_height {
        app.inactive_row_height = h;
    }
    if start_drag.is_some() {
        app.mod_drag = start_drag;
    }

    panel_footer(
        ui,
        true,
        Some(ICON_PLUS),
        tr!(
            "Нажмите + или перетащите мод вправо, чтобы включить его.",
            "Use + or drag a mod to the right to enable it."
        ),
        None,
    );

    // Active mods dragged over this panel: dropping them here disables them.
    if app.mod_drag.as_ref().is_some_and(|d| d.from_active) {
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            if panel_rect.contains(pos) {
                app.drop_target = Some(DropTarget::Available);
                ui.painter().rect_stroke(
                    panel_rect.shrink(1.0),
                    Rounding::same(6.0),
                    Stroke::new(2.0_f32, THEME.accent_cyan),
                );
            }
        }
    }

    for (pkg_id, preview_opt) in missing_icons {
        app.mod_textures.insert(pkg_id.clone(), None);
        if let Some(path) = preview_opt {
            let _ = app.icon_tx.send((pkg_id, path));
        }
    }
    if open_translations {
        app.open_batch_translations();
    }
    if open_analyzer {
        app.current_tab = crate::app::AppTab::Analyzer;
    }

    if let Some(pkg_id) = toggle_ignore_id {
        app.toggle_optimizer_ignore(&pkg_id);
    }
    if let Some(ws_id) = download_dep_ws_id {
        app.subscribe_steam_mod(&ws_id);
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
        app.all_mods
            .iter()
            .find(|m| m.package_id.eq_ignore_ascii_case(sel_id))
            .cloned()
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
                let tex_key = m.package_id.to_lowercase();
                if !app.mod_textures.contains_key(&tex_key) {
                    if let Some(ref path) = m.preview_path {
                        let _ = app.icon_tx.send((tex_key.clone(), path.clone()));
                        app.mod_textures.insert(tex_key.clone(), None);
                    }
                }

                if let Some(Some(tex)) = app.mod_textures.get(&tex_key) {
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

                // Text column: exactly the width left of the card, so every line wraps
                // inside it instead of running past the edge in a narrow window.
                let text_width = ui.available_width().max(120.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(text_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                    ui.set_max_width(text_width);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&m.name).color(THEME.text_primary).size(14.0).strong(),
                        )
                        .wrap(),
                    );

            // Badges, categories and the Folder / Steam buttons: one row that wraps when narrow.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                if is_protected {
                    icon_badge(ui, ICON_SHIELD, app.i18n.get(TranslationKey::ModsBadgeProtected), Color32::from_rgb(107, 33, 168).linear_multiply(0.4), Color32::from_rgb(192, 132, 252));
                }
                origin_badge(ui, m.origin);
                if !m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
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
                }
                if small_button(ui, app.i18n.get(TranslationKey::ModsBtnFolder), ButtonStyle::Secondary).clicked() {
                    open_external(&m.dir_path);
                }
                if let Some(ref steam_id) = m.steam_item_id {
                    if small_button(ui, "Steam", ButtonStyle::Secondary).clicked() {
                        open_external(workshop_url(steam_id));
                    }
                }
            });

            // Package ID and author, each on its own line (a long author list wraps on its own).
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Package ID: {}", m.package_id))
                        .color(Color32::from_rgb(245, 158, 11))
                        .monospace()
                        .size(12.0),
                )
                .wrap(),
            );
            if !m.author.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(app.i18n.get(TranslationKey::ModsAuthor).replace("{}", &m.author))
                            .color(THEME.text_muted)
                            .size(12.0),
                    )
                    .wrap(),
                );
            }

            // Dependencies list
            let mut dep_download_ws_id = None;
            if !m.dependencies.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                    ui.label(egui::RichText::new(app.i18n.get(TranslationKey::ModsDependencies)).color(THEME.text_muted).size(12.0));
                    for dep in &m.dependencies {
                        let installed = app.find_mod(dep);
                        let label = installed
                            .map(|im| im.name.as_str())
                            .or_else(|| {
                                app.workshop_index
                                    .as_ref()
                                    .and_then(|idx| idx.name_for_package(dep))
                            })
                            .unwrap_or(dep.as_str());
                        if installed.is_some() {
                            badge(ui, label, THEME.bg_dark, THEME.text_secondary).on_hover_text(dep);
                        } else {
                            tag(ui, label, THEME.accent_rose).on_hover_text(dep);
                            if let Some(ws_id) = app.workshop_id_for(dep, None, None) {
                                if ui
                                    .add_enabled_ui(!app.steam_busy(), |ui| {
                                        small_button(
                                            ui,
                                            app.i18n.get(TranslationKey::ModsDepDownload),
                                            ButtonStyle::Primary,
                                        )
                                    })
                                    .inner
                                    .on_hover_text(format!("{label} (#{ws_id})"))
                                    .clicked()
                                {
                                    dep_download_ws_id = Some(ws_id);
                                }
                            }
                        }
                    }
                });
            }
            if let Some(ws_id) = dep_download_ws_id {
                app.subscribe_steam_mod(&ws_id);
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
                }); // close text column
            }); // close horizontal
        } else {
            // No mod selected: counts + reminder ("Mod list summary")
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
        ModTier::Tier0Prepatcher => (
            "T0 Prepatcher",
            Color32::from_rgb(88, 28, 135),
            Color32::from_rgb(216, 180, 254),
        ),
        ModTier::Tier1CoreOfficial => (
            "T1 Core/DLC",
            Color32::from_rgb(180, 83, 9),
            Color32::from_rgb(253, 230, 138),
        ),
        ModTier::Tier2Frameworks => (
            "T2 Framework",
            Color32::from_rgb(30, 58, 138),
            Color32::from_rgb(191, 219, 254),
        ),
        ModTier::Tier3Overhauls => (
            "T3 Overhaul",
            Color32::from_rgb(127, 29, 29),
            Color32::from_rgb(254, 202, 202),
        ),
        ModTier::Tier4FactionsRaces => (
            "T4 Faction",
            Color32::from_rgb(19, 78, 74),
            Color32::from_rgb(153, 246, 228),
        ),
        ModTier::Tier5ContentItems => (
            "T5 Items",
            Color32::from_rgb(31, 41, 55),
            Color32::from_rgb(209, 213, 219),
        ),
        ModTier::Tier6BiomesAnimals => (
            "T6 Biomes",
            Color32::from_rgb(20, 83, 45),
            Color32::from_rgb(187, 247, 208),
        ),
        ModTier::Tier7UiCamera => (
            "T7 UI",
            Color32::from_rgb(67, 56, 202),
            Color32::from_rgb(224, 231, 255),
        ),
        ModTier::Tier8QolBehaviors => (
            "T8 QoL",
            Color32::from_rgb(133, 77, 14),
            Color32::from_rgb(254, 240, 138),
        ),
        ModTier::Tier9Patches => (
            "T9 Patch",
            Color32::from_rgb(76, 29, 149),
            Color32::from_rgb(233, 213, 255),
        ),
        ModTier::Tier10Translations => (
            "T10 Trans",
            Color32::from_rgb(131, 24, 67),
            Color32::from_rgb(251, 207, 232),
        ),
        ModTier::Tier11Optimizers => (
            "T11 Opt",
            Color32::from_rgb(6, 78, 59),
            Color32::from_rgb(167, 243, 208),
        ),
    };
    badge(ui, label, bg, fg);
}
