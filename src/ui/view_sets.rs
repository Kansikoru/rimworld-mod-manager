//! "Modpacks" tab: mod sets as cards on the left, the selected one's details, actions and
//! mods on the right. The sets are the game's own ModLists files.

use eframe::egui::{self, Color32, Rounding, Sense, Stroke, Ui, Vec2};

use crate::app::{App, AppTab, ModListDialog};
use crate::i18n::{fmt, TranslationKey};
use crate::modlists::ModList;
use super::components::*;
use super::icons::*;
use super::theme::{tint, THEME};

/// What the user clicked; applied after drawing.
enum SetAction {
    Select(String),
    Apply { name: String, launch: bool },
    OpenInMods(String),
    SaveHere(String),
    Duplicate(String),
    Dialog(ModListDialog),
    OpenFolder,
}

const CARD_SIZE: Vec2 = Vec2::new(168.0, 132.0);

pub fn render_sets_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    section_header(ui, t.get(TranslationKey::SetsTitle), t.get(TranslationKey::SetsPickerHint), None);
    ui.add_space(8.0);

    // Something is always open on the right: the loaded set, else the first one.
    if app.selected_modlist.is_none() {
        app.selected_modlist = app.current_modlist.clone().or_else(|| app.modlists.first().map(|l| l.name.clone()));
    }

    let mut actions: Vec<SetAction> = Vec::new();
    let gap = 16.0;
    let left_w = ((ui.available_width() - gap) * 0.5).floor().max(300.0);
    let right_w = (ui.available_width() - gap - left_w).max(300.0);
    let height = ui.available_height();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(Vec2::new(left_w, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(left_w);
            render_cards(app, ui, &mut actions);
        });
        ui.allocate_ui_with_layout(Vec2::new(right_w, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(right_w);
            render_details(app, ui, &mut actions);
        });
    });

    for action in actions {
        match action {
            SetAction::Select(name) => app.selected_modlist = Some(name),
            SetAction::Apply { name, launch } => app.apply_modlist(&name, launch),
            SetAction::OpenInMods(name) => {
                app.load_modlist(&name);
                app.current_tab = AppTab::Mods;
            }
            SetAction::SaveHere(name) => app.overwrite_modlist(&name),
            SetAction::Duplicate(name) => app.duplicate_modlist(&name),
            SetAction::Dialog(dialog) => app.modlist_dialog = Some(dialog),
            SetAction::OpenFolder => {
                if let Some(dir) = app.modlists_folder() {
                    let _ = std::fs::create_dir_all(&dir);
                    open_external(dir);
                }
            }
        }
    }
}

/// A steady color per name, for the letter square of a card.
fn card_color(name: &str) -> Color32 {
    let palette = [
        THEME.accent_purple,
        THEME.accent_cyan,
        THEME.accent_emerald,
        THEME.accent_rose,
        THEME.accent_amber,
        THEME.accent_info,
    ];
    let sum: usize = name.bytes().map(usize::from).sum();
    palette[sum % palette.len()]
}

fn render_cards(app: &mut App, ui: &mut Ui, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let new_label = t.get(TranslationKey::SetsNewFromCurrent);
        let button_w = styled_button_width(ui, true, new_label);
        let search_w = (ui.available_width() - button_w - 8.0).max(140.0);
        search_input_width(ui, &mut app.sets_search, t.get(TranslationKey::SetsSearch), search_w);
        if primary_button_icon(ui, ICON_PLUS, new_label).clicked() {
            let default = fmt(t.get(TranslationKey::SetsDefaultName), &[&(app.modlists.len() + 1).to_string()]);
            actions.push(SetAction::Dialog(ModListDialog::SaveAs { name: default, error: None }));
        }
    });
    ui.add_space(10.0);
    let app: &App = app;

    if app.modlists.is_empty() {
        card_container(ui, |ui| {
            ui.label(egui::RichText::new(t.get(TranslationKey::SetsNoneYet)).color(THEME.text_secondary).strong());
            ui.add(egui::Label::new(egui::RichText::new(t.get(TranslationKey::SetsHint)).color(THEME.text_muted).size(12.0)).wrap());
        });
        return;
    }

    let query = app.sets_search.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("sets_cards_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0);
                for list in app.modlists.iter().filter(|l| query.is_empty() || l.name.to_lowercase().contains(&query)) {
                    let selected = app.selected_modlist.as_deref() == Some(list.name.as_str());
                    if set_card(app, ui, list, selected).clicked() {
                        actions.push(SetAction::Select(list.name.clone()));
                    }
                }
            });
        });
}

fn set_card(app: &App, ui: &mut Ui, list: &ModList, selected: bool) -> egui::Response {
    let t = app.i18n;
    let (rect, response) = ui.allocate_exact_size(CARD_SIZE, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let fill = if selected || response.hovered() { THEME.bg_surface_hover } else { THEME.bg_card };
    let stroke = if selected { Stroke::new(1.5_f32, THEME.accent_cyan) } else { Stroke::new(1.0_f32, THEME.border_subtle) };
    painter.rect(rect, Rounding::same(8.0), fill, stroke);

    // Letter square
    let color = card_color(&list.name);
    let square = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 34.0), Vec2::splat(44.0));
    painter.rect_filled(square, Rounding::same(8.0), tint(color));
    let letter: String = list.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    painter.text(square.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(22.0), color);

    // Name (cut with "…" if long)
    let name_galley = painter.layout(
        list.name.clone(),
        egui::FontId::proportional(13.0),
        THEME.text_primary,
        rect.width() - 16.0,
    );
    let name_pos = egui::pos2(rect.center().x - name_galley.size().x.min(rect.width() - 16.0) / 2.0, rect.top() + 64.0);
    painter.with_clip_rect(rect.shrink(6.0)).galley(name_pos, name_galley, THEME.text_primary);

    // Mod count and game version
    let version = list.game_version.as_deref().and_then(crate::game_install::major_minor).unwrap_or_default();
    let mut line = fmt(t.get(TranslationKey::SetsModsCount), &[&list.ids.len().to_string()]);
    if !version.is_empty() {
        line.push_str(&format!("  ·  {version}"));
    }
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 30.0),
        egui::Align2::CENTER_CENTER,
        line,
        egui::FontId::proportional(11.0),
        THEME.text_muted,
    );

    // State: the set the game loads now, or mods missing on disk
    let (badge, badge_color) = if app.modlist_diff(list).is_same() {
        (t.get(TranslationKey::SetsActiveBadge).to_string(), THEME.accent_emerald)
    } else {
        let missing = app.modlist_missing(list);
        if missing > 0 {
            (fmt(t.get(TranslationKey::SetsMissingShort), &[&missing.to_string()]), THEME.accent_rose)
        } else {
            (String::new(), THEME.text_muted)
        }
    };
    if !badge.is_empty() {
        painter.text(
            egui::pos2(rect.center().x, rect.bottom() - 13.0),
            egui::Align2::CENTER_CENTER,
            badge,
            egui::FontId::proportional(11.0),
            badge_color,
        );
    }
    response
}

fn render_details(app: &App, ui: &mut Ui, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    let Some(list) = app
        .selected_modlist
        .as_deref()
        .and_then(|name| app.modlists.iter().find(|l| l.name == name))
    else {
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(t.get(TranslationKey::SetsSelectHint)).color(THEME.text_muted)).wrap());
            ui.add_space(6.0);
            if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SetsOpenFolder)).clicked() {
                actions.push(SetAction::OpenFolder);
            }
        });
        return;
    };
    let name = list.name.clone();

    ui.label(egui::RichText::new(&list.name).color(THEME.accent_cyan).size(18.0).strong());
    ui.add(egui::Label::new(egui::RichText::new(list.path.display().to_string()).color(THEME.text_muted).size(11.0)).truncate());
    ui.add_space(8.0);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        if success_button_icon(ui, ICON_CHECK_ALL, t.get(TranslationKey::SetsBtnApply))
            .on_hover_text(t.get(TranslationKey::SetsApplyHint))
            .clicked()
        {
            actions.push(SetAction::Apply { name: name.clone(), launch: false });
        }
        if primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::SetsBtnApplyPlay)).clicked() {
            actions.push(SetAction::Apply { name: name.clone(), launch: true });
        }
        if secondary_button_icon(ui, ICON_MODS, t.get(TranslationKey::SetsBtnOpenInMods)).clicked() {
            actions.push(SetAction::OpenInMods(name.clone()));
        }
        if secondary_button_icon(ui, ICON_SAVE, t.get(TranslationKey::SetsBtnSaveHere)).clicked() {
            actions.push(SetAction::SaveHere(name.clone()));
        }
        if secondary_button_icon(ui, ICON_COPY, t.get(TranslationKey::SetsBtnDuplicate)).clicked() {
            actions.push(SetAction::Duplicate(name.clone()));
        }
        if secondary_button(ui, t.get(TranslationKey::SetsBtnRename)).clicked() {
            actions.push(SetAction::Dialog(ModListDialog::Rename { from: name.clone(), name: name.clone(), error: None }));
        }
        if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::ModsBtnFolder)).clicked() {
            actions.push(SetAction::OpenFolder);
        }
        if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::SetsBtnDelete)).clicked() {
            actions.push(SetAction::Dialog(ModListDialog::Delete { name: name.clone() }));
        }
    });
    ui.add_space(10.0);

    // Details
    let diff = app.modlist_diff(list);
    let missing = app.modlist_missing(list);
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(t.get(TranslationKey::SetsDetails)).color(THEME.accent_cyan).strong());
        ui.add_space(4.0);
        egui::Grid::new("set_details_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            let row = |ui: &mut Ui, key: TranslationKey, value: String, color: Color32| {
                ui.label(egui::RichText::new(t.get(key)).color(THEME.text_muted));
                ui.label(egui::RichText::new(value).color(color));
                ui.end_row();
            };
            row(ui, TranslationKey::SetsGameVersion, list.game_version.clone().unwrap_or_else(|| "—".into()), THEME.text_primary);
            row(ui, TranslationKey::SetsModCount, list.ids.len().to_string(), THEME.text_primary);
            row(
                ui,
                TranslationKey::SetsMissing,
                missing.to_string(),
                if missing > 0 { THEME.accent_rose } else { THEME.text_primary },
            );
            row(
                ui,
                TranslationKey::SetsModified,
                list.modified.map(local_date_time).unwrap_or_else(|| "—".into()),
                THEME.text_primary,
            );
            let (diff_text, diff_color) = if diff.is_same() {
                (t.get(TranslationKey::SetsDiffSame).to_string(), THEME.accent_emerald)
            } else {
                let mut parts = Vec::new();
                if diff.added > 0 {
                    parts.push(format!("+{}", diff.added));
                }
                if diff.removed > 0 {
                    parts.push(format!("−{}", diff.removed));
                }
                if diff.reordered {
                    parts.push(t.get(TranslationKey::SetsDiffOrder).to_string());
                }
                (parts.join(" · "), THEME.accent_amber)
            };
            row(ui, TranslationKey::SetsDiff, diff_text, diff_color);
        });
    });

    // Mods of the set
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(fmt(t.get(TranslationKey::SetsModsTitle), &[&list.ids.len().to_string()]))
                .color(THEME.accent_cyan)
                .strong(),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("set_mods_scroll")
            .auto_shrink([false, false])
            .max_height(ui.available_height().max(120.0))
            .show_rows(ui, 22.0, list.ids.len(), |ui, range| {
                for i in range {
                    let id = &list.ids[i];
                    let found = app.find_mod(id);
                    let name = found
                        .map(|m| m.name.clone())
                        .or_else(|| list.names.get(i).cloned())
                        .unwrap_or_else(|| id.clone());
                    ui.horizontal(|ui| {
                        ui.set_height(20.0);
                        // Numbers in a fixed right-aligned column, so names line up past row 9.
                        let (num_rect, _) = ui.allocate_exact_size(Vec2::new(30.0, 20.0), Sense::hover());
                        ui.painter().text(
                            num_rect.right_center(),
                            egui::Align2::RIGHT_CENTER,
                            format!("{}.", i + 1),
                            egui::FontId::proportional(12.0),
                            THEME.text_muted,
                        );
                        ui.add(egui::Label::new(egui::RichText::new(name).color(THEME.text_primary).size(12.0)).truncate());
                        if found.is_none() {
                            tag(ui, t.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                        }
                    });
                }
            });
    });
}
