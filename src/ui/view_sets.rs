//! "Modpacks" tab: mod sets as cards on the left, the selected one's details, actions and
//! mods on the right. The sets are the game's own ModLists files.

use eframe::egui::{self, Color32, CornerRadius, Sense, Stroke, Ui, Vec2};

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
    SetPicture(String, std::path::PathBuf),
    RemovePicture(String),
    ToggleTag(String, String),
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

const CARD_SIZE: Vec2 = Vec2::new(168.0, 140.0);

pub fn render_sets_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    section_header(
        ui,
        t.get(TranslationKey::SetsTitle),
        t.get(TranslationKey::SetsPickerHint),
        None,
    );
    ui.add_space(8.0);
    app.load_modlist_pictures(ui.ctx());

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
            .show(ui, |ui| render_details(app, ui, &mut actions, true));
        apply_actions(app, actions);
        return;
    }

    // Wide window: cards | the selected modpack | its mods, each column the full height.
    // Narrower: the modpack and its mods share the right column, which scrolls.
    let gap = 24.0;
    let full = ui.available_rect_before_wrap();
    let total = full.width();
    let height = full.height();
    let three = total >= WIDE_THREE_COLUMNS;
    let (left_w, mid_w) = if three {
        let free = total - 2.0 * gap;
        ((free * 0.36).floor(), (free * 0.34).floor())
    } else {
        let left = ((total - gap) * 0.5).floor();
        (left, total - gap - left)
    };
    let right_w = total - 2.0 * gap - left_w - mid_w;

    // Each column has a fixed place and is clipped to it: something too wide inside one
    // column is cut there instead of pushing the next columns past the window.
    let column_rect =
        |left: f32, width: f32| egui::Rect::from_min_size(egui::pos2(left, full.top()), Vec2::new(width, height));
    let cards_rect = column_rect(full.left(), left_w);
    let mid_rect = column_rect(cards_rect.right() + gap, mid_w);
    let mods_rect = column_rect(mid_rect.right() + gap, right_w);
    let column = |ui: &mut Ui, rect: egui::Rect| {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // 2 px on the sides for the card frames on the column edge.
        child.set_clip_rect(rect.expand2(Vec2::new(2.0, 0.0)).intersect(ui.clip_rect()));
        child
    };

    render_cards(app, &mut column(ui, cards_rect), &mut actions);
    egui::ScrollArea::vertical()
        .id_salt("sets_details")
        .auto_shrink([false, false])
        .show(&mut column(ui, mid_rect), |ui| {
            render_details(app, ui, &mut actions, !three)
        });
    if three {
        let mut mods_ui = column(ui, mods_rect);
        if let Some(list) = selected_list(app) {
            let max_h = (mods_ui.available_height() - 48.0).max(200.0);
            render_set_mods(app, &mut mods_ui, list, &mut actions, max_h);
        }
    }
    ui.allocate_rect(full, Sense::hover());
    apply_actions(app, actions);
}

/// From this width the Modpacks tab shows the selected modpack's mods in a third column.
const WIDE_THREE_COLUMNS: f32 = 1150.0;

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

/// A button above the modpack cards.
#[derive(Clone, Copy)]
enum HeaderItem {
    New,
    Import,
    Collection,
    /// While a Steam collection loads.
    Spinner,
}

fn render_cards(app: &mut App, ui: &mut Ui, actions: &mut Vec<SetAction>) {
    use HeaderItem::*;
    let t = app.i18n;
    // The column's own width, taken before anything is drawn: a row that came out too wide
    // makes `available_width` larger afterwards.
    let width = ui.available_width();
    let gap = 8.0;
    let sizes = [
        (
            New,
            styled_button_width(ui, true, t.get(TranslationKey::SetsNewFromCurrent)),
        ),
        (
            Import,
            styled_button_width(ui, true, t.get(TranslationKey::SetsBtnImport)),
        ),
        (
            Collection,
            styled_button_width(ui, true, t.get(TranslationKey::SetsBtnCollection)),
        ),
    ];
    let buttons_w: f32 = sizes.iter().map(|(_, w)| w + gap).sum();
    // Search and the buttons in one row while the search keeps 160 px: the buttons go from
    // the right edge and the search takes what is left. Otherwise the buttons go in rows
    // of what fits (the widths are exact: the same font and padding the buttons use), and
    // the search under them over the whole column.
    if width - buttons_w >= 160.0 {
        let row = Vec2::new(width, 32.0);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            header_buttons(app, ui, actions, &[Spinner, Collection, Import, New]);
            let search_w = ui.available_width();
            search_input_width(ui, &mut app.sets_search, t.get(TranslationKey::SetsSearch), search_w);
        });
    } else {
        let mut rows: Vec<Vec<HeaderItem>> = vec![Vec::new()];
        let mut used = 0.0;
        for (item, w) in sizes {
            let need = if used > 0.0 { gap + w } else { w };
            if used > 0.0 && used + need > width {
                rows.push(Vec::new());
                used = 0.0;
            }
            used += if used > 0.0 { gap + w } else { w };
            rows.last_mut().expect("a row is always there").push(item);
        }
        rows.last_mut().expect("a row is always there").push(Spinner);
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                ui.add_space(gap);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                header_buttons(app, ui, actions, row);
            });
        }
        ui.add_space(6.0);
        search_input_width(ui, &mut app.sets_search, t.get(TranslationKey::SetsSearch), width);
    }
    ui.add_space(10.0);
    render_card_grid(app, ui, actions);
}

/// Draws `items` in the given order ("New from current", "Import…", "Collection…", the
/// spinner while a collection loads).
fn header_buttons(app: &App, ui: &mut Ui, actions: &mut Vec<SetAction>, items: &[HeaderItem]) {
    use HeaderItem as Item;
    let t = app.i18n;
    let mut draw = |ui: &mut Ui, item: Item| match item {
        Item::New => {
            if primary_button_icon(ui, ICON_PLUS, t.get(TranslationKey::SetsNewFromCurrent)).clicked() {
                let default = fmt(
                    t.get(TranslationKey::SetsDefaultName),
                    &[&(app.modlists.len() + 1).to_string()],
                );
                actions.push(SetAction::Dialog(ModListDialog::SaveAs {
                    name: default,
                    error: None,
                }));
            }
        }
        Item::Import => {
            if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SetsBtnImport))
                .on_hover_text(t.get(TranslationKey::SetsImportHint))
                .clicked()
            {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter(t.get(TranslationKey::SetsImportFilter), &["rml", "xml", "zip"])
                    .pick_file()
                {
                    actions.push(SetAction::Import(file));
                }
            }
        }
        Item::Collection => {
            if ui
                .add_enabled_ui(!app.collection_busy(), |ui| {
                    secondary_button_icon(ui, ICON_STEAM, t.get(TranslationKey::SetsBtnCollection))
                })
                .inner
                .on_hover_text(t.get(TranslationKey::SetsCollectionHint))
                .clicked()
            {
                actions.push(SetAction::Dialog(ModListDialog::Collection {
                    url: String::new(),
                    error: None,
                }));
            }
        }
        Item::Spinner => {
            if app.collection_busy() {
                ui.spinner();
            }
        }
    };
    for item in items {
        draw(ui, *item);
    }
}

/// The tag filter and the cards of the sets.
fn render_card_grid(app: &mut App, ui: &mut Ui, actions: &mut Vec<SetAction>) {
    let t = app.i18n;
    // Tags in use: clicking one shows only the sets with it (several: any of them).
    let used = crate::modlist_tags::in_use(&app.modlist_tags, app.modlists.iter().map(|l| l.name.as_str()));
    app.sets_tag_filter
        .retain(|f| used.iter().any(|(tag, _)| tag.to_lowercase() == *f));
    if !used.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            if ui
                .selectable_label(app.sets_tag_filter.is_empty(), t.get(TranslationKey::CategoriesAll))
                .clicked()
            {
                app.sets_tag_filter.clear();
            }
            for (tag, n) in &used {
                let key = tag.to_lowercase();
                let on = app.sets_tag_filter.contains(&key);
                if ui
                    .selectable_label(on, format!("{} · {n}", tag_label(t, tag)))
                    .clicked()
                {
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
                    .filter(|l| {
                        app.sets_tag_filter.is_empty()
                            || app
                                .sets_tag_filter
                                .iter()
                                .any(|f| crate::modlist_tags::has(&app.modlist_tags, &l.name, f))
                    })
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
    painter.rect(rect, CornerRadius::same(8), fill, stroke, egui::StrokeKind::Middle);

    // Picture: the user's own or the generated one
    let square = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 36.0), Vec2::splat(52.0));
    paint_set_picture(app, ui, &list.name, square, 8);

    // Name: at most two centred lines, cut with "…" (a third would run into the mod count)
    let mut job = egui::text::LayoutJob::simple(
        list.name.clone(),
        egui::FontId::proportional(13.0),
        THEME.text_primary,
        rect.width() - 16.0,
    );
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    job.halign = egui::Align::Center;
    let name_galley = painter.layout_job(job);
    painter.with_clip_rect(rect.shrink(6.0)).galley(
        egui::pos2(rect.center().x, rect.top() + 68.0),
        name_galley,
        THEME.text_primary,
    );

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
                        .color(THEME.accent_cyan)
                        .size(18.0)
                        .strong(),
                )
                .wrap(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(list.path.display().to_string())
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
            if ui.selectable_label(on, tag_label(t, tag)).clicked() {
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
                .add_filter(t.get(TranslationKey::SetsImportFilter), &["rml"])
                .add_filter(t.get(TranslationKey::SetsExportFilterZip), &["zip"])
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
            ui.ctx().copy_text(text);
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
            .color(THEME.accent_cyan)
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
