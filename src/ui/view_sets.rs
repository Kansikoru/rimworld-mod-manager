//! "Modpacks" tab: mod sets as cards on the left, the selected one's details, actions and
//! mods on the right. The sets are the game's own ModLists files.

use eframe::egui::{self, Color32, CornerRadius, Sense, Stroke, Ui, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::{tint, THEME};
use crate::app::{App, AppTab, CompareWith, ModListDialog};
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
    SetPicture(String, std::path::PathBuf),
    RemovePicture(String),
    ToggleTag(String, String),
    /// Mod settings kept with the modpack: take a copy, put it back, forget it.
    SaveSettings(String),
    RestoreSettings(String),
    ForgetSettings(String),
    /// Open the "Compare" window, or change what the modpack is compared with.
    Compare(String, CompareWith),
    CloseCompare,
    /// A new set with the mods of both, in load order.
    Merge(String, String),
    /// Mod previews the column's rows want (lowercase id, preview file).
    LoadIcons(Vec<(String, Option<std::path::PathBuf>)>),
    /// Ask Steam whether these Workshop items still exist.
    CheckWorkshop(Vec<String>),
}

/// A tag as shown: built-in ones translated, own ones as typed.
fn tag_label(t: crate::i18n::I18n, tag: &str) -> String {
    let key = match tag {
        "stable" => TranslationKey::SetsTagStable,
        "test" => TranslationKey::SetsTagTest,
        "performance" => TranslationKey::SetsTagPerformance,
        "roleplay" => TranslationKey::SetsTagRoleplay,
        _ => return tag.to_string(),
    };
    t.get(key).to_string()
}

pub fn render_sets_view(app: &mut App, ui: &mut Ui) {
    app.load_modlist_pictures(ui.ctx());

    // Something is always open on the right: the loaded set, else the first one.
    if app.selected_modlist.is_none() {
        app.selected_modlist = app
            .current_modlist
            .clone()
            .or_else(|| app.modlists.first().map(|l| l.name.clone()));
    }

    let mut actions: Vec<SetAction> = Vec::new();

    // Like Settings: the modpacks as a list on the left, the chosen one on the right.
    egui::Panel::left("sets_list")
        .resizable(false)
        .default_size(LIST_W)
        .size_range(LIST_W..=LIST_W)
        .show_separator_line(false)
        .frame(
            // Raised like the cards; the outer margin leaves room for its shadow (the panel
            // clips at its own edge).
            egui::Frame {
                outer_margin: egui::Margin {
                    left: 0,
                    right: 12,
                    top: 0,
                    bottom: 12,
                },
                inner_margin: egui::Margin::same(12),
                ..raised_frame(10)
            },
        )
        .show(ui, |ui| render_list_panel(app, ui, &mut actions));

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 4,
            bottom: 4,
        }))
        .show(ui, |ui| {
            // A wide window: the modpack's mods in a full-height column on the right, the same
            // rows as the active list on the Mods tab; narrower, they stay under the details.
            let wide = ui.available_width() >= WIDE_WITH_MODS && !app.modlists.is_empty();
            if wide {
                if let Some(list) = selected_list(app) {
                    // The column takes what the details leave, so no empty strip between them.
                    let avail = ui.available_width();
                    let column_w = (avail - 12.0 - DETAILS_MAX_W)
                        .max(avail * 0.45)
                        .clamp(MODS_COLUMN_MIN_W, 1000.0);
                    egui::Panel::right("set_mods_column")
                        .resizable(false)
                        .default_size(column_w)
                        .size_range(column_w..=column_w)
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                            left: 12,
                            right: 0,
                            top: 0,
                            bottom: 12,
                        }))
                        .show(ui, |ui| render_set_mods_column(app, ui, list, &mut actions));
                }
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sets_details")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(DETAILS_MAX_W);
                        if app.modlists.is_empty() {
                            render_empty(app, ui, &mut actions);
                        } else {
                            render_details(app, ui, &mut actions, !wide);
                        }
                    });
            });
        });
    render_compare_window(app, ui.ctx(), &mut actions);
    apply_actions(app, actions);
}

/// Width of the modpack list on the left.
const LIST_W: f32 = 280.0;
/// The details on the right stop growing here, so lines stay readable on a wide screen.
const DETAILS_MAX_W: f32 = 820.0;
/// From this width (right of the modpack list) the modpack's mods get their own column.
const WIDE_WITH_MODS: f32 = 1000.0;
/// That column's narrowest width.
const MODS_COLUMN_MIN_W: f32 = 360.0;

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
            SetAction::SetPicture(name, file) => app.set_modlist_picture(&name, &file),
            SetAction::RemovePicture(name) => app.remove_modlist_picture(&name),
            SetAction::ToggleTag(name, tag) => app.toggle_modlist_tag(&name, &tag),
            SetAction::SaveSettings(name) => app.save_modlist_settings(&name),
            SetAction::RestoreSettings(name) => app.restore_modlist_settings(&name),
            SetAction::ForgetSettings(name) => app.forget_modlist_settings(&name),
            SetAction::Compare(name, with) => app.open_set_compare(&name, with),
            SetAction::CloseCompare => app.set_compare = None,
            SetAction::Merge(first, second) => app.merge_modlists(&first, &second),
            SetAction::CheckWorkshop(ids) => app.check_workshop_items(&ids),
            SetAction::LoadIcons(icons) => {
                for (key, preview) in icons {
                    app.mod_textures.insert(key.clone(), None);
                    if let Some(path) = preview {
                        let _ = app.icon_tx.send((key, path));
                    }
                }
            }
            SetAction::OpenFolder => {
                if let Some(dir) = app.modlists_folder() {
                    let _ = std::fs::create_dir_all(&dir);
                    open_external(dir);
                }
            }
        }
    }
}

/// The picture of a set in `rect`: the user's own, or the generated placeholder with the
/// modpack icon on top. Before the textures are made, a plain tinted square.
fn paint_set_picture(app: &App, ui: &Ui, name: &str, rect: egui::Rect, radius: u8) {
    let Some((tex, custom)) = app.modlist_pictures.get(name) else {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius), tint(THEME.accent_cyan));
        return;
    };
    egui::Image::from_texture(tex)
        .corner_radius(CornerRadius::same(radius))
        .paint_at(ui, rect);
    if !custom {
        let ([r, g, b], _) = crate::modlist_images::placeholder_colors(name);
        let icon = egui::Rect::from_center_size(rect.center(), rect.size() * 0.46);
        egui::Image::new(ICON_MODS)
            .tint(Color32::from_rgb(r, g, b))
            .paint_at(ui, icon);
    }
}

/// The left list: title with Import / Collection icons, "New from current", search, tag
/// filter, then one row per modpack.
fn render_list_panel(app: &mut App, ui: &mut Ui, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t.get(TranslationKey::SetsTitle))
                .color(THEME.text_primary)
                .size(18.0)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let busy = app.collection_busy();
            if ui
                .add_enabled_ui(!busy, |ui| boxed_icon_button(ui, ICON_STEAM))
                .inner
                .on_hover_text(format!(
                    "{}\n{}",
                    t.get(TranslationKey::SetsBtnCollection),
                    t.get(TranslationKey::SetsCollectionHint)
                ))
                .clicked()
            {
                actions.push(SetAction::Dialog(ModListDialog::Collection {
                    url: String::new(),
                    error: None,
                }));
            }
            if boxed_icon_button(ui, ICON_FOLDER)
                .on_hover_text(format!(
                    "{}\n{}",
                    t.get(TranslationKey::SetsBtnImport),
                    t.get(TranslationKey::SetsImportHint)
                ))
                .clicked()
            {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter(t.get(TranslationKey::SetsImportFilter), &["rml", "xml", "zip"])
                    .pick_file()
                {
                    actions.push(SetAction::Import(file));
                }
            }
            if busy {
                ui.spinner();
            }
        });
    });
    ui.add_space(10.0);
    if primary_button_icon(ui, ICON_PLUS, t.get(TranslationKey::SetsNewFromCurrent)).clicked() {
        actions.push(new_set_dialog(app));
    }
    ui.add_space(10.0);
    let width = ui.available_width();
    search_input_width(ui, &mut app.sets_search, t.get(TranslationKey::SetsSearch), width);
    ui.add_space(8.0);

    // Tags in use: clicking one shows only the sets with it (several: any of them).
    let used = crate::modlist_tags::in_use(&app.modlist_tags, app.modlists.iter().map(|l| l.name.as_str()));
    app.sets_tag_filter
        .retain(|f| used.iter().any(|(tag, _)| tag.to_lowercase() == *f));
    if !used.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            if chip(
                ui,
                None,
                t.get(TranslationKey::CategoriesAll),
                app.modlists.len(),
                app.sets_tag_filter.is_empty(),
            )
            .clicked()
            {
                app.sets_tag_filter.clear();
            }
            for (tag, n) in &used {
                let key = tag.to_lowercase();
                let on = app.sets_tag_filter.contains(&key);
                if chip(ui, None, &tag_label(t, tag), *n, on).clicked() {
                    if on {
                        app.sets_tag_filter.remove(&key);
                    } else {
                        app.sets_tag_filter.insert(key);
                    }
                }
            }
        });
        ui.add_space(8.0);
    }

    let app: &App = app;
    if app.modlists.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(t.get(TranslationKey::SetsNoneYet))
                    .color(THEME.text_muted)
                    .size(12.0),
            )
            .wrap(),
        );
        return;
    }
    let query = app.sets_search.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("sets_list_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for list in app
                .modlists
                .iter()
                .filter(|l| query.is_empty() || l.name.to_lowercase().contains(&query))
                .filter(|l| {
                    app.sets_tag_filter.is_empty()
                        || app
                            .sets_tag_filter
                            .iter()
                            .any(|f| crate::modlist_tags::has(&app.modlist_tags, &l.name, f))
                })
            {
                let selected = app.selected_modlist.as_deref() == Some(list.name.as_str());
                if set_row(app, ui, list, selected).clicked() {
                    actions.push(SetAction::Select(list.name.clone()));
                }
            }
        });
}

/// "New from current" opens the name dialog with a numbered default name.
fn new_set_dialog(app: &App) -> SetAction {
    let default = fmt(
        app.i18n.get(TranslationKey::SetsDefaultName),
        &[&(app.modlists.len() + 1).to_string()],
    );
    SetAction::Dialog(ModListDialog::SaveAs {
        name: default,
        error: None,
    })
}

/// One modpack in the list: picture, name, "N mods · 1.7", and a state mark on the right
/// (green dot: the game loads it now; red count: mods missing on disk).
fn set_row(app: &App, ui: &mut Ui, list: &ModList, selected: bool) -> egui::Response {
    let t = app.i18n;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = response.hovered();
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), tint(THEME.accent_cyan));
        let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 12.0), Vec2::new(3.0, 24.0));
        ui.painter().rect_filled(bar, CornerRadius::same(2), THEME.accent_cyan);
    } else if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), THEME.bg_surface_hover);
    }
    let pic = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 10.0, rect.center().y - 18.0),
        Vec2::splat(36.0),
    );
    paint_set_picture(app, ui, &list.name, pic, 8);

    // State on the right, then the name and the line under it take what is left.
    let mut right = rect.right() - 10.0;
    let painter = ui.painter();
    if app.modlist_diff(list).is_same() {
        painter.circle_filled(egui::pos2(right - 4.0, rect.center().y), 4.0, THEME.accent_emerald);
        right -= 16.0;
    } else {
        let missing = app.modlist_missing(list);
        if missing > 0 {
            let galley = painter.layout_no_wrap(
                format!("−{missing}"),
                egui::FontId::proportional(11.0),
                THEME.accent_rose,
            );
            let w = galley.size().x;
            painter.galley(
                egui::pos2(right - w, rect.center().y - galley.size().y / 2.0),
                galley,
                THEME.accent_rose,
            );
            right -= w + 10.0;
        }
    }
    let text_left = pic.right() + 10.0;
    let text_w = (right - text_left).max(20.0);
    let clip = egui::Rect::from_min_max(egui::pos2(text_left, rect.top()), egui::pos2(right, rect.bottom()));
    let name = painter.layout(
        list.name.clone(),
        egui::FontId::proportional(13.5),
        if selected {
            THEME.text_primary
        } else {
            THEME.text_secondary
        },
        f32::INFINITY,
    );
    let name = if name.size().x > text_w {
        let mut job = egui::text::LayoutJob::simple_singleline(
            list.name.clone(),
            egui::FontId::proportional(13.5),
            THEME.text_primary,
        );
        job.wrap.max_width = text_w;
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
        painter.layout_job(job)
    } else {
        name
    };
    painter
        .with_clip_rect(clip)
        .galley(egui::pos2(text_left, rect.center().y - 16.0), name, THEME.text_primary);
    let version = list
        .game_version
        .as_deref()
        .and_then(crate::game_install::major_minor)
        .unwrap_or_default();
    let mut line = fmt(t.get(TranslationKey::SetsModsCount), &[&list.ids.len().to_string()]);
    if !version.is_empty() {
        line.push_str(&format!("  ·  {version}"));
    }
    painter.with_clip_rect(clip).text(
        egui::pos2(text_left, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        line,
        egui::FontId::proportional(11.0),
        THEME.text_muted,
    );
    response
}

/// No modpacks yet: what they are and the three ways to make one.
fn render_empty(app: &App, ui: &mut Ui, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(14), tint(THEME.accent_cyan));
        egui::Image::new(ICON_COPY)
            .tint(THEME.accent_cyan)
            .paint_at(ui, rect.shrink(14.0));
        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(t.get(TranslationKey::SetsNoneYet))
                .color(THEME.text_primary)
                .size(18.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.set_max_width(420.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(t.get(TranslationKey::SetsHint))
                    .color(THEME.text_muted)
                    .size(12.5),
            )
            .halign(egui::Align::Center)
            .wrap(),
        );
        ui.add_space(14.0);
        if primary_button_icon(ui, ICON_PLUS, t.get(TranslationKey::SetsNewFromCurrent)).clicked() {
            actions.push(new_set_dialog(app));
        }
    });
}

/// A tag of the open modpack: a small pill, lit when the modpack has it.
fn tag_pill(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.0),
        THEME.text_secondary,
    );
    let size = Vec2::new(galley.size().x + 20.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let (fill, stroke, color) = if on {
            (tint(THEME.accent_cyan), THEME.accent_cyan, THEME.accent_cyan)
        } else if hovered {
            (THEME.bg_surface_hover, THEME.border_strong, THEME.text_primary)
        } else {
            (THEME.bg_surface, THEME.border_subtle, THEME.text_secondary)
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(13),
            fill,
            Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The set open on the right: the loaded one, else the first.
fn selected_list(app: &App) -> Option<&ModList> {
    app.selected_modlist
        .as_deref()
        .and_then(|name| app.modlists.iter().find(|l| l.name == name))
}

/// The selected set: picture, tags, actions, details, missing mods; its mod list too when
/// `with_mods` (the wide layout puts that in a column of its own).
fn render_details(app: &App, ui: &mut Ui, actions: &mut Vec<SetAction>, with_mods: bool) {
    let t = app.i18n;
    let Some(list) = selected_list(app) else {
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

    // Picture (click to choose another), name, file, picture buttons
    let custom_picture = app.modlist_pictures.get(&name).is_some_and(|(_, custom)| *custom);
    let pick_picture = || {
        rfd::FileDialog::new()
            .add_filter(t.get(TranslationKey::SetsPictureFilter), &["png", "jpg", "jpeg", "gif"])
            .pick_file()
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (pic_rect, pic_resp) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::click());
        paint_set_picture(app, ui, &name, pic_rect, 10);
        if pic_resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(t.get(TranslationKey::SetsPictureHint))
            .clicked()
        {
            if let Some(file) = pick_picture() {
                actions.push(SetAction::SetPicture(name.clone(), file));
            }
        }
        ui.vertical(|ui| {
            // A long name wraps instead of widening the column past the window.
            ui.set_max_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&list.name)
                        .color(THEME.text_primary)
                        .size(20.0)
                        .strong(),
                )
                .wrap(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(shown_path(&list.path))
                        .color(THEME.text_muted)
                        .size(11.0),
                )
                .truncate(),
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if secondary_button(ui, t.get(TranslationKey::SetsPictureChoose))
                    .on_hover_text(t.get(TranslationKey::SetsPictureHint))
                    .clicked()
                {
                    if let Some(file) = pick_picture() {
                        actions.push(SetAction::SetPicture(name.clone(), file));
                    }
                }
                if custom_picture && secondary_button(ui, t.get(TranslationKey::SetsPictureRemove)).clicked() {
                    actions.push(SetAction::RemovePicture(name.clone()));
                }
            });
        });
    });
    ui.add_space(8.0);

    // Tags: the built-in ones and own ones used by any set, lit when this set has them;
    // the field adds a new own tag (Enter or "+").
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        ui.label(egui::RichText::new(t.get(TranslationKey::SetsTagsLabel)).color(THEME.text_muted));
        let mut offered: Vec<String> = crate::modlist_tags::BUILT_IN.iter().map(|s| s.to_string()).collect();
        let used = crate::modlist_tags::in_use(&app.modlist_tags, app.modlists.iter().map(|l| l.name.as_str()));
        for (tag, _) in used {
            if !offered.iter().any(|o| o.eq_ignore_ascii_case(&tag)) {
                offered.push(tag);
            }
        }
        for tag in &offered {
            let on = crate::modlist_tags::has(&app.modlist_tags, &name, tag);
            if tag_pill(ui, &tag_label(t, tag), on).clicked() {
                actions.push(SetAction::ToggleTag(name.clone(), tag.clone()));
            }
        }
        let id = ui.make_persistent_id(("sets_new_tag", &name));
        let mut text = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
        let edit = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text(t.get(TranslationKey::SetsTagNewHint))
                .char_limit(crate::modlist_tags::MAX_LEN)
                .desired_width(130.0),
        );
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let plus = ui
            .add_enabled(!text.trim().is_empty(), egui::Button::new("+"))
            .on_hover_text(t.get(TranslationKey::SetsTagAdd))
            .clicked();
        if enter || plus {
            if let Some(tag) = crate::modlist_tags::clean(&text) {
                if !crate::modlist_tags::has(&app.modlist_tags, &name, &tag) {
                    actions.push(SetAction::ToggleTag(name.clone(), tag));
                }
                text.clear();
            }
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    });
    ui.add_space(8.0);

    // The three main actions as buttons; the rest in the "More" menu.
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
        let more = menu_chip(ui, crate::i18n::tr!("Ещё", "More", "更多"), 0);
        egui::Popup::from_toggle_button_response(&more)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(220.0);
                ui.spacing_mut().button_padding = Vec2::new(8.0, 5.0);
                // Plain menu rows (no frame), icon or an empty space of the same width.
                let item = |ui: &mut Ui, icon: Option<egui::ImageSource<'static>>, label: &str| {
                    let has_icon = icon.is_some();
                    let image = egui::Image::new(icon.unwrap_or(ICON_DOT))
                        .tint(if has_icon {
                            THEME.text_secondary
                        } else {
                            Color32::TRANSPARENT
                        })
                        .fit_to_exact_size(Vec2::splat(14.0));
                    ui.add_sized(
                        [ui.available_width(), 26.0],
                        egui::Button::image_and_text(image, label).frame(false),
                    )
                    .clicked()
                };
                if item(ui, Some(ICON_SAVE), t.get(TranslationKey::SetsBtnSaveHere)) {
                    actions.push(SetAction::SaveHere(name.clone()));
                    ui.close();
                }
                if item(ui, Some(ICON_COPY), t.get(TranslationKey::SetsBtnDuplicate)) {
                    actions.push(SetAction::Duplicate(name.clone()));
                    ui.close();
                }
                if item(ui, None, t.get(TranslationKey::SetsBtnRename)) {
                    actions.push(SetAction::Dialog(ModListDialog::Rename {
                        from: name.clone(),
                        name: name.clone(),
                        error: None,
                    }));
                    ui.close();
                }
                ui.separator();
                if item(ui, None, t.get(TranslationKey::SetsBtnCompare)) {
                    actions.push(SetAction::Compare(name.clone(), CompareWith::Current));
                    ui.close();
                }
                // "Merge with…": the other sets in a submenu.
                if app.modlists.len() > 1 {
                    ui.menu_button(t.get(TranslationKey::SetsBtnMerge), |ui| {
                        ui.label(
                            egui::RichText::new(t.get(TranslationKey::SetsMergeHint))
                                .color(THEME.text_muted)
                                .size(12.0),
                        );
                        ui.separator();
                        for other in app.modlists.iter().filter(|l| l.name != *name) {
                            if ui
                                .button(fmt(
                                    t.get(TranslationKey::SetsMergeItem),
                                    &[&other.name, &other.ids.len().to_string()],
                                ))
                                .clicked()
                            {
                                actions.push(SetAction::Merge(name.clone(), other.name.clone()));
                                ui.close();
                            }
                        }
                    });
                }
                ui.separator();
                if item(ui, Some(ICON_SAVE), t.get(TranslationKey::SetsBtnExport)) {
                    if let Some(target) = rfd::FileDialog::new()
                        .set_file_name(format!("{name}.rml"))
                        .add_filter(t.get(TranslationKey::SetsImportFilter), &["rml"])
                        .add_filter(t.get(TranslationKey::SetsExportFilterZip), &["zip"])
                        .save_file()
                    {
                        actions.push(SetAction::Export {
                            name: name.clone(),
                            target,
                        });
                    }
                    ui.close();
                }
                if item(ui, Some(ICON_COPY), t.get(TranslationKey::SetsBtnCopyText)) {
                    let text = app.modlist_share_text(list);
                    ui.ctx().copy_text(text);
                    actions.push(SetAction::Copied(name.clone()));
                    ui.close();
                }
                if item(ui, Some(ICON_FOLDER), t.get(TranslationKey::ModsBtnFolder)) {
                    actions.push(SetAction::OpenFolder);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_sized(
                        [ui.available_width(), 26.0],
                        egui::Button::image_and_text(
                            egui::Image::new(ICON_TRASH)
                                .tint(THEME.accent_rose)
                                .fit_to_exact_size(Vec2::splat(14.0)),
                            egui::RichText::new(t.get(TranslationKey::SetsBtnDelete)).color(THEME.accent_rose),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    actions.push(SetAction::Dialog(ModListDialog::Delete { name: name.clone() }));
                    ui.close();
                }
            });
    });
    ui.add_space(12.0);

    // Details
    let diff = app.modlist_diff(list);
    let missing = app.modlist_missing(list);
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(t.get(TranslationKey::SetsDetails))
                .color(THEME.text_primary)
                .size(15.0)
                .strong(),
        );
        ui.add_space(4.0);
        egui::Grid::new("set_details_grid")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                // A value too long for a narrow column is cut with "…" (full text on hover)
                // instead of widening the card past the column.
                let row = |ui: &mut Ui, key: TranslationKey, value: String, color: Color32| {
                    ui.label(egui::RichText::new(t.get(key)).color(THEME.text_muted));
                    ui.add(egui::Label::new(egui::RichText::new(value).color(color)).truncate());
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

    render_settings_card(app, ui, &name, actions);

    // F6: mods of the set that are not installed
    if missing > 0 {
        // Whether the missing mods still exist on the Workshop (asked once per item); the
        // removed ones are neither offered for download nor counted as "find by hand".
        let links: Vec<String> = list
            .ids
            .iter()
            .enumerate()
            .filter(|(_, id)| app.find_mod(id).is_none())
            .filter_map(|(i, id)| app.workshop_id_for(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
            .collect();
        let to_check: Vec<String> = links
            .iter()
            .filter(|ws| !app.ws_item_checked.contains(*ws))
            .cloned()
            .collect();
        if !to_check.is_empty() {
            actions.push(SetAction::CheckWorkshop(to_check));
        }
        let gone = links.iter().filter(|ws| app.workshop_item_gone(ws)).count();
        let workshop = app.modlist_missing_workshop_ids(list).len();
        let manual = missing.saturating_sub(workshop + gone);
        let by_name = app.modlist_wanted(list, None).len();
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(fmt(t.get(TranslationKey::SetsMissingTitle), &[&missing.to_string()]))
                    .color(THEME.accent_rose)
                    .strong(),
            );
            if gone > 0 {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(crate::i18n::tr!(
                            "Удалены из Workshop: {} — их уже не скачать. Уберите их из сборки или поставьте замену (другую версию этого мода).",
                            "Removed from the Workshop: {}, they can't be downloaded any more. Take them out of the modpack or use a replacement (another version of the mod).",
                            "已从创意工坊移除：{} 个，无法再下载。请将其移出整合包或换用替代版本。",
                            gone
                        ))
                        .color(THEME.accent_amber)
                        .size(12.0),
                    )
                    .wrap(),
                );
            }
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
                let key = if app.steamcmd_active() {
                    TranslationKey::SetsBtnDownloadMissing
                } else {
                    TranslationKey::SetsBtnSubscribeMissing
                };
                let label = fmt(t.get(key), &[&workshop.to_string()]);
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

    if with_mods {
        // Inside a scrolling column the height is unlimited: keep the list to the window.
        let max_h = ui
            .available_height()
            .min(ui.ctx().content_rect().height() * 0.7)
            .max(240.0);
        render_set_mods(app, ui, list, actions, max_h);
    }
}

/// Mod settings kept with the modpack: whether a copy exists, and its buttons.
fn render_settings_card(app: &App, ui: &mut Ui, name: &str, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    let saved = app.modlist_settings.get(name);
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(t.get(TranslationKey::SetsSettingsTitle))
                .color(THEME.text_primary)
                .size(15.0)
                .strong(),
        );
        ui.add_space(4.0);
        let (status, hint) = match saved {
            Some(s) => (
                fmt(
                    t.get(TranslationKey::SetsSettingsSaved),
                    &[
                        &s.saved_at.map(local_date_time).unwrap_or_else(|| "—".into()),
                        &s.files.to_string(),
                    ],
                ),
                TranslationKey::SetsSettingsKeptHint,
            ),
            None => (
                t.get(TranslationKey::SetsSettingsNone).to_string(),
                TranslationKey::SetsSettingsHint,
            ),
        };
        ui.add(egui::Label::new(egui::RichText::new(status).color(THEME.text_primary)).wrap());
        ui.add(egui::Label::new(egui::RichText::new(t.get(hint)).color(THEME.text_muted).size(12.0)).wrap());
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            let save = if saved.is_some() {
                TranslationKey::SetsSettingsBtnUpdate
            } else {
                TranslationKey::SetsSettingsBtnSave
            };
            if secondary_button_icon(ui, ICON_SAVE, t.get(save))
                .on_hover_text(t.get(TranslationKey::SetsSettingsSaveHint))
                .clicked()
            {
                actions.push(SetAction::SaveSettings(name.to_string()));
            }
            if saved.is_some() {
                if secondary_button(ui, t.get(TranslationKey::SetsSettingsBtnRestore))
                    .on_hover_text(t.get(TranslationKey::SetsSettingsRestoreHint))
                    .clicked()
                {
                    actions.push(SetAction::RestoreSettings(name.to_string()));
                }
                if secondary_button(ui, t.get(TranslationKey::SetsSettingsBtnForget)).clicked() {
                    actions.push(SetAction::ForgetSettings(name.to_string()));
                }
            }
        });
    });
}

/// "Compare" window: the selected modpack against the current list, another modpack or a
/// save: what only one of them has, and which shared mods sit elsewhere.
fn render_compare_window(app: &App, ctx: &egui::Context, actions: &mut Vec<SetAction>) {
    let Some(compare) = app.set_compare.as_ref() else {
        return;
    };
    let t = app.i18n;
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let (width, max_height) = dialog_size(ctx, 640.0);
    let with_label = |with: &CompareWith| match with {
        CompareWith::Current => t.get(TranslationKey::SetsCompareCurrent).to_string(),
        CompareWith::Modpack(name) => name.clone(),
        CompareWith::Save(name) => fmt(t.get(TranslationKey::SetsCompareSave), &[name]),
    };
    let second = with_label(&compare.with);

    egui::Window::new(t.get(TranslationKey::SetsCompareTitle))
        .id(egui::Id::unique("set_compare_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .max_height(max_height)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(&compare.first).color(THEME.accent_cyan).strong());
                ui.label(egui::RichText::new(t.get(TranslationKey::SetsCompareWith)).color(THEME.text_muted));
                egui::ComboBox::from_id_salt("set_compare_with")
                    .selected_text(&second)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        let mut choices = vec![CompareWith::Current];
                        choices.extend(
                            app.modlists
                                .iter()
                                .filter(|l| l.name != compare.first)
                                .map(|l| CompareWith::Modpack(l.name.clone())),
                        );
                        choices.extend(app.saves.iter().map(|s| CompareWith::Save(s.name.clone())));
                        for choice in choices {
                            if ui
                                .selectable_label(choice == compare.with, with_label(&choice))
                                .clicked()
                            {
                                actions.push(SetAction::Compare(compare.first.clone(), choice));
                            }
                        }
                    });
            });
            ui.add_space(6.0);

            let diff = &compare.result;
            if diff.is_same() {
                ui.label(egui::RichText::new(t.get(TranslationKey::SetsCompareSame)).color(THEME.accent_emerald));
                return;
            }
            ui.label(
                egui::RichText::new(fmt(
                    t.get(TranslationKey::SetsCompareShared),
                    &[&diff.shared.to_string()],
                ))
                .color(THEME.text_muted),
            );
            let section = |ui: &mut Ui, title: String, ids: &[String], color: Color32| {
                if ids.is_empty() {
                    return;
                }
                ui.add_space(6.0);
                card_container(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(egui::RichText::new(title).color(color).strong()).wrap());
                    ui.add_space(2.0);
                    for id in ids {
                        let name = compare.names.get(&id.to_lowercase()).unwrap_or(id);
                        ui.add(
                            egui::Label::new(egui::RichText::new(name).color(THEME.text_secondary).size(12.5))
                                .truncate(),
                        )
                        .on_hover_text(id);
                    }
                });
            };
            let count = |ids: &[String]| ids.len().to_string();
            section(
                ui,
                fmt(
                    t.get(TranslationKey::SetsCompareOnly),
                    &[&compare.first, &count(&diff.only_first)],
                ),
                &diff.only_first,
                THEME.accent_amber,
            );
            section(
                ui,
                fmt(
                    t.get(TranslationKey::SetsCompareOnly),
                    &[&second, &count(&diff.only_second)],
                ),
                &diff.only_second,
                THEME.accent_info,
            );
            section(
                ui,
                fmt(t.get(TranslationKey::SetsCompareMoved), &[&count(&diff.moved)]),
                &diff.moved,
                THEME.text_primary,
            );
        });
    if !open {
        actions.push(SetAction::CloseCompare);
    }
}

/// The mods of the selected set, numbered; ones not on disk get a Workshop link or a
/// "find by name" button. `max_h`: height of the scrolling list.
fn render_set_mods(app: &App, ui: &mut Ui, list: &ModList, actions: &mut Vec<SetAction>, max_h: f32) {
    let t = app.i18n;
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(fmt(
                t.get(TranslationKey::SetsModsTitle),
                &[&list.ids.len().to_string()],
            ))
            .color(THEME.text_primary)
            .size(15.0)
            .strong(),
        );
        ui.add_space(4.0);
        // Every set starts at the top of its list.
        egui::ScrollArea::vertical()
            .id_salt(("set_mods_scroll", &list.name))
            .auto_shrink([false, false])
            .max_height(max_h)
            .show_rows(ui, 22.0, list.ids.len(), |ui, range| {
                for i in range {
                    let id = &list.ids[i];
                    let found = app.find_mod(id);
                    let link = found
                        .is_none()
                        .then(|| app.workshop_id_for(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
                        .flatten();
                    let gone = link.as_deref().is_some_and(|ws| app.workshop_item_gone(ws));
                    let name = found
                        .map(|m| m.name.clone())
                        .or_else(|| list.names.get(i).cloned())
                        .unwrap_or_else(|| id.clone());
                    // Kept as "workshop.<ID>": the title Steam (or the Workshop database) has.
                    let name = link
                        .as_deref()
                        .filter(|_| name.starts_with("workshop."))
                        .and_then(|ws| {
                            app.ws_item_titles.get(ws).cloned().or_else(|| {
                                app.workshop_index
                                    .as_ref()
                                    .and_then(|idx| idx.name_for_id(ws))
                                    .map(str::to_string)
                            })
                        })
                        .unwrap_or(name);
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
                            if gone {
                                tag(
                                    ui,
                                    crate::i18n::tr!(
                                        "удалён из Workshop",
                                        "removed from the Workshop",
                                        "已从创意工坊移除"
                                    ),
                                    THEME.accent_rose,
                                );
                            } else {
                                tag(ui, t.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                            }
                            if let Some(steam_id) = link {
                                if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                                    .on_hover_text(t.get(TranslationKey::SetsOpenWorkshop))
                                    .clicked()
                                {
                                    open_in_workshop_tab(ui.ctx(), &steam_id);
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

/// The modpack's mods as a full-height column: numbered rows like the active list on the Mods
/// tab (icon, name, categories, "up to 1.6"); a mod that isn't installed is marked, with its
/// Workshop page or a search by its name.
fn render_set_mods_column(app: &App, ui: &mut Ui, list: &ModList, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    let game_mm = app.steam_paths.game_major_minor();
    let mut missing_icons: Vec<(String, Option<std::path::PathBuf>)> = Vec::new();
    // Whether the modpack's missing mods still exist on the Workshop (asked once per item).
    let to_check: Vec<String> = list
        .ids
        .iter()
        .enumerate()
        .filter(|(_, id)| app.find_mod(id).is_none())
        .filter_map(|(i, id)| app.workshop_id_for(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
        .filter(|ws| !app.ws_item_checked.contains(ws))
        .collect();
    if !to_check.is_empty() {
        actions.push(SetAction::CheckWorkshop(to_check));
    }
    egui::Frame {
        inner_margin: egui::Margin::same(0),
        ..raised_frame(8)
    }
    .show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(ui.available_height());
        // Header like the Mods tab's panels: title, count, what's missing.
        ui.horizontal(|ui| {
            ui.set_height(44.0);
            ui.add_space(14.0);
            ui.label(
                egui::RichText::new(crate::i18n::tr!("МОДЫ СБОРКИ", "MODPACK MODS", "整合包模组"))
                    .color(THEME.text_secondary)
                    .size(12.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(list.ids.len().to_string())
                    .color(THEME.text_muted)
                    .size(12.0),
            );
            let missing = app.modlist_missing(list);
            if missing > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    tag(
                        ui,
                        &fmt(t.get(TranslationKey::SetsMissingShort), &[&missing.to_string()]),
                        THEME.accent_rose,
                    );
                });
            }
        });
        let rows_h = (ui.available_height() - 8.0).max(120.0);
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("set_mods_column_scroll", &list.name))
                    .auto_shrink([false, false])
                    .max_height(rows_h)
                    .show_rows(ui, 34.0, list.ids.len(), |ui, range| {
                        for i in range {
                            let id = &list.ids[i];
                            let found = app.find_mod(id);
                            let name = found
                                .map(|m| m.name.clone())
                                .or_else(|| list.names.get(i).cloned())
                                .unwrap_or_else(|| id.clone());
                            super::view_mods::mod_row_frame(false, false, false).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.set_height(22.0);
                                    let (num, _) = ui.allocate_exact_size(Vec2::new(28.0, 22.0), Sense::hover());
                                    ui.painter().text(
                                        num.right_center(),
                                        egui::Align2::RIGHT_CENTER,
                                        format!("{}.", i + 1),
                                        egui::FontId::proportional(12.0),
                                        THEME.text_muted,
                                    );
                                    match found {
                                        Some(m) => {
                                            let key = m.package_id.to_lowercase();
                                            let tex = app.mod_textures.get(&key);
                                            if tex.is_none() {
                                                missing_icons.push((key, m.preview_path.clone()));
                                            }
                                            super::view_mods::render_mod_icon_pill(
                                                ui,
                                                m.categories.first().copied(),
                                                crate::models::is_protected_package_id(&m.package_id),
                                                tex.and_then(|t| t.as_ref()),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&name).color(THEME.text_primary).size(13.5),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&m.package_id);
                                            super::view_mods::category_badges(ui, &m.categories, t, 2);
                                            super::view_mods::version_badge(
                                                ui,
                                                m,
                                                game_mm.as_deref(),
                                                app.mod_updates.as_ref(),
                                                t,
                                            );
                                        }
                                        None => {
                                            // Not on disk: the name the modpack kept, a red mark,
                                            // and where to get it. A row kept as "workshop.<ID>"
                                            // shows the title Steam (or the Workshop database) has.
                                            let link = app.workshop_id_for(
                                                id,
                                                list.steam_id(id),
                                                list.names.get(i).map(String::as_str),
                                            );
                                            let gone = link.as_deref().is_some_and(|ws| app.workshop_item_gone(ws));
                                            let name = link
                                                .as_deref()
                                                .filter(|_| name.starts_with("workshop."))
                                                .and_then(|ws| {
                                                    app.ws_item_titles.get(ws).cloned().or_else(|| {
                                                        app.workshop_index
                                                            .as_ref()
                                                            .and_then(|idx| idx.name_for_id(ws))
                                                            .map(str::to_string)
                                                    })
                                                })
                                                .unwrap_or(name.clone());
                                            let (icon, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                                            ui.painter().rect_filled(
                                                icon,
                                                CornerRadius::same(4),
                                                tint(THEME.accent_rose),
                                            );
                                            egui::Image::new(ICON_STATUS_NOT_ON_DISK)
                                                .tint(THEME.accent_rose)
                                                .paint_at(ui, icon.shrink(3.0));
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&name).color(THEME.text_secondary).size(13.5),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(id);
                                            if gone {
                                                tag(
                                                    ui,
                                                    crate::i18n::tr!("удалён из Workshop", "removed from the Workshop", "已从创意工坊移除"),
                                                    THEME.accent_rose,
                                                )
                                                .on_hover_text(crate::i18n::tr!(
                                                    "Автор удалил мод или скрыл его — скачать его нельзя. Уберите его из сборки или поставьте замену (другую версию этого мода).",
                                                    "The author removed or hid it: it can't be downloaded. Take it out of the modpack or use a replacement (another version of this mod).",
                                                    "作者已删除或隐藏该模组，无法下载。请将其移出整合包或换用替代版本。"
                                                ));
                                            } else {
                                                tag(ui, t.get(TranslationKey::ModsNotOnDisk), THEME.accent_rose);
                                            }
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if let Some(steam_id) = link {
                                                    if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                                                        .on_hover_text(t.get(TranslationKey::SetsOpenWorkshop))
                                                        .clicked()
                                                    {
                                                        open_in_workshop_tab(ui.ctx(), &steam_id);
                                                    }
                                                } else if !id.to_lowercase().starts_with("ludeon.rimworld")
                                                    && icon_button(ui, ICON_SEARCH, Vec2::splat(14.0), THEME.text_muted)
                                                        .on_hover_text(t.get(TranslationKey::NameSearchRowHint))
                                                        .clicked()
                                                {
                                                    actions.push(SetAction::FindByName(
                                                        list.name.clone(),
                                                        Some(id.clone()),
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                });
                            });
                        }
                    });
            });
    });
    if !missing_icons.is_empty() {
        actions.push(SetAction::LoadIcons(missing_icons));
    }
}
