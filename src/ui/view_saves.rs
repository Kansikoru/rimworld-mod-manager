//! "Saves" tab (F7): the game's saves on the left; for the selected one, how its mods
//! compare with the current load order, and buttons to load them as in the save.

use eframe::egui::{self, Color32, Rounding, Sense, Stroke, Ui, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::{App, AppTab};
use crate::i18n::{fmt, TranslationKey};
use crate::saves::SaveInfo;

enum SaveAction {
    Select(String),
    LoadMods(String),
    ToModlist(String),
    SubscribeMissing(String),
    /// D1: the save's missing mods with no link (or just this one) -> "find by name" window.
    FindByName(String, Option<String>),
    Refresh,
    OpenFolder,
    DownloadWorkshopDb,
}

/// What a row of the save's mod list shows next to the name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModState {
    Enabled,
    Disabled,
    Missing,
    Extra,
}

const ROW_H: f32 = 52.0;

pub fn render_saves_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    app.ensure_saves_loaded();
    section_header(
        ui,
        t.get(TranslationKey::SavesTitle),
        t.get(TranslationKey::SavesHint),
        None,
    );
    ui.add_space(8.0);

    if app.selected_save.is_none() {
        app.selected_save = app.saves.first().map(|s| s.name.clone());
    }

    let mut actions: Vec<SaveAction> = Vec::new();

    // Narrow window: the saves above, the selected one below (scrolling).
    if ui.available_width() < super::components::NARROW_TWO_COLUMNS {
        let width = ui.available_width();
        let list_h = (ui.available_height() * 0.4).max(150.0);
        ui.allocate_ui_with_layout(
            Vec2::new(width, list_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width);
                ui.set_max_height(list_h);
                render_list(app, ui, &mut actions);
            },
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("saves_details_narrow")
            .auto_shrink([false, false])
            .show(ui, |ui| render_details(app, ui, &mut actions));
        apply_actions(app, actions);
        return;
    }

    let gap = 16.0;
    let left_w = ((ui.available_width() - gap) * 0.42).floor().max(280.0);
    let right_w = (ui.available_width() - gap - left_w).max(300.0);
    let height = ui.available_height();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            Vec2::new(left_w, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(left_w);
                render_list(app, ui, &mut actions);
            },
        );
        ui.allocate_ui_with_layout(
            Vec2::new(right_w, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(right_w);
                render_details(app, ui, &mut actions);
            },
        );
    });
    apply_actions(app, actions);
}

/// Runs what the user clicked (after drawing, when `app` is free to change).
fn apply_actions(app: &mut App, actions: Vec<SaveAction>) {
    for action in actions {
        match action {
            SaveAction::Select(name) => app.selected_save = Some(name),
            SaveAction::LoadMods(name) => {
                app.load_save_mods(&name);
                app.current_tab = AppTab::Mods;
            }
            SaveAction::ToModlist(name) => {
                app.save_to_modlist(&name);
                if app.tab_visible(AppTab::Sets) {
                    app.current_tab = AppTab::Sets;
                }
            }
            SaveAction::SubscribeMissing(name) => app.subscribe_save_missing(&name),
            SaveAction::FindByName(name, only) => {
                let wanted = app
                    .saves
                    .iter()
                    .find(|s| s.name == name)
                    .map(|save| app.save_wanted(save, only.as_deref()))
                    .unwrap_or_default();
                app.start_name_search(name, wanted);
            }
            SaveAction::Refresh => app.refresh_saves(),
            SaveAction::OpenFolder => {
                if let Some(dir) = app.saves_folder() {
                    open_external(dir);
                }
            }
            SaveAction::DownloadWorkshopDb => app.download_workshop_index(),
        }
    }
}

fn render_list(app: &mut App, ui: &mut Ui, actions: &mut Vec<SaveAction>) {
    let t = app.i18n;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let refresh_w = styled_button_width(ui, true, "");
        let search_w = (ui.available_width() - refresh_w - 8.0).max(140.0);
        search_input_width(ui, &mut app.saves_search, t.get(TranslationKey::SavesSearch), search_w);
        if styled_button(ui, Some(ICON_REFRESH), "", ButtonStyle::Secondary)
            .on_hover_text(t.get(TranslationKey::SavesRefresh))
            .clicked()
        {
            actions.push(SaveAction::Refresh);
        }
    });
    ui.add_space(10.0);
    let app: &App = app;

    if app.saves.is_empty() {
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            if app.saves_loading() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new(t.get(TranslationKey::SavesLoading)).color(THEME.text_muted));
                });
            } else {
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::SavesNone))
                        .color(THEME.text_secondary)
                        .strong(),
                );
            }
        });
        return;
    }

    let query = app.saves_search.trim().to_lowercase();
    let shown: Vec<&SaveInfo> = app
        .saves
        .iter()
        .filter(|s| query.is_empty() || s.name.to_lowercase().contains(&query))
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("saves_list_scroll")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, shown.len(), |ui, range| {
            for save in &shown[range] {
                let selected = app.selected_save.as_deref() == Some(save.name.as_str());
                if save_row(app, ui, save, selected).clicked() {
                    actions.push(SaveAction::Select(save.name.clone()));
                }
            }
        });
}

fn save_row(app: &App, ui: &mut Ui, save: &SaveInfo, selected: bool) -> egui::Response {
    let t = app.i18n;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H - 6.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let fill = if selected || response.hovered() {
        THEME.bg_surface_hover
    } else {
        THEME.bg_card
    };
    let stroke = if selected {
        Stroke::new(1.5_f32, THEME.accent_cyan)
    } else {
        Stroke::new(1.0_f32, THEME.border_subtle)
    };
    painter.rect(rect, Rounding::same(6.0), fill, stroke);

    let left = rect.left() + 12.0;

    // Right side first: what stops this save from loading as it was. The name is then
    // cut with "…" so it never runs under it.
    let missing = save.ids.iter().filter(|id| app.find_mod(id).is_none()).count();
    let (badge, color) = if missing > 0 {
        (
            fmt(t.get(TranslationKey::SetsMissingShort), &[&missing.to_string()]),
            THEME.accent_rose,
        )
    } else if app.save_other_version(save) {
        (
            t.get(TranslationKey::SavesOtherVersionShort).to_string(),
            THEME.accent_amber,
        )
    } else {
        (
            t.get(TranslationKey::SavesAllInstalled).to_string(),
            THEME.accent_emerald,
        )
    };
    let badge_rect = painter.text(
        egui::pos2(rect.right() - 12.0, rect.top() + 14.0),
        egui::Align2::RIGHT_CENTER,
        badge,
        egui::FontId::proportional(11.0),
        color,
    );

    let mut job = egui::text::LayoutJob::simple_singleline(
        save.name.clone(),
        egui::FontId::proportional(13.5),
        THEME.text_primary,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (badge_rect.left() - 10.0 - left).max(40.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = painter.layout_job(job);
    let name_pos = egui::pos2(left, rect.top() + 14.0 - galley.size().y / 2.0);
    painter.galley(name_pos, galley, THEME.text_primary);
    let mut line = save.modified.map(local_date_time).unwrap_or_default();
    if let Some(v) = save.game_version.as_deref().and_then(crate::game_install::major_minor) {
        line.push_str(&format!("  ·  {v}"));
    }
    line.push_str(&format!(
        "  ·  {}",
        fmt(t.get(TranslationKey::SetsModsCount), &[&save.ids.len().to_string()])
    ));
    painter.text(
        egui::pos2(left, rect.bottom() - 13.0),
        egui::Align2::LEFT_CENTER,
        line,
        egui::FontId::proportional(11.0),
        THEME.text_muted,
    );
    response
}

fn render_details(app: &App, ui: &mut Ui, actions: &mut Vec<SaveAction>) {
    let t = app.i18n;
    let Some(save) = app
        .selected_save
        .as_deref()
        .and_then(|name| app.saves.iter().find(|s| s.name == name))
    else {
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(egui::RichText::new(t.get(TranslationKey::SavesSelectHint)).color(THEME.text_muted))
                    .wrap(),
            );
            ui.add_space(6.0);
            if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::ModsBtnFolder)).clicked() {
                actions.push(SaveAction::OpenFolder);
            }
        });
        return;
    };
    let name = save.name.clone();
    let cmp = app.save_compare(save);
    let workshop_missing = app.save_missing_workshop_ids(save).len();
    let by_name = app.save_wanted(save, None).len();

    ui.label(
        egui::RichText::new(&save.name)
            .color(THEME.accent_cyan)
            .size(18.0)
            .strong(),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(save.path.display().to_string())
                .color(THEME.text_muted)
                .size(11.0),
        )
        .truncate(),
    );
    ui.add_space(8.0);

    if app.save_other_version(save) {
        let installed = app.steam_paths.game_version.clone().unwrap_or_default();
        let saved = save.game_version.clone().unwrap_or_default();
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(fmt(t.get(TranslationKey::SavesOtherVersion), &[&saved, &installed]))
                        .color(THEME.accent_amber),
                )
                .wrap(),
            );
        });
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        if success_button_icon(ui, ICON_CHECK_ALL, t.get(TranslationKey::SavesBtnLoad))
            .on_hover_text(t.get(TranslationKey::SavesBtnLoadHint))
            .clicked()
        {
            actions.push(SaveAction::LoadMods(name.clone()));
        }
        if secondary_button_icon(ui, ICON_SAVE, t.get(TranslationKey::SavesBtnToSet))
            .on_hover_text(t.get(TranslationKey::SavesBtnToSetHint))
            .clicked()
        {
            actions.push(SaveAction::ToModlist(name.clone()));
        }
        if workshop_missing > 0 {
            let label = fmt(
                t.get(TranslationKey::SetsBtnSubscribeMissing),
                &[&workshop_missing.to_string()],
            );
            let busy = app.steam_busy();
            if ui
                .add_enabled_ui(!busy, |ui| primary_button_icon(ui, ICON_STEAM, &label))
                .inner
                .clicked()
            {
                actions.push(SaveAction::SubscribeMissing(name.clone()));
            }
            if busy {
                ui.spinner();
            }
        }
        if by_name > 0
            && secondary_button_icon(
                ui,
                ICON_SEARCH,
                &fmt(t.get(TranslationKey::NameSearchBtn), &[&by_name.to_string()]),
            )
            .on_hover_text(t.get(TranslationKey::NameSearchBtnHint))
            .clicked()
        {
            actions.push(SaveAction::FindByName(name.clone(), None));
        }
        if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::ModsBtnFolder)).clicked() {
            actions.push(SaveAction::OpenFolder);
        }
    });
    // Missing mods the save gave no Workshop link for (the game writes ID 0 for many).
    let no_link = cmp.missing.len().saturating_sub(workshop_missing);
    if workshop_links_hint(ui, t, no_link, app.workshop_index.is_some(), app.workshop_index_busy()) {
        actions.push(SaveAction::DownloadWorkshopDb);
    }
    ui.add_space(10.0);

    // Details and the comparison with the current load order
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(t.get(TranslationKey::SetsDetails))
                .color(THEME.accent_cyan)
                .strong(),
        );
        ui.add_space(4.0);
        egui::Grid::new("save_details_grid")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                let row = |ui: &mut Ui, label: &str, value: String, color: Color32| {
                    ui.label(egui::RichText::new(label).color(THEME.text_muted));
                    ui.label(egui::RichText::new(value).color(color));
                    ui.end_row();
                };
                let num = |n: usize, bad: Color32| if n > 0 { bad } else { THEME.text_primary };
                row(
                    ui,
                    t.get(TranslationKey::SetsGameVersion),
                    save.game_version.clone().unwrap_or_else(|| "—".into()),
                    THEME.text_primary,
                );
                row(
                    ui,
                    t.get(TranslationKey::SavesSaved),
                    save.modified.map(local_date_time).unwrap_or_else(|| "—".into()),
                    THEME.text_primary,
                );
                row(
                    ui,
                    t.get(TranslationKey::SavesSize),
                    format!("{:.1} MB", save.size as f64 / 1_048_576.0),
                    THEME.text_primary,
                );
                row(
                    ui,
                    t.get(TranslationKey::SetsModCount),
                    save.ids.len().to_string(),
                    THEME.text_primary,
                );
                row(
                    ui,
                    t.get(TranslationKey::SavesEnabled),
                    cmp.enabled.len().to_string(),
                    THEME.text_primary,
                );
                row(
                    ui,
                    t.get(TranslationKey::SavesDisabled),
                    cmp.disabled.len().to_string(),
                    num(cmp.disabled.len(), THEME.accent_amber),
                );
                row(
                    ui,
                    t.get(TranslationKey::SetsMissing),
                    cmp.missing.len().to_string(),
                    num(cmp.missing.len(), THEME.accent_rose),
                );
                row(
                    ui,
                    t.get(TranslationKey::SavesExtra),
                    cmp.extra.len().to_string(),
                    num(cmp.extra.len(), THEME.accent_amber),
                );
            });
        ui.add_space(4.0);
        let (summary, color) = if cmp.matches() {
            (t.get(TranslationKey::SavesMatches), THEME.accent_emerald)
        } else if cmp.reordered && cmp.disabled.is_empty() && cmp.missing.is_empty() && cmp.extra.is_empty() {
            (t.get(TranslationKey::SavesOnlyOrder), THEME.accent_amber)
        } else {
            (t.get(TranslationKey::SavesDiffers), THEME.accent_amber)
        };
        ui.add(egui::Label::new(egui::RichText::new(summary).color(color).size(12.0)).wrap());
    });

    // The save's mods, then the enabled mods the save doesn't have
    // (name, state, Workshop link, package ID for "find by name" when there's no link)
    type Row = (String, ModState, Option<String>, Option<String>);
    let mut rows: Vec<Row> = Vec::with_capacity(save.ids.len() + cmp.extra.len());
    let enabled: std::collections::HashSet<String> = cmp.enabled.iter().map(|s| s.to_lowercase()).collect();
    let disabled: std::collections::HashSet<String> = cmp.disabled.iter().map(|s| s.to_lowercase()).collect();
    for id in &save.ids {
        let lower = id.to_lowercase();
        let state = if enabled.contains(&lower) {
            ModState::Enabled
        } else if disabled.contains(&lower) {
            ModState::Disabled
        } else {
            ModState::Missing
        };
        let mod_name = app
            .find_mod(id)
            .map(|m| m.name.clone())
            .or_else(|| save.name_of(id).map(str::to_string))
            .unwrap_or_else(|| id.clone());
        let link = (state == ModState::Missing)
            .then(|| app.workshop_id_for(id, save.steam_id(id), save.name_of(id)))
            .flatten();
        let findable =
            (state == ModState::Missing && link.is_none() && !id.to_lowercase().starts_with("ludeon.rimworld"))
                .then(|| id.clone());
        rows.push((mod_name, state, link, findable));
    }
    for id in &cmp.extra {
        let mod_name = app.find_mod(id).map(|m| m.name.clone()).unwrap_or_else(|| id.clone());
        rows.push((mod_name, ModState::Extra, None, None));
    }

    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(fmt(
                t.get(TranslationKey::SavesModsTitle),
                &[&save.ids.len().to_string()],
            ))
            .color(THEME.accent_cyan)
            .strong(),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt(("save_mods_scroll", &save.name))
            .auto_shrink([false, false])
            // Inside the narrow layout's scrolling the height is unlimited: keep it to the window.
            .max_height(
                ui.available_height()
                    .min(ui.ctx().screen_rect().height() * 0.7)
                    .max(120.0),
            )
            .show_rows(ui, 22.0, rows.len(), |ui, range| {
                for i in range {
                    let (mod_name, state, steam_id, findable) = &rows[i];
                    ui.horizontal(|ui| {
                        ui.set_height(20.0);
                        let (num_rect, _) = ui.allocate_exact_size(Vec2::new(30.0, 20.0), Sense::hover());
                        if *state != ModState::Extra {
                            ui.painter().text(
                                num_rect.right_center(),
                                egui::Align2::RIGHT_CENTER,
                                format!("{}.", i + 1),
                                egui::FontId::proportional(12.0),
                                THEME.text_muted,
                            );
                        }
                        let text_color = if *state == ModState::Extra {
                            THEME.text_muted
                        } else {
                            THEME.text_primary
                        };
                        ui.add(egui::Label::new(egui::RichText::new(mod_name).color(text_color).size(12.0)).truncate());
                        match state {
                            ModState::Enabled => {}
                            ModState::Disabled => {
                                tag(ui, t.get(TranslationKey::SavesTagDisabled), THEME.accent_amber);
                            }
                            ModState::Missing => {
                                tag(ui, t.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                                if let Some(steam_id) = steam_id {
                                    if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                                        .on_hover_text(t.get(TranslationKey::SetsOpenWorkshop))
                                        .clicked()
                                    {
                                        open_external(crate::steam::workshop_url(steam_id));
                                    }
                                }
                                if let Some(id) = findable {
                                    if icon_button(ui, ICON_SEARCH, Vec2::splat(14.0), THEME.text_muted)
                                        .on_hover_text(t.get(TranslationKey::NameSearchRowHint))
                                        .clicked()
                                    {
                                        actions.push(SaveAction::FindByName(name.clone(), Some(id.clone())));
                                    }
                                }
                            }
                            ModState::Extra => {
                                tag(ui, t.get(TranslationKey::SavesTagExtra), THEME.text_muted);
                            }
                        }
                    });
                }
            });
    });
}
