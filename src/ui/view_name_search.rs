//! "Find mods by name" window (D1): missing mods with no Workshop link, what the Workshop
//! search found for each, ticks, and one "Subscribe" for the ticked ones.

use eframe::egui::{self, Rounding, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::i18n::{fmt, TranslationKey};

const WIDTH: f32 = 620.0;

pub fn render_name_search_window(app: &mut App, ctx: &egui::Context) {
    if app.name_search.is_none() {
        return;
    }
    let t = app.i18n;
    let steam_busy = app.steam_busy();
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut subscribe = false;
    let mut stop = false;
    let mut close = false;
    let (width, max_height) = dialog_size(ctx, WIDTH);

    egui::Window::new(t.get(TranslationKey::NameSearchTitle))
        .id(egui::Id::new("name_search_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .max_height(max_height)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.set_width(width);
            let Some(search) = app.name_search.as_mut() else { return };
            ui.label(
                egui::RichText::new(fmt(
                    t.get(TranslationKey::NameSearchFrom),
                    &[&search.source, &search.rows.len().to_string()],
                ))
                .color(THEME.accent_cyan)
                .strong(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(t.get(TranslationKey::NameSearchIntro))
                        .color(THEME.text_muted)
                        .size(12.0),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let total = search.rows.len().to_string();
                let done = search.done.to_string();
                let text = if search.running {
                    fmt(t.get(TranslationKey::NameSearchProgress), &[&done, &total])
                } else {
                    fmt(
                        t.get(TranslationKey::NameSearchSummary),
                        &[&done, &total, &search.found().to_string()],
                    )
                };
                ui.label(egui::RichText::new(text).color(THEME.text_secondary).size(12.0));
                if search.running {
                    ui.spinner();
                }
            });
            ui.add_space(6.0);

            for row in &search.rows {
                egui::Frame {
                    inner_margin: egui::Margin::symmetric(10.0, 6.0),
                    outer_margin: egui::Margin::symmetric(0.0, 2.0),
                    rounding: Rounding::same(6.0),
                    fill: THEME.bg_card,
                    ..Default::default()
                }
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if icon_button(ui, ICON_SEARCH, Vec2::splat(14.0), THEME.text_muted)
                            .on_hover_text(t.get(TranslationKey::NameSearchBrowse))
                            .clicked()
                        {
                            open_external(crate::name_search::browse_url(&row.wanted.name));
                        }
                        ui.add(
                            egui::Label::new(egui::RichText::new(&row.wanted.name).color(THEME.text_primary).strong())
                                .truncate(),
                        )
                        .on_hover_text(&row.wanted.package_id);
                    });
                    match &row.matches {
                        None => {
                            ui.spinner();
                        }
                        Some(_) if row.failed => {
                            tag(ui, t.get(TranslationKey::NameSearchFailed), THEME.accent_rose);
                        }
                        Some(matches) if matches.is_empty() => {
                            ui.label(
                                egui::RichText::new(t.get(TranslationKey::NameSearchNothing))
                                    .color(THEME.text_muted)
                                    .size(12.0),
                            );
                        }
                        Some(matches) => {
                            for m in matches {
                                ui.horizontal(|ui| {
                                    let sent = search.subscribed.contains(&m.id);
                                    let mut ticked = search.checked.contains(&m.id);
                                    ui.add_enabled_ui(!m.installed && !sent, |ui| {
                                        if ui.checkbox(&mut ticked, "").changed() {
                                            if ticked {
                                                search.checked.insert(m.id.clone());
                                            } else {
                                                search.checked.remove(&m.id);
                                            }
                                        }
                                    });
                                    if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                                        .on_hover_text(t.get(TranslationKey::NameSearchOpenSteam))
                                        .clicked()
                                    {
                                        open_external(crate::steam::workshop_url(&m.id));
                                    }
                                    // Tags first, so a long title is the part that gets cut.
                                    if m.exact {
                                        tag(ui, t.get(TranslationKey::NameSearchExact), THEME.accent_emerald);
                                    }
                                    if m.known {
                                        tag(ui, t.get(TranslationKey::NameSearchKnown), THEME.accent_cyan);
                                    }
                                    if m.installed {
                                        tag(ui, t.get(TranslationKey::NameSearchInstalled), THEME.text_muted);
                                    }
                                    if sent {
                                        tag(ui, t.get(TranslationKey::NameSearchSent), THEME.accent_emerald);
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&m.title).color(THEME.text_secondary).size(12.0),
                                        )
                                        .truncate(),
                                    );
                                });
                            }
                        }
                    }
                });
            }

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                let n = search.checked.len();
                let label = fmt(t.get(TranslationKey::NameSearchSubscribe), &[&n.to_string()]);
                if ui
                    .add_enabled_ui(n > 0 && !steam_busy, |ui| primary_button_icon(ui, ICON_STEAM, &label))
                    .inner
                    .clicked()
                {
                    subscribe = true;
                }
                if steam_busy {
                    ui.spinner();
                }
                if search.running
                    && secondary_button_icon(ui, ICON_STOP, t.get(TranslationKey::NameSearchStop)).clicked()
                {
                    stop = true;
                }
                if secondary_button(ui, t.get(TranslationKey::NameSearchClose)).clicked() {
                    close = true;
                }
            });
        });

    if subscribe {
        app.subscribe_name_search_checked();
    }
    if stop {
        app.stop_name_search();
    }
    if !open || close {
        app.close_name_search();
    }
}
