use eframe::egui::{self, ProgressBar, Ui};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::game_install::GameEngine;
use crate::i18n::{fmt, tr, TranslationKey};
use crate::optimizer::OptimizeScope;

pub fn render_optimizer_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    section_header(
        ui,
        t.get(TranslationKey::OptTitle),
        t.get(TranslationKey::OptSubtitle),
        None,
    );

    ui.add_space(8.0);

    // RimWorld 1.7 compresses and caches mod textures itself: conversion is optional there,
    // and resetting the game's own cache is often the useful action.
    // Long lines wrap inside the card instead of being cut off in a narrow window.
    if app.steam_paths.engine == GameEngine::Waylith {
        card_container(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(svg_icon(
                        ICON_STATUS_WARNING,
                        egui::Vec2::splat(18.0),
                        THEME.accent_amber,
                    ));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(t.get(TranslationKey::OptWaylithWarning))
                                .color(THEME.accent_amber)
                                .strong(),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if secondary_button_icon(ui, ICON_TRASH, t.get(TranslationKey::OptClearGameCache)).clicked() {
                        app.clear_game_caches();
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(t.get(TranslationKey::OptClearGameCacheHint))
                                .color(THEME.text_muted)
                                .size(12.0),
                        )
                        .wrap(),
                    );
                });
            });
        });
        ui.add_space(8.0);
    }

    // Optimizer DDS stored the wrong way up for this game version (1.6 and 1.7 read DDS
    // differently): flip them or roll them back.
    if app.opt_wrong_orientation > 0 {
        let version = app.steam_paths.game_major_minor().unwrap_or_else(|| "?".into());
        card_container(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(svg_icon(ICON_STATUS_ERROR, egui::Vec2::splat(18.0), THEME.accent_rose));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(fmt(
                                t.get(TranslationKey::OptLeftoverDds),
                                &[&version, &app.opt_wrong_orientation.to_string()],
                            ))
                            .color(THEME.accent_rose)
                            .strong(),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!app.optimizer_busy(), |ui| {
                        if primary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::OptBtnFixOrientation)).clicked()
                        {
                            app.fix_texture_orientation();
                        }
                        if secondary_button_icon(ui, ICON_UNDO, t.get(TranslationKey::OptBtnRollback)).clicked() {
                            app.rollback_textures();
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::OptLeftoverHint))
                        .color(THEME.text_muted)
                        .size(12.0),
                );
            });
        });
        ui.add_space(8.0);
    }

    // Main Control Card
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{}:", t.get(TranslationKey::OptScope)))
                        .color(THEME.text_secondary)
                        .strong(),
                );

                let scopes = [
                    (OptimizeScope::AllInstalled, t.get(TranslationKey::OptScopeAll)),
                    (OptimizeScope::ActiveOnly, t.get(TranslationKey::OptScopeActive)),
                    (OptimizeScope::SelectedOnly, t.get(TranslationKey::OptScopeSelected)),
                ];

                for (scope_val, label) in scopes {
                    let is_sel = app.opt_scope == scope_val;
                    if ui.selectable_label(is_sel, label).clicked() {
                        app.opt_scope = scope_val;
                    }
                }
            });

            ui.add_space(6.0);

            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut app.opt_force, t.get(TranslationKey::OptForceReencode));
                ui.add_space(12.0);
                ui.checkbox(&mut app.opt_delete_source, t.get(TranslationKey::OptDeleteSource));
            });
            if app.opt_delete_source {
                ui.add(egui::Label::new(
                    egui::RichText::new(tr!(
                        "Исходники переносятся в %LOCALAPPDATA%\\RimWorldModManager\\texture_backup и возвращаются кнопкой «Откатить».\nОставьте выключенным, если у вас достаточно места на диске: некоторые моды читают PNG напрямую.",
                        "Sources are moved to %LOCALAPPDATA%\\RimWorldModManager\\texture_backup and come back with \"Rollback\".\nLeave it off if you have enough disk space: some mods read PNG files directly."
                    ))
                        .color(THEME.accent_rose)
                        .size(12.0)
                ).wrap());
            }

            ui.add_space(6.0);

            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{}:", t.get(TranslationKey::OptThreads)))
                        .color(THEME.text_secondary),
                );
                ui.add(egui::Slider::new(&mut app.opt_threads, 1..=app.max_system_threads));
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::OptThreadsCount).replace("{}", &app.max_system_threads.to_string()))
                        .color(THEME.text_muted)
                        .size(12.0),
                );
            });

            ui.add_space(10.0);

            ui.horizontal_wrapped(|ui| {
                if !app.opt_is_running {
                    // Buttons are disabled while a scan/rollback runs in the background.
                    let idle = !app.optimizer_busy();
                    ui.add_enabled_ui(idle, |ui| {
                        if primary_button_icon(ui, ICON_ROCKET, t.get(TranslationKey::OptBtnStart)).clicked() {
                            app.start_texture_optimization();
                        }
                    });
                    ui.add_enabled_ui(idle, |ui| {
                        if secondary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::OptBtnScan)).clicked() {
                            app.scan_texture_counts();
                        }
                        if secondary_button_icon(ui, ICON_UNDO, t.get(TranslationKey::OptBtnRollback)).clicked() {
                            app.rollback_textures();
                        }
                    });
                    if !idle {
                        ui.spinner();
                    }
                } else {
                    if danger_button_icon(ui, ICON_STOP, t.get(TranslationKey::OptBtnCancel)).clicked() {
                        app.cancel_texture_optimization();
                    }
                }
            });
        });
    });

    ui.add_space(12.0);

    // Progress Card
    if app.opt_is_running || app.opt_total > 0 {
        card_container(ui, |ui| {
            ui.vertical(|ui| {
                let progress = if app.opt_total > 0 {
                    (app.opt_done as f32) / (app.opt_total as f32)
                } else {
                    0.0
                };

                let progress_pct = (progress * 100.0) as usize;

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tr!(
                            "Обработано: {} / {} ({}%)",
                            "Processed: {} / {} ({}%)",
                            app.opt_done,
                            app.opt_total,
                            progress_pct
                        ))
                        .color(THEME.text_primary)
                        .strong(),
                    );
                    if app.opt_errors > 0 {
                        badge(
                            ui,
                            &tr!("Ошибок: {}", "Errors: {}", app.opt_errors),
                            THEME.accent_rose.linear_multiply(0.2),
                            THEME.accent_rose,
                        );
                    }
                });

                ui.add_space(4.0);

                ui.add(
                    ProgressBar::new(progress)
                        .fill(THEME.accent_purple)
                        .show_percentage()
                        .animate(app.opt_is_running),
                );

                if !app.opt_current_file.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(tr!("Файл: {}", "File: {}", app.opt_current_file))
                            .color(THEME.text_muted)
                            .size(12.0),
                    );
                }
            });
        });
    }

    ui.add_space(8.0);

    // Informational card about RimWorld BC7 benefits
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(t.get(TranslationKey::OptDdsTitle))
                    .color(THEME.accent_cyan)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(t.get(TranslationKey::OptDdsDesc))
                    .color(THEME.text_secondary)
                    .size(12.0),
            );
        });
    });
}
