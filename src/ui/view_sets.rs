//! "Modpacks" tab: mod sets as cards on the left, the selected one's details, actions and
//! mods on the right. The sets are the game's own ModLists files.

use eframe::egui::{self, Color32, Rounding, Sense, Stroke, Ui, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::{tint, THEME};
use crate::app::{App, AppTab, ModListDialog};
use crate::i18n::{fmt, TranslationKey};
use crate::modlists::ModList;

/// What the user clicked; applied after drawing.
enum SetAction {
    Select(String),
    Apply {
        name: String,
        launch: bool,
    },
    OpenInMods(String),
    SaveHere(String),
    Duplicate(String),
    Dialog(ModListDialog),
    OpenFolder,
    SubscribeMissing(String),
    /// D1: the set's missing mods with no link (or just this one) -> "find by name" window.
    FindByName(String, Option<String>),
    DownloadWorkshopDb,
    Rescan,
    Import(std::path::PathBuf),
    Export {
        name: String,
        target: std::path::PathBuf,
    },
    Copied(String),
}

const CARD_SIZE: Vec2 = Vec2::new(168.0, 132.0);

pub fn render_sets_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    section_header(
        ui,
        t.get(TranslationKey::SetsTitle),
        t.get(TranslationKey::SetsPickerHint),
        None,
    );
    ui.add_space(8.0);

    // Something is always open on the right: the loaded set, else the first one.
    if app.selected_modlist.is_none() {
        app.selected_modlist = app
            .current_modlist
            .clone()
            .or_else(|| app.modlists.first().map(|l| l.name.clone()));
    }

    let mut actions: Vec<SetAction> = Vec::new();

    // Narrow window: the cards above, the selected modpack below (scrolling).
    if ui.available_width() < super::components::NARROW_TWO_COLUMNS {
        let width = ui.available_width();
        let list_h = (ui.available_height() * 0.4).max(150.0);
        ui.allocate_ui_with_layout(
            Vec2::new(width, list_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width);
                ui.set_max_height(list_h);
                render_cards(app, ui, &mut actions);
            },
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("sets_details_narrow")
            .auto_shrink([false, false])
            .show(ui, |ui| render_details(app, ui, &mut actions));
        apply_actions(app, actions);
        return;
    }

    let gap = 16.0;
    let left_w = ((ui.available_width() - gap) * 0.5).floor().max(300.0);
    let right_w = (ui.available_width() - gap - left_w).max(300.0);
    let height = ui.available_height();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            Vec2::new(left_w, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(left_w);
                render_cards(app, ui, &mut actions);
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
fn apply_actions(app: &mut App, actions: Vec<SetAction>) {
    let t = app.i18n;
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
            SetAction::SubscribeMissing(name) => app.subscribe_modlist_missing(&name),
            SetAction::FindByName(name, only) => {
                let wanted = app
                    .modlists
                    .iter()
                    .find(|l| l.name == name)
                    .map(|list| app.modlist_wanted(list, only.as_deref()))
                    .unwrap_or_default();
                app.start_name_search(name, wanted);
            }
            SetAction::Rescan => app.rescan_mods(),
            SetAction::DownloadWorkshopDb => app.download_workshop_index(),
            SetAction::Import(file) => app.import_modlist(&file),
            SetAction::Export { name, target } => app.export_modlist(&name, &target),
            SetAction::Copied(name) => app.status_message = fmt(t.get(TranslationKey::SetsCopied), &[&name]),
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
        let import_label = t.get(TranslationKey::SetsBtnImport);
        let buttons_w = styled_button_width(ui, true, new_label) + styled_button_width(ui, true, import_label) + 16.0;
        let search_w = (ui.available_width() - buttons_w).max(140.0);
        search_input_width(ui, &mut app.sets_search, t.get(TranslationKey::SetsSearch), search_w);
        if primary_button_icon(ui, ICON_PLUS, new_label).clicked() {
            let default = fmt(
                t.get(TranslationKey::SetsDefaultName),
                &[&(app.modlists.len() + 1).to_string()],
            );
            actions.push(SetAction::Dialog(ModListDialog::SaveAs {
                name: default,
                error: None,
            }));
        }
        if secondary_button_icon(ui, ICON_FOLDER, import_label)
            .on_hover_text(t.get(TranslationKey::SetsImportHint))
            .clicked()
        {
            if let Some(file) = rfd::FileDialog::new()
                .add_filter(t.get(TranslationKey::SetsImportFilter), &["rml", "xml"])
                .pick_file()
            {
                actions.push(SetAction::Import(file));
            }
        }
    });
    ui.add_space(10.0);
    let app: &App = app;

    if app.modlists.is_empty() {
        card_container(ui, |ui| {
            ui.label(
                egui::RichText::new(t.get(TranslationKey::SetsNoneYet))
                    .color(THEME.text_secondary)
                    .strong(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(t.get(TranslationKey::SetsHint))
                        .color(THEME.text_muted)
                        .size(12.0),
                )
                .wrap(),
            );
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
                for list in app
                    .modlists
                    .iter()
                    .filter(|l| query.is_empty() || l.name.to_lowercase().contains(&query))
                {
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
    painter.rect(rect, Rounding::same(8.0), fill, stroke);

    // Letter square
    let color = card_color(&list.name);
    let square = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 34.0), Vec2::splat(44.0));
    painter.rect_filled(square, Rounding::same(8.0), tint(color));
    let letter: String = list
        .name
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    painter.text(
        square.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::proportional(22.0),
        color,
    );

    // Name (cut with "…" if long)
    let name_galley = painter.layout(
        list.name.clone(),
        egui::FontId::proportional(13.0),
        THEME.text_primary,
        rect.width() - 16.0,
    );
    let name_pos = egui::pos2(
        rect.center().x - name_galley.size().x.min(rect.width() - 16.0) / 2.0,
        rect.top() + 64.0,
    );
    painter
        .with_clip_rect(rect.shrink(6.0))
        .galley(name_pos, name_galley, THEME.text_primary);

    // Mod count and game version
    let version = list
        .game_version
        .as_deref()
        .and_then(crate::game_install::major_minor)
        .unwrap_or_default();
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
            (
                fmt(t.get(TranslationKey::SetsMissingShort), &[&missing.to_string()]),
                THEME.accent_rose,
            )
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
            ui.add(
                egui::Label::new(egui::RichText::new(t.get(TranslationKey::SetsSelectHint)).color(THEME.text_muted))
                    .wrap(),
            );
            ui.add_space(6.0);
            if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SetsOpenFolder)).clicked() {
                actions.push(SetAction::OpenFolder);
            }
        });
        return;
    };
    let name = list.name.clone();

    ui.label(
        egui::RichText::new(&list.name)
            .color(THEME.accent_cyan)
            .size(18.0)
            .strong(),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(list.path.display().to_string())
                .color(THEME.text_muted)
                .size(11.0),
        )
        .truncate(),
    );
    ui.add_space(8.0);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        if success_button_icon(ui, ICON_CHECK_ALL, t.get(TranslationKey::SetsBtnApply))
            .on_hover_text(t.get(TranslationKey::SetsApplyHint))
            .clicked()
        {
            actions.push(SetAction::Apply {
                name: name.clone(),
                launch: false,
            });
        }
        if primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::SetsBtnApplyPlay)).clicked() {
            actions.push(SetAction::Apply {
                name: name.clone(),
                launch: true,
            });
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
            actions.push(SetAction::Dialog(ModListDialog::Rename {
                from: name.clone(),
                name: name.clone(),
                error: None,
            }));
        }
        if secondary_button_icon(ui, ICON_SAVE, t.get(TranslationKey::SetsBtnExport))
            .on_hover_text(t.get(TranslationKey::SetsExportHint))
            .clicked()
        {
            if let Some(target) = rfd::FileDialog::new()
                .set_file_name(format!("{name}.rml"))
                .add_filter("RimWorld mod list", &["rml"])
                .save_file()
            {
                actions.push(SetAction::Export {
                    name: name.clone(),
                    target,
                });
            }
        }
        if secondary_button_icon(ui, ICON_COPY, t.get(TranslationKey::SetsBtnCopyText))
            .on_hover_text(t.get(TranslationKey::SetsCopyTextHint))
            .clicked()
        {
            let text = app.modlist_share_text(list);
            ui.output_mut(|o| o.copied_text = text);
            actions.push(SetAction::Copied(name.clone()));
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
        ui.label(
            egui::RichText::new(t.get(TranslationKey::SetsDetails))
                .color(THEME.accent_cyan)
                .strong(),
        );
        ui.add_space(4.0);
        egui::Grid::new("set_details_grid")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                let row = |ui: &mut Ui, key: TranslationKey, value: String, color: Color32| {
                    ui.label(egui::RichText::new(t.get(key)).color(THEME.text_muted));
                    ui.label(egui::RichText::new(value).color(color));
                    ui.end_row();
                };
                row(
                    ui,
                    TranslationKey::SetsGameVersion,
                    list.game_version.clone().unwrap_or_else(|| "—".into()),
                    THEME.text_primary,
                );
                row(
                    ui,
                    TranslationKey::SetsModCount,
                    list.ids.len().to_string(),
                    THEME.text_primary,
                );
                row(
                    ui,
                    TranslationKey::SetsMissing,
                    missing.to_string(),
                    if missing > 0 {
                        THEME.accent_rose
                    } else {
                        THEME.text_primary
                    },
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

    // F6: mods of the set that are not installed
    if missing > 0 {
        let workshop = app.modlist_missing_workshop_ids(list).len();
        let manual = missing.saturating_sub(workshop);
        let by_name = app.modlist_wanted(list, None).len();
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(fmt(t.get(TranslationKey::SetsMissingTitle), &[&missing.to_string()]))
                    .color(THEME.accent_rose)
                    .strong(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(t.get(TranslationKey::SetsMissingHint))
                        .color(THEME.text_muted)
                        .size(12.0),
                )
                .wrap(),
            );
            // Mods without a Workshop link: the database finds most of them (D1).
            if workshop_links_hint(ui, t, manual, app.workshop_index.is_some(), app.workshop_index_busy()) {
                actions.push(SetAction::DownloadWorkshopDb);
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                let busy = app.steam_busy();
                let label = fmt(t.get(TranslationKey::SetsBtnSubscribeMissing), &[&workshop.to_string()]);
                let subscribe = ui
                    .add_enabled_ui(workshop > 0 && !busy, |ui| primary_button_icon(ui, ICON_STEAM, &label))
                    .inner;
                if subscribe.clicked() {
                    actions.push(SetAction::SubscribeMissing(name.clone()));
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
                    actions.push(SetAction::FindByName(name.clone(), None));
                }
                if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::BtnRescanMods)).clicked() {
                    actions.push(SetAction::Rescan);
                }
                if busy {
                    ui.spinner();
                }
            });
        });
    }

    // Mods of the set
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(fmt(
                t.get(TranslationKey::SetsModsTitle),
                &[&list.ids.len().to_string()],
            ))
            .color(THEME.accent_cyan)
            .strong(),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("set_mods_scroll")
            .auto_shrink([false, false])
            // Inside the narrow layout's scrolling the height is unlimited: keep it to the window.
            .max_height(
                ui.available_height()
                    .min(ui.ctx().screen_rect().height() * 0.7)
                    .max(120.0),
            )
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
                        ui.add(
                            egui::Label::new(egui::RichText::new(name).color(THEME.text_primary).size(12.0)).truncate(),
                        );
                        if found.is_none() {
                            tag(ui, t.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                            let link =
                                app.workshop_id_for(id, list.steam_id(id), list.names.get(i).map(String::as_str));
                            if let Some(steam_id) = link {
                                if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                                    .on_hover_text(t.get(TranslationKey::SetsOpenWorkshop))
                                    .clicked()
                                {
                                    open_external(crate::steam::workshop_url(&steam_id));
                                }
                            } else if !id.to_lowercase().starts_with("ludeon.rimworld")
                                && icon_button(ui, ICON_SEARCH, Vec2::splat(14.0), THEME.text_muted)
                                    .on_hover_text(t.get(TranslationKey::NameSearchRowHint))
                                    .clicked()
                            {
                                actions.push(SetAction::FindByName(list.name.clone(), Some(id.clone())));
                            }
                        }
                    });
                }
            });
    });
}
