use eframe::egui::{self, ProgressBar, Ui};

use crate::app::App;
use crate::i18n::TranslationKey;
use crate::optimizer::OptimizeScope;
use super::components::*;
use super::icons::*;
use super::theme::THEME;

pub fn render_optimizer_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    section_header(
        ui,
        t.get(TranslationKey::OptTitle),
        t.get(TranslationKey::OptSubtitle),
        Some(("DirectDraw Surface DXT5 (BC3)", THEME.accent_emerald)),
    );

    ui.add_space(8.0);

    // Main Control Card
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
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

            ui.horizontal(|ui| {
                ui.checkbox(&mut app.opt_force, t.get(TranslationKey::OptForceReencode));
                ui.add_space(12.0);
                ui.checkbox(&mut app.opt_delete_source, t.get(TranslationKey::OptDeleteSource));
            });
            if app.opt_delete_source {
                ui.label(
                    egui::RichText::new("Внимание: Удаление оригиналов может сломать интерфейс в нестандартных модах.\nОставьте выключенным, если у вас достаточно места на диске.")
                        .color(THEME.accent_rose)
                        .size(12.0)
                );
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
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

            ui.horizontal(|ui| {
                if !app.opt_is_running {
                    if primary_button_icon(ui, ICON_ROCKET, t.get(TranslationKey::OptBtnStart)).clicked() {
                        app.start_texture_optimization();
                    }

                    if secondary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::OptBtnScan)).clicked() {
                        app.scan_texture_counts();
                    }

                    if secondary_button_icon(ui, ICON_UNDO, t.get(TranslationKey::OptBtnRollback)).clicked() {
                        app.rollback_textures();
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
                        egui::RichText::new(format!(
                            "Обработано: {} / {} ({}%)",
                            app.opt_done, app.opt_total, progress_pct
                        ))
                        .color(THEME.text_primary)
                        .strong(),
                    );
                    if app.opt_errors > 0 {
                        badge(
                            ui,
                            &format!("Ошибок: {}", app.opt_errors),
                            THEME.accent_rose.linear_multiply(0.2),
                            THEME.accent_rose,
                        );
                    }
                });

                ui.add_space(4.0);

                ui.add(
                    ProgressBar::new(progress)
                        .show_percentage()
                        .animate(app.opt_is_running),
                );

                if !app.opt_current_file.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Файл: {}", app.opt_current_file))
                            .color(THEME.text_muted)
                            .size(11.0),
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
