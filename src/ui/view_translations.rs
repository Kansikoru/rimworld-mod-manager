use eframe::egui::{self, Rounding, Stroke, Ui};

use crate::app::App;
use crate::i18n::{tr, TranslationKey};
use crate::models::{ModInfo, TranslationTargetLang};
use super::components::*;
use super::icons::*;
use super::theme::THEME;

pub fn render_translations_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    section_header(
        ui,
        t.get(TranslationKey::TransTitle),
        t.get(TranslationKey::TransSubtitle),
        None,
    );

    ui.add_space(8.0);

    // Filter and search tabs
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{}:", t.get(TranslationKey::TransTargetLang)))
                .color(THEME.text_secondary)
                .strong(),
        );

        let langs = [
            (TranslationTargetLang::Ru, "Русский (RU)"),
            (TranslationTargetLang::Zh, "中文 (ZH)"),
            (TranslationTargetLang::De, "Deutsch (DE)"),
            (TranslationTargetLang::Fr, "Français (FR)"),
        ];

        for (lang_val, label) in langs {
            let is_sel = app.trans_target_lang == lang_val;
            if ui.selectable_label(is_sel, label).clicked() {
                app.trans_target_lang = lang_val;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::TransBtnDeleteAll)).clicked() {
                app.request_delete_translations();
            }
            let searching = app.workshop_rx.is_some();
            ui.add_enabled_ui(!searching, |ui| {
                if secondary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::TransBtnFindWorkshop)).clicked() {
                    app.search_workshop_translations_for_selected();
                }
            });
            if searching {
                ui.spinner();
            }
        });
    });

    ui.add_space(8.0);

    // Two sections: Detected Translations in Collection (Left) & Workshop Search Results (Right)
    let available_width = ui.available_width();
    let col_width = (available_width - 16.0) / 2.0;

    ui.horizontal_top(|ui| {
        // Left Column: Detected Local Translation Mods
        ui.allocate_ui_with_layout(
            egui::Vec2::new(col_width, ui.available_height() - 20.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                render_installed_translations_column(app, ui);
            },
        );

        ui.separator();

        // Right Column: Steam Workshop Search & Recommendations
        ui.allocate_ui_with_layout(
            egui::Vec2::new(col_width, ui.available_height() - 20.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                render_workshop_search_column(app, ui);
            },
        );
    });
}

fn render_installed_translations_column(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    // Same list that "Delete all translations" would act on: selected language only.
    let translation_mods: Vec<&ModInfo> = app.translations_for_target_lang();

    ui.horizontal(|ui| {
        ui.heading(
            egui::RichText::new(format!(
                "{} ({})",
                t.get(TranslationKey::TransFoundTitle),
                translation_mods.len()
            ))
            .color(THEME.accent_purple)
            .size(15.0),
        );
    });

    ui.add_space(4.0);

    let mut to_delete_single = None;

    egui::ScrollArea::vertical()
        .id_salt("installed_trans_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if translation_mods.is_empty() {
                card_container(ui, |ui| {
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::TransCleanNotice))
                            .color(THEME.accent_emerald)
                            .size(13.0),
                    );
                });
                return;
            }

            for (idx, m) in translation_mods.iter().enumerate() {
                let frame = egui::Frame {
                    inner_margin: egui::Margin::symmetric(8.0, 6.0),
                    outer_margin: egui::Margin::symmetric(0.0, 2.0),
                    rounding: Rounding::same(6.0),
                    fill: if idx % 2 == 0 { THEME.bg_card } else { THEME.bg_surface },
                    stroke: Stroke::new(1.0_f32, THEME.border_subtle),
                    shadow: egui::epaint::Shadow::NONE,
                };

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&m.name)
                                    .color(THEME.text_primary)
                                    .strong(),
                            );
                            if let Some(ref parent_id) = m.parent_package_id {
                                ui.label(
                                    egui::RichText::new(tr!("Перевод для: {}", "Translation of: {}", parent_id))
                                        .color(THEME.accent_cyan)
                                        .size(12.0),
                                );
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, ICON_TRASH, egui::vec2(14.0, 14.0), THEME.text_muted).on_hover_text(tr!("Удалить и отписаться", "Delete and unsubscribe")).clicked() {
                                to_delete_single = Some(m.package_id.clone());
                            }
                            if let Some(ref sid) = m.steam_item_id {
                                badge(ui, sid, THEME.accent_cyan.linear_multiply(0.2), THEME.accent_cyan);
                            }
                        });
                    });
                });
            }
        });

    if let Some(pkg_id) = to_delete_single {
        app.request_delete_single(&pkg_id);
    }
}

fn render_workshop_search_column(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    ui.horizontal(|ui| {
        ui.heading(
            egui::RichText::new(format!(
                "{} ({})",
                t.get(TranslationKey::TransWorkshopResultsTitle),
                app.workshop_search_results.len()
            ))
            .color(THEME.accent_cyan)
            .size(15.0),
        );
    });

    ui.add_space(4.0);

    // Search bar for workshop
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.workshop_search_query)
                .hint_text(t.get(TranslationKey::TransSearchPlaceholder))
                .desired_width(ui.available_width() - 80.0),
        );
        let searching = app.workshop_rx.is_some();
        ui.add_enabled_ui(!searching, |ui| {
            if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::TransBtnSearch)).clicked() {
                app.search_workshop_custom();
            }
        });
    });

    ui.add_space(6.0);

    let mut to_subscribe_id = None;

    egui::ScrollArea::vertical()
        .id_salt("workshop_search_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.workshop_search_results.is_empty() {
                card_container(ui, |ui| {
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::TransSearchHint))
                            .color(THEME.text_muted)
                            .size(13.0),
                    );
                });
                return;
            }

            for item in &app.workshop_search_results {
                card_container(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&item.title)
                                    .color(THEME.text_primary)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!("Steam ID: {}", item.id))
                                    .color(THEME.text_muted)
                                    .size(12.0),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if success_button_icon(ui, ICON_PLUS, t.get(TranslationKey::TransBtnSubscribe)).clicked() {
                                to_subscribe_id = Some(item.id.clone());
                            }
                            if secondary_button_icon(ui, ICON_STEAM, "Workshop").clicked() {
                                open_external(crate::steam::workshop_url(&item.id));
                            }
                        });
                    });
                });
            }
        });

    if let Some(item_id) = to_subscribe_id {
        app.subscribe_steam_mod(&item_id);
    }
}
