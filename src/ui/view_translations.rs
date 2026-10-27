use eframe::egui::{self, Rounding, Stroke, Ui};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::i18n::{tr, TranslationKey};
use crate::models::{ModInfo, TranslationTargetLang};

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

        // Dropdown of the most common languages, most popular first.
        // The language can't change under a running modpack search.
        let lang_text = |l: TranslationTargetLang| format!("{} ({})", l.label(), l.code());
        let before = app.trans_target_lang;
        ui.add_enabled_ui(!app.is_batch_searching(), |ui| {
            egui::ComboBox::from_id_salt("translation_target_language")
                .selected_text(lang_text(app.trans_target_lang))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for lang in TranslationTargetLang::POPULAR {
                        ui.selectable_value(&mut app.trans_target_lang, lang, lang_text(lang));
                    }
                });
        });
        if app.trans_target_lang != before {
            // Results of another language would only confuse.
            app.batch_results.clear();
            app.batch_checked.clear();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::TransBtnDeleteAll)).clicked() {
                app.request_delete_translations();
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
                            ui.label(egui::RichText::new(&m.name).color(THEME.text_primary).strong());
                            if let Some(ref parent_id) = m.parent_package_id {
                                ui.label(
                                    egui::RichText::new(tr!("Перевод для: {}", "Translation of: {}", parent_id))
                                        .color(THEME.accent_cyan)
                                        .size(12.0),
                                );
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, ICON_TRASH, egui::vec2(14.0, 14.0), THEME.text_muted)
                                .on_hover_text(tr!("Удалить и отписаться", "Delete and unsubscribe"))
                                .clicked()
                            {
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
            egui::RichText::new(t.get(TranslationKey::TransWorkshopResultsTitle))
                .color(THEME.accent_cyan)
                .size(15.0),
        );
    });

    ui.add_space(4.0);

    // Modpack-wide search (main way) and search for the mod selected in the list.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if app.is_batch_searching() {
            if danger_button_icon(ui, ICON_STOP, tr!("Остановить", "Stop")).clicked() {
                app.stop_batch_translation_search();
            }
            let (done, total) = app.batch_progress;
            ui.add(
                egui::ProgressBar::new(if total > 0 { done as f32 / total as f32 } else { 0.0 })
                    .desired_width(240.0)
                    .fill(THEME.accent_purple)
                    .text(format!("{done} / {total}")),
            );
        } else {
            let lang = app.trans_target_lang.label();
            if primary_button_icon(ui, ICON_SEARCH, tr!("Найти переводы для сборки", "Find translations for my mods"))
                .on_hover_text(tr!(
                    "Ищет перевод на {} для каждого активного мода. Показывает только переводы, в названии которых есть название мода. Моды, где этот язык уже есть, пропускаются.",
                    "Looks for a {} translation of every active mod. Shows only translations whose title has the mod's name. Mods that already have this language are skipped.",
                    lang
                ))
                .clicked()
            {
                app.start_batch_translation_search();
            }
            let selected_name = app
                .selected_mod_id
                .as_deref()
                .and_then(|id| app.find_mod(id))
                .map(|m| m.name.clone());
            let hint = match &selected_name {
                Some(name) => tr!("Искать перевод только для «{}» (выбран на вкладке «Моды»)", "Search only for \"{}\" (selected on the Mods tab)", name),
                None => tr!("Сначала выберите мод на вкладке «Моды»", "Select a mod on the Mods tab first").to_string(),
            };
            let searching = app.workshop_rx.is_some();
            ui.add_enabled_ui(!searching && selected_name.is_some(), |ui| {
                if secondary_button(ui, tr!("Для выбранного мода", "For the selected mod"))
                    .on_hover_text(&hint)
                    .on_disabled_hover_text(&hint)
                    .clicked()
                {
                    app.search_workshop_translations_for_selected();
                }
            });
            if searching {
                ui.spinner();
            }
        }
    });

    ui.add_space(6.0);

    // Search bar for workshop
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // Leave room for the "Search" button (icon + label + padding), whatever the language.
        let button_w = 12.0 * 2.0 + 16.0 + 8.0 + badge_width(ui, t.get(TranslationKey::TransBtnSearch)) + 4.0;
        let field_w = (ui.available_width() - button_w - 16.0).max(120.0);
        search_input_width(
            ui,
            &mut app.workshop_search_query,
            t.get(TranslationKey::TransSearchPlaceholder),
            field_w,
        );
        let searching = app.workshop_rx.is_some();
        ui.add_enabled_ui(!searching, |ui| {
            if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::TransBtnSearch)).clicked() {
                app.search_workshop_custom();
            }
        });
    });

    ui.add_space(6.0);

    if app.show_batch_results {
        render_batch_results(app, ui);
        return;
    }
    if !app.batch_results.is_empty()
        && ui
            .link(tr!(
                "← Показать переводы для сборки",
                "← Back to translations for my mods"
            ))
            .clicked()
    {
        app.show_batch_results = true;
    }

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
                            ui.label(egui::RichText::new(&item.title).color(THEME.text_primary).strong());
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

/// Results of the modpack-wide search: each mod with its translations, ticked ones get
/// subscribed with one button.
fn render_batch_results(app: &mut App, ui: &mut Ui) {
    let lang = app.batch_lang.label();
    let (done, total) = app.batch_progress;
    let found_items: usize = app.batch_results.iter().map(|r| r.items.len()).sum();

    let mut summary = tr!(
        "Модов с переводом на {}: {} (проверено {} из {})",
        "Mods with a {} translation: {} (checked {} of {})",
        lang,
        app.batch_results.len(),
        done,
        total
    );
    if app.batch_skipped > 0 {
        summary.push_str(&tr!(
            " · пропущено {}: язык уже есть",
            " · {} skipped: language already there",
            app.batch_skipped
        ));
    }
    ui.label(egui::RichText::new(summary).color(THEME.text_secondary).size(12.0));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let n = app.batch_checked.len();
        ui.add_enabled_ui(n > 0 && app.steam_rx.is_none(), |ui| {
            if success_button_icon(
                ui,
                ICON_PLUS,
                &tr!("Подписаться на отмеченные ({})", "Subscribe to ticked ({})", n),
            )
            .clicked()
            {
                app.subscribe_checked_translations();
            }
        });
        if found_items > 0 && !app.is_batch_searching() {
            if small_button(ui, tr!("Отметить все", "Tick all"), ButtonStyle::Secondary).clicked() {
                for r in &app.batch_results {
                    for item in &r.items {
                        if !app.batch_subscribed.contains(&item.id) {
                            app.batch_checked.insert(item.id.clone());
                        }
                    }
                }
            }
            if small_button(ui, tr!("Снять все", "Untick all"), ButtonStyle::Secondary).clicked() {
                app.batch_checked.clear();
            }
        }
    });
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .id_salt("batch_translations_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.batch_results.is_empty() {
                card_container(ui, |ui| {
                    let text = if app.is_batch_searching() {
                        tr!(
                            "Ищу... Найденные переводы появятся здесь сразу.",
                            "Searching... Found translations show up here right away."
                        )
                    } else if total == 0 {
                        tr!(
                            "Искать нечего: у всех активных модов этот язык уже есть.",
                            "Nothing to search: every active mod already has this language."
                        )
                    } else {
                        tr!("Подходящих переводов не нашлось.", "No matching translations found.")
                    };
                    ui.label(egui::RichText::new(text).color(THEME.text_muted).size(13.0));
                });
                return;
            }
            for r in &app.batch_results {
                card_container(ui, |ui| {
                    ui.label(egui::RichText::new(&r.mod_name).color(THEME.accent_cyan).strong());
                    for item in &r.items {
                        ui.horizontal(|ui| {
                            if app.batch_subscribed.contains(&item.id) {
                                tag(ui, tr!("подписка отправлена", "subscribed"), THEME.accent_emerald);
                            } else {
                                let mut on = app.batch_checked.contains(&item.id);
                                if ui.checkbox(&mut on, "").changed() {
                                    if on {
                                        app.batch_checked.insert(item.id.clone());
                                    } else {
                                        app.batch_checked.remove(&item.id);
                                    }
                                }
                            }
                            ui.add(
                                egui::Label::new(egui::RichText::new(&item.title).color(THEME.text_primary)).truncate(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, ICON_STEAM, egui::vec2(14.0, 14.0), THEME.text_muted)
                                    .on_hover_text(tr!("Открыть в Мастерской", "Open on the Workshop"))
                                    .clicked()
                                {
                                    open_external(crate::steam::workshop_url(&item.id));
                                }
                            });
                        });
                    }
                });
            }
        });
}
