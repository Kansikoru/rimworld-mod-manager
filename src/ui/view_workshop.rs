//! "Workshop" tab: RimWorld's Steam Workshop inside the manager — search, sort, game
//! version, cards with previews, the picked mod's description, and "Download" (a Steam
//! subscription or SteamCMD, as set in Settings).

use eframe::egui::{self, Color32, Rounding, Sense, Stroke, Ui, Vec2};
use std::collections::{HashMap, HashSet};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::i18n::{fmt, I18n, TranslationKey};
use crate::workshop_browse::{human_size, short_count, BrowseSort, WorkshopItem, PER_PAGE};

const CARD_W: f32 = 210.0;
const GAP: f32 = 10.0;
const THUMB_W: u32 = 320;
const BIG_W: u32 = 640;
const VERSIONS: [&str; 4] = ["1.7", "1.6", "1.5", "1.4"];

enum WsAction {
    Search,
    Sort(BrowseSort),
    Version(Option<String>),
    Page(u32),
    Select(Option<String>),
    Download(String),
    DownloadMany(Vec<String>),
    DownloadWorkshopDb,
    Console,
}

/// What a card or the details show about an item being on disk.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Have {
    No,
    Installed,
    Requested,
}

fn sort_label(t: I18n, sort: BrowseSort) -> &'static str {
    t.get(match sort {
        BrowseSort::TrendWeek => TranslationKey::WsSortTrendWeek,
        BrowseSort::TrendMonth => TranslationKey::WsSortTrendMonth,
        BrowseSort::MostSubscribed => TranslationKey::WsSortMostSubscribed,
        BrowseSort::Recent => TranslationKey::WsSortRecent,
        BrowseSort::Updated => TranslationKey::WsSortUpdated,
    })
}

/// Versions oldest first: "1.5, 1.6, 1.7".
fn sorted_versions(versions: &[String]) -> Vec<String> {
    let mut v = versions.to_vec();
    v.sort_by_key(|s| s.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    v
}

pub fn render_workshop_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    app.ensure_workshop_loaded();
    section_header(ui, t.get(TranslationKey::WsTitle), t.get(TranslationKey::WsHint), None);
    ui.add_space(8.0);

    let mut actions = Vec::new();
    render_toolbar(app, ui, &mut actions);
    ui.add_space(8.0);

    // Previews first (they need `app` mutably), then everything is drawn from `&App`.
    let mut thumbs: HashMap<String, egui::TextureHandle> = HashMap::new();
    let urls: Vec<(String, String)> = app
        .ws_items
        .iter()
        .filter_map(|i| i.preview_url.clone().map(|u| (i.id.clone(), u)))
        .collect();
    for (id, url) in urls {
        if let Some(tex) = app.workshop_preview(&url, THUMB_W) {
            thumbs.insert(id, tex);
        }
    }
    let big = app
        .ws_selected
        .as_ref()
        .and_then(|id| app.ws_items.iter().find(|i| &i.id == id))
        .and_then(|i| i.preview_url.clone())
        .and_then(|url| app.workshop_preview(&url, BIG_W));
    let installed = app.installed_workshop_ids();

    let narrow = ui.available_width() < NARROW_TWO_COLUMNS;
    let selected = app
        .ws_selected
        .as_ref()
        .and_then(|id| app.ws_items.iter().find(|i| &i.id == id));
    if narrow {
        // One column: the list, or the picked mod with "Back to the list".
        if let Some(item) = selected {
            if secondary_button_icon(ui, ICON_CLOSE, t.get(TranslationKey::WsBack)).clicked() {
                actions.push(WsAction::Select(None));
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_salt("ws_details_narrow")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    render_details(app, ui, item, big.as_ref(), &installed, &mut actions)
                });
        } else {
            render_grid(app, ui, &thumbs, &installed, &mut actions);
        }
    } else {
        let height = ui.available_height();
        let right_w = (ui.available_width() * 0.38).clamp(300.0, 460.0);
        let left_w = ui.available_width() - right_w - 16.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(left_w);
                    render_grid(app, ui, &thumbs, &installed, &mut actions);
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(right_w);
                    match selected {
                        Some(item) => {
                            egui::ScrollArea::vertical()
                                .id_salt("ws_details")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    render_details(app, ui, item, big.as_ref(), &installed, &mut actions)
                                });
                        }
                        None => {
                            card_container(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    egui::RichText::new(t.get(TranslationKey::WsPickHint)).color(THEME.text_muted),
                                );
                            });
                        }
                    }
                },
            );
        });
    }
    apply(app, actions);
}

fn apply(app: &mut App, actions: Vec<WsAction>) {
    for action in actions {
        match action {
            WsAction::Search => app.search_workshop_tab(),
            WsAction::Sort(sort) => {
                app.ws_query.sort = sort;
                app.search_workshop_tab();
            }
            WsAction::Version(v) => {
                app.ws_query.version = v;
                app.search_workshop_tab();
            }
            WsAction::Page(p) => app.workshop_page(p),
            WsAction::Select(id) => app.ws_selected = id,
            WsAction::Download(id) => app.download_workshop_item(&id),
            WsAction::DownloadMany(ids) => app.download_workshop_items(ids),
            WsAction::DownloadWorkshopDb => app.download_workshop_index(),
            WsAction::Console => app.open_steamcmd_console(),
        }
    }
}

fn render_toolbar(app: &mut App, ui: &mut Ui, actions: &mut Vec<WsAction>) {
    let t = app.i18n;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        let field_w = ui.available_width().clamp(180.0, 340.0);
        if search_input_width(ui, &mut app.ws_search, t.get(TranslationKey::WsSearch), field_w) {
            actions.push(WsAction::Search);
        }
        if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::WsBtnSearch)).clicked() {
            actions.push(WsAction::Search);
        }
        // A search is sorted by relevance: the sort applies to the catalog.
        ui.add_enabled_ui(app.ws_query.text.is_empty(), |ui| {
            egui::ComboBox::from_id_salt("ws_sort")
                .selected_text(sort_label(t, app.ws_query.sort))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for sort in BrowseSort::ALL {
                        if ui
                            .selectable_label(app.ws_query.sort == sort, sort_label(t, sort))
                            .clicked()
                        {
                            actions.push(WsAction::Sort(sort));
                        }
                    }
                });
        });
        let version_text = app
            .ws_query
            .version
            .clone()
            .unwrap_or_else(|| t.get(TranslationKey::WsAnyVersion).to_string());
        egui::ComboBox::from_id_salt("ws_version")
            .selected_text(version_text)
            .width(120.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(app.ws_query.version.is_none(), t.get(TranslationKey::WsAnyVersion))
                    .clicked()
                {
                    actions.push(WsAction::Version(None));
                }
                for v in VERSIONS {
                    if ui
                        .selectable_label(app.ws_query.version.as_deref() == Some(v), v)
                        .clicked()
                    {
                        actions.push(WsAction::Version(Some(v.to_string())));
                    }
                }
            });
        if app.steamcmd_active()
            && secondary_button_icon(ui, ICON_STEAM, t.get(TranslationKey::SteamCmdConsole)).clicked()
        {
            actions.push(WsAction::Console);
        }
        if app.workshop_loading() {
            ui.spinner();
        }
    });
}

fn have(item: &WorkshopItem, installed: &HashSet<String>, app: &App) -> Have {
    if installed.contains(&item.id) {
        Have::Installed
    } else if app.ws_requested.contains(&item.id) {
        Have::Requested
    } else {
        Have::No
    }
}

fn render_grid(
    app: &App,
    ui: &mut Ui,
    thumbs: &HashMap<String, egui::TextureHandle>,
    installed: &HashSet<String>,
    actions: &mut Vec<WsAction>,
) {
    let t = app.i18n;
    if let Some(e) = &app.ws_error {
        card_container(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(fmt(t.get(TranslationKey::WsError), &[e])).color(THEME.accent_rose),
                )
                .wrap(),
            );
            if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::WsBtnSearch)).clicked() {
                actions.push(WsAction::Page(app.ws_query.page));
            }
        });
        return;
    }
    if app.ws_items.is_empty() {
        if !app.workshop_loading() {
            ui.label(egui::RichText::new(t.get(TranslationKey::WsEmpty)).color(THEME.text_muted));
        }
        return;
    }

    let pager_h = 44.0;
    egui::ScrollArea::vertical()
        .id_salt(("ws_grid", app.ws_query.page, &app.ws_query.text))
        .auto_shrink([false, false])
        .max_height((ui.available_height() - pager_h).max(120.0))
        .show(ui, |ui| {
            let width = ui.available_width();
            let cols = (((width + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
            let card_w = ((width - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
            for row in app.ws_items.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for item in row {
                        let selected = app.ws_selected.as_deref() == Some(item.id.as_str());
                        let state = have(item, installed, app);
                        if card(ui, t, item, thumbs.get(&item.id), card_w, selected, state).clicked() {
                            actions.push(WsAction::Select(Some(item.id.clone())));
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });

    // Pages: Steam doesn't say how many there are, a full page means there's a next one.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let page = app.ws_query.page.max(1);
        let busy = app.workshop_loading();
        if ui
            .add_enabled_ui(page > 1 && !busy, |ui| {
                secondary_button(ui, t.get(TranslationKey::WsPrev))
            })
            .inner
            .clicked()
        {
            actions.push(WsAction::Page(page - 1));
        }
        ui.label(
            egui::RichText::new(fmt(t.get(TranslationKey::WsPage), &[&page.to_string()])).color(THEME.text_secondary),
        );
        if ui
            .add_enabled_ui(app.ws_items.len() >= PER_PAGE && !busy, |ui| {
                secondary_button(ui, t.get(TranslationKey::WsNext))
            })
            .inner
            .clicked()
        {
            actions.push(WsAction::Page(page + 1));
        }
    });
}

/// A preview drawn into `rect` without stretching (letterboxed on the dark color).
fn paint_preview(ui: &Ui, rect: egui::Rect, tex: Option<&egui::TextureHandle>, rounding: Rounding) {
    ui.painter().rect_filled(rect, rounding, THEME.bg_dark);
    let Some(tex) = tex else { return };
    let size = tex.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let img_rect = egui::Rect::from_center_size(rect.center(), size * scale);
    ui.painter().image(
        tex.id(),
        img_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn card(
    ui: &mut Ui,
    t: I18n,
    item: &WorkshopItem,
    thumb: Option<&egui::TextureHandle>,
    width: f32,
    selected: bool,
    state: Have,
) -> egui::Response {
    let pad = 8.0;
    let preview_h = ((width - pad * 2.0) * 9.0 / 16.0).round();
    let height = pad + preview_h + 8.0 + 18.0 + 4.0 + 20.0 + pad;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let (fill, stroke) = if selected {
        (THEME.bg_surface_hover, Stroke::new(1.5_f32, THEME.accent_purple))
    } else if response.hovered() {
        (THEME.bg_surface_hover, Stroke::new(1.0_f32, THEME.border_strong))
    } else {
        (THEME.bg_card, Stroke::new(1.0_f32, THEME.border_subtle))
    };
    ui.painter().rect(rect, Rounding::same(8.0), fill, stroke);

    let inner = rect.shrink(pad);
    let preview = egui::Rect::from_min_size(inner.min, Vec2::new(inner.width(), preview_h));
    paint_preview(ui, preview, thumb, Rounding::same(6.0));

    // Title, then subscribers and versions, then the state.
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(inner.min.x, preview.max.y + 8.0),
                inner.max,
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.add(
        egui::Label::new(
            egui::RichText::new(&item.title)
                .color(THEME.text_primary)
                .strong()
                .size(13.0),
        )
        .truncate()
        .selectable(false),
    );
    child.add_space(4.0);
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(
            egui::Label::new(
                egui::RichText::new(short_count(item.subscriptions))
                    .color(THEME.text_muted)
                    .size(11.0),
            )
            .selectable(false),
        );
        match state {
            Have::Installed => {
                tag(ui, t.get(TranslationKey::WsInstalled), THEME.accent_emerald);
            }
            Have::Requested => {
                tag(ui, t.get(TranslationKey::WsRequested), THEME.accent_cyan);
            }
            Have::No => {}
        }
        for v in sorted_versions(&item.versions).iter().rev().take(2) {
            tag(ui, v, THEME.accent_purple);
        }
    });
    response.on_hover_text(&item.title)
}

fn render_details(
    app: &App,
    ui: &mut Ui,
    item: &WorkshopItem,
    preview: Option<&egui::TextureHandle>,
    installed: &HashSet<String>,
    actions: &mut Vec<WsAction>,
) {
    let t = app.i18n;
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, (w * 9.0 / 16.0).round()), Sense::hover());
        paint_preview(ui, rect, preview, Rounding::same(6.0));
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&item.title)
                    .color(THEME.text_primary)
                    .size(17.0)
                    .strong(),
            )
            .wrap(),
        );
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            let hint = if app.steamcmd_active() {
                t.get(TranslationKey::WsViaSteamCmd)
            } else {
                t.get(TranslationKey::WsViaSteam)
            };
            let all_dep_ids = app
                .workshop_index
                .as_ref()
                .map(|idx| idx.all_dependencies_of(&item.id))
                .unwrap_or_default();
            let is_dep_installed = |dep_id: &str| {
                installed.contains(dep_id)
                    || app.ws_requested.contains(dep_id)
                    || app
                        .workshop_index
                        .as_ref()
                        .and_then(|idx| idx.package_for_id(dep_id))
                        .is_some_and(|pkg| app.find_mod(pkg).is_some())
            };
            let missing_deps: Vec<String> = all_dep_ids.into_iter().filter(|id| !is_dep_installed(id)).collect();

            match have(item, installed, app) {
                Have::Installed => {
                    tag(ui, t.get(TranslationKey::WsInstalled), THEME.accent_emerald);
                }
                Have::Requested => {
                    tag(ui, t.get(TranslationKey::WsRequested), THEME.accent_cyan);
                }
                Have::No => {
                    if ui
                        .add_enabled_ui(!app.steam_busy(), |ui| {
                            primary_button_icon(ui, ICON_STEAM, t.get(TranslationKey::WsDownload))
                        })
                        .inner
                        .on_hover_text(hint)
                        .clicked()
                    {
                        actions.push(WsAction::Download(item.id.clone()));
                    }
                }
            }
            if !missing_deps.is_empty() {
                let mut to_download = Vec::new();
                if have(item, installed, app) == Have::No {
                    to_download.push(item.id.clone());
                }
                to_download.extend(missing_deps.clone());
                let label = fmt(
                    t.get(TranslationKey::WsDownloadWithDeps),
                    &[&missing_deps.len().to_string()],
                );
                if ui
                    .add_enabled_ui(!app.steam_busy(), |ui| secondary_button_icon(ui, ICON_STEAM, &label))
                    .inner
                    .on_hover_text(hint)
                    .clicked()
                {
                    actions.push(WsAction::DownloadMany(to_download));
                }
            }
            if secondary_button(ui, t.get(TranslationKey::NameSearchOpenSteam)).clicked() {
                open_external(crate::steam::workshop_url(&item.id));
            }
            if app.steam_busy() {
                ui.spinner();
            }
        });
        ui.add_space(6.0);

        if let Some(idx) = app.workshop_index.as_ref().filter(|i| !i.dependencies.is_empty()) {
            let direct_deps = idx.dependencies_of(&item.id);
            if !direct_deps.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::ModsDependencies))
                            .color(THEME.text_muted)
                            .size(12.0),
                    );
                    for (dep_id, dep_name) in &direct_deps {
                        let have_dep = installed.contains(dep_id)
                            || app.ws_requested.contains(dep_id)
                            || idx
                                .package_for_id(dep_id)
                                .is_some_and(|pkg| app.find_mod(pkg).is_some());
                        let color = if have_dep {
                            THEME.accent_emerald
                        } else {
                            THEME.accent_amber
                        };
                        if tag(ui, dep_name, color)
                            .interact(Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(format!("#{dep_id}"))
                            .clicked()
                        {
                            open_external(crate::steam::workshop_url(dep_id));
                        }
                    }
                });
                ui.add_space(4.0);
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::WsDepsNeedDb))
                        .color(THEME.text_muted)
                        .size(12.0),
                );
                let btn_key = if app.workshop_index.is_some() {
                    TranslationKey::WorkshopDbUpdateBtn
                } else {
                    TranslationKey::WorkshopDbBtn
                };
                if ui
                    .add_enabled_ui(!app.workshop_index_busy(), |ui| {
                        small_button(ui, t.get(btn_key), ButtonStyle::Secondary)
                    })
                    .inner
                    .on_hover_text(t.get(TranslationKey::WorkshopDbBtnHint))
                    .clicked()
                {
                    actions.push(WsAction::DownloadWorkshopDb);
                }
                if app.workshop_index_busy() {
                    ui.spinner();
                }
            });
            ui.add_space(4.0);
        }

        let muted = |ui: &mut Ui, text: String| {
            ui.label(egui::RichText::new(text).color(THEME.text_secondary).size(12.0));
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            muted(
                ui,
                fmt(t.get(TranslationKey::WsSubscribers), &[&item.subscriptions.to_string()]),
            );
            if item.file_size > 0 {
                muted(ui, human_size(item.file_size));
            }
            if item.time_updated > 0 {
                muted(
                    ui,
                    fmt(t.get(TranslationKey::WsUpdated), &[&local_date_time(item.time_updated)]),
                );
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for v in sorted_versions(&item.versions).iter().rev() {
                tag(ui, v, THEME.accent_purple);
            }
            for tg in &item.tags {
                tag(ui, tg, THEME.text_muted);
            }
        });
        if !item.description.is_empty() {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&item.description)
                        .color(THEME.text_secondary)
                        .size(12.5),
                )
                .wrap(),
            );
        }
    });
}
