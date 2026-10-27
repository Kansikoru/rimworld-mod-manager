//! "Find the broken mod" window (F8): setup (which mods are never turned off), then one
//! step per game launch, then the result.

use eframe::egui::{self, Ui, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::bisect::{Bisect, Outcome, TestKind};
use crate::i18n::{fmt, TranslationKey};

const WIDTH: f32 = 580.0;

pub fn render_bisect_window(app: &mut App, ctx: &egui::Context) {
    if !app.bisect_open {
        return;
    }
    let t = app.i18n;
    let mut open = true;
    let (width, max_height) = dialog_size(ctx, WIDTH);
    egui::Window::new(t.get(TranslationKey::BisectTitle))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .max_height(max_height)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.set_width(width);
            if app.bisect.is_some() {
                render_search(app, ui);
            } else {
                render_setup(app, ui);
            }
            if let Some(note) = app.bisect_note.clone() {
                ui.add_space(6.0);
                ui.add(egui::Label::new(egui::RichText::new(note).color(THEME.accent_rose)).wrap());
            }
        });
    if !open {
        // The search itself is kept: it can be continued from the header or the analyzer.
        app.close_bisect_window();
    }
}

fn muted(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).color(THEME.text_muted).size(12.0)).wrap());
}

fn render_setup(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    ui.add(
        egui::Label::new(egui::RichText::new(t.get(TranslationKey::BisectIntro)).color(THEME.text_secondary)).wrap(),
    );
    ui.add_space(10.0);

    let mut keep = app.bisect_keep.clone().unwrap_or_default();
    let order = app.saved_active_mods.clone();
    ui.label(
        egui::RichText::new(fmt(t.get(TranslationKey::BisectKeepTitle), &[&keep.len().to_string()]))
            .color(THEME.accent_cyan)
            .strong(),
    );
    muted(ui, t.get(TranslationKey::BisectKeepHint));
    ui.add_space(4.0);
    let field_w = ui.available_width();
    search_input_width(ui, &mut app.bisect_filter, t.get(TranslationKey::BisectFilter), field_w);
    ui.add_space(4.0);
    let query = app.bisect_filter.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("bisect_keep_scroll")
        // Shorter in a short window, so "Start" stays within reach.
        .max_height((ui.ctx().screen_rect().height() * 0.3).clamp(100.0, 240.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for id in &order {
                let name = app.mod_display_name(id);
                if !query.is_empty() && !name.to_lowercase().contains(&query) && !id.contains(&query) {
                    continue;
                }
                let mut on = keep.contains(id);
                if ui.checkbox(&mut on, egui::RichText::new(&name).size(12.5)).changed() {
                    if on {
                        keep.insert(id.clone());
                    } else {
                        keep.remove(id);
                    }
                }
            }
        });

    // What the search will look like with these ticks.
    let keep_list: Vec<String> = keep.iter().cloned().collect();
    let preview = Bisect::new(&order, &keep_list, &app.bisect_requirements(&order));
    let suspects = preview.suspects_left();
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(fmt(
            t.get(TranslationKey::BisectEstimate),
            &[&suspects.to_string(), &preview.launches_left().to_string()],
        ))
        .color(THEME.text_primary),
    );
    if app.has_unsaved_changes() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(t.get(TranslationKey::BisectUnsaved))
                    .color(THEME.accent_amber)
                    .size(12.0),
            )
            .wrap(),
        );
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let can_start = suspects > 0 && !app.has_unsaved_changes();
        if ui
            .add_enabled_ui(can_start, |ui| {
                primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::BisectStart))
            })
            .inner
            .clicked()
        {
            app.bisect_keep = Some(keep.clone());
            app.start_bisect();
        }
        if secondary_button(ui, t.get(TranslationKey::BtnCancel)).clicked() {
            app.close_bisect_window();
        }
    });
    if app.bisect.is_none() && app.bisect_open {
        app.bisect_keep = Some(keep);
    }
}

fn render_search(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let Some(search) = app.bisect.clone() else { return };
    if let Some(outcome) = search.outcome().cloned() {
        render_result(app, ui, &search, &outcome);
        return;
    }
    let Some(test) = search.stage.test.clone() else { return };

    ui.label(
        egui::RichText::new(fmt(
            t.get(TranslationKey::BisectProgress),
            &[
                &(search.stage.answers + 1).to_string(),
                &search.suspects_left().to_string(),
                &search.launches_left().to_string(),
            ],
        ))
        .color(THEME.accent_cyan)
        .strong(),
    );
    if search.searching_group() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(t.get(TranslationKey::BisectGroupNote))
                    .color(THEME.accent_amber)
                    .size(12.0),
            )
            .wrap(),
        );
    }
    ui.add_space(6.0);

    let enabled = search.enabled();
    let what = if test.kind == TestKind::BaseOnly {
        fmt(t.get(TranslationKey::BisectBaseTest), &[&enabled.len().to_string()])
    } else {
        fmt(
            t.get(TranslationKey::BisectTestOn),
            &[&test.part.len().to_string(), &enabled.len().to_string()],
        )
    };
    ui.add(egui::Label::new(egui::RichText::new(what).color(THEME.text_primary)).wrap());
    if !test.part.is_empty() {
        egui::CollapsingHeader::new(egui::RichText::new(t.get(TranslationKey::BisectShowMods)).size(12.0))
            .id_salt(("bisect_tested", search.stage.answers))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("bisect_tested_scroll")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for id in &test.part {
                            ui.label(
                                egui::RichText::new(app.mod_display_name(id))
                                    .size(12.0)
                                    .color(THEME.text_secondary),
                            );
                        }
                    });
            });
    }
    ui.add_space(8.0);

    // 1. Write the list (and start the game)
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        if primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::BisectApplyLaunch)).clicked() {
            app.bisect_apply(true);
        }
        if secondary_button(ui, t.get(TranslationKey::BisectApplyOnly))
            .on_hover_text(t.get(TranslationKey::BisectApplyOnlyHint))
            .clicked()
        {
            app.bisect_apply(false);
        }
    });
    let applied = app.bisect.as_ref().is_some_and(|s| s.applied);
    if applied {
        ui.label(
            egui::RichText::new(t.get(TranslationKey::BisectApplied))
                .color(THEME.accent_emerald)
                .size(12.0),
        );
    }
    ui.add_space(10.0);

    // 2. The answer
    ui.label(
        egui::RichText::new(t.get(TranslationKey::BisectQuestion))
            .color(THEME.text_primary)
            .strong(),
    );
    muted(ui, t.get(TranslationKey::BisectHintCrash));
    ui.add_space(4.0);
    ui.add_enabled_ui(applied, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if danger_button(ui, t.get(TranslationKey::BisectYes)).clicked() {
                app.bisect_answer(true);
            }
            if success_button(ui, t.get(TranslationKey::BisectNo)).clicked() {
                app.bisect_answer(false);
            }
        });
    });
    if !applied {
        muted(ui, t.get(TranslationKey::BisectApplyFirst));
    }
    ui.add_space(10.0);
    ui.separator();
    footer(app, ui, &search);
}

fn footer(app: &mut App, ui: &mut Ui, search: &Bisect) {
    let t = app.i18n;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if ui
            .add_enabled_ui(!search.history.is_empty(), |ui| {
                secondary_button_icon(ui, ICON_UNDO, t.get(TranslationKey::BisectStepBack))
            })
            .inner
            .clicked()
        {
            app.bisect_step_back();
        }
        if secondary_button(ui, t.get(TranslationKey::BisectAbort))
            .on_hover_text(t.get(TranslationKey::BisectAbortHint))
            .clicked()
        {
            app.bisect_finish(false);
        }
    });
}

fn render_result(app: &mut App, ui: &mut Ui, search: &Bisect, outcome: &Outcome) {
    let t = app.i18n;
    match outcome {
        Outcome::InBase => {
            ui.label(
                egui::RichText::new(t.get(TranslationKey::BisectInBaseTitle))
                    .color(THEME.accent_amber)
                    .size(16.0)
                    .strong(),
            );
            ui.add(
                egui::Label::new(egui::RichText::new(t.get(TranslationKey::BisectInBase)).color(THEME.text_secondary))
                    .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary_button(ui, t.get(TranslationKey::BisectRestoreBtn)).clicked() {
                    app.bisect_finish(false);
                }
                if ui
                    .add_enabled_ui(!search.history.is_empty(), |ui| {
                        secondary_button_icon(ui, ICON_UNDO, t.get(TranslationKey::BisectStepBack))
                    })
                    .inner
                    .clicked()
                {
                    app.bisect_step_back();
                }
            });
        }
        Outcome::Found(found) => {
            let title = match found.len() {
                1 => t.get(TranslationKey::BisectFoundOne),
                2 => t.get(TranslationKey::BisectFoundPair),
                _ => t.get(TranslationKey::BisectFoundGroup),
            };
            ui.label(egui::RichText::new(title).color(THEME.accent_rose).size(16.0).strong());
            ui.add_space(4.0);
            for id in found {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(app.mod_display_name(id))
                            .color(THEME.text_primary)
                            .size(15.0)
                            .strong(),
                    );
                    ui.label(egui::RichText::new(id).color(THEME.text_muted).size(11.0));
                    if let Some(steam_id) = app.find_mod(id).and_then(|m| m.steam_item_id.clone()) {
                        if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                            .on_hover_text(t.get(TranslationKey::SetsOpenWorkshop))
                            .clicked()
                        {
                            open_external(crate::steam::workshop_url(&steam_id));
                        }
                    }
                });
            }
            ui.add_space(4.0);
            muted(
                ui,
                &fmt(
                    t.get(TranslationKey::BisectLaunches),
                    &[&search.stage.answers.to_string()],
                ),
            );
            let dependents = search.dependents_of(found);
            if !dependents.is_empty() {
                ui.add_space(6.0);
                let names: Vec<String> = dependents.iter().map(|id| app.mod_display_name(id)).collect();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(fmt(t.get(TranslationKey::BisectDependents), &[&names.join(", ")]))
                            .color(THEME.accent_amber)
                            .size(12.0),
                    )
                    .wrap(),
                );
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                if primary_button(ui, t.get(TranslationKey::BisectTurnOffBtn)).clicked() {
                    app.bisect_finish(true);
                }
                if secondary_button(ui, t.get(TranslationKey::BisectRestoreBtn)).clicked() {
                    app.bisect_finish(false);
                }
                if secondary_button_icon(ui, ICON_UNDO, t.get(TranslationKey::BisectStepBack)).clicked() {
                    app.bisect_step_back();
                }
            });
        }
    }
}

/// The card on the Log Analyzer tab that opens the search.
pub fn render_bisect_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 220.0).max(200.0));
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::BisectCardTitle))
                        .color(THEME.accent_cyan)
                        .strong(),
                );
                muted(ui, t.get(TranslationKey::BisectCardText));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if app.bisect.is_some() {
                    t.get(TranslationKey::BisectCardContinue)
                } else {
                    t.get(TranslationKey::BisectCardBtn)
                };
                if primary_button_icon(ui, ICON_SEARCH, label).clicked() {
                    app.open_bisect();
                }
            });
        });
    });
}
