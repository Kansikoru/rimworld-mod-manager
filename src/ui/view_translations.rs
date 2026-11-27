use eframe::egui::{self, Ui};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::i18n::{fmt, tr, TranslationKey};
use crate::models::{ModInfo, TranslationTargetLang};

/// The tab's sections, listed on the left like the Settings sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Section {
    /// Translations for every active mod at once (the main way).
    #[default]
    Batch,
    /// A Workshop search by text, or for the mod selected on the Mods tab.
    Search,
    /// Translation mods already installed for the chosen language.
    Installed,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Batch => tr!("Для сборки", "For my mods", "为我的模组"),
            Section::Search => tr!("Поиск в Workshop", "Workshop search", "创意工坊搜索"),
            Section::Installed => tr!("Установленные", "Installed", "已安装"),
        }
    }

    fn icon(self) -> egui::ImageSource<'static> {
        match self {
            Section::Batch => ICON_MODS,
            Section::Search => ICON_SEARCH,
            Section::Installed => ICON_TRANSLATIONS,
        }
    }
}

const NAV_W: f32 = 244.0;

pub fn render_translations_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let section_id = egui::Id::unique("translations_section");
    let flag_id = egui::Id::unique("translations_batch_flag");
    let mut section: Section = ui.data(|d| d.get_temp(section_id).unwrap_or_default());
    // The app switches between the modpack results and a single search: follow it.
    let last_flag: Option<bool> = ui.data(|d| d.get_temp(flag_id));
    if last_flag.is_some_and(|f| f != app.translations.show_batch_results) {
        section = if app.translations.show_batch_results {
            Section::Batch
        } else {
            Section::Search
        };
    }
    ui.data_mut(|d| d.insert_temp(flag_id, app.translations.show_batch_results));

    let installed = app.translations_for_target_lang().len();
    let found: usize = app.translations.batch_results.iter().map(|r| r.items.len()).sum();

    egui::Panel::left("translations_nav")
        .resizable(false)
        .default_size(NAV_W)
        .size_range(NAV_W..=NAV_W)
        .show_separator_line(false)
        .frame(egui::Frame {
            outer_margin: egui::Margin {
                left: 0,
                right: 12,
                top: 0,
                bottom: 12,
            },
            inner_margin: egui::Margin::same(10),
            ..raised_frame(10)
        })
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(t.get(TranslationKey::TransTitle))
                    .color(THEME.text_primary)
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(t.get(TranslationKey::TransTargetLang))
                    .color(THEME.text_muted)
                    .size(11.0)
                    .strong(),
            );
            ui.add_space(2.0);
            // Dropdown of the most common languages, most popular first.
            // The language can't change under a running modpack search.
            let lang_text = |l: TranslationTargetLang| format!("{} ({})", l.label(), l.code());
            let before = app.translations.target_lang;
            ui.add_enabled_ui(!app.is_batch_searching(), |ui| {
                egui::ComboBox::from_id_salt("translation_target_language")
                    .selected_text(lang_text(app.translations.target_lang))
                    .width(ui.available_width() - 4.0)
                    .show_ui(ui, |ui| {
                        for lang in TranslationTargetLang::POPULAR {
                            ui.selectable_value(&mut app.translations.target_lang, lang, lang_text(lang));
                        }
                    });
            });
            if app.translations.target_lang != before {
                // Results of another language would only confuse.
                app.translations.batch_results.clear();
                app.translations.batch_checked.clear();
            }
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            side_nav_group(ui, tr!("ПЕРЕВОДЫ", "TRANSLATIONS", "翻译"), false);
            for item in [Section::Batch, Section::Search, Section::Installed] {
                let count = match item {
                    Section::Batch => (found > 0).then_some((found, true)),
                    Section::Search => (!app.translations.search_results.is_empty())
                        .then_some((app.translations.search_results.len(), false)),
                    Section::Installed => (installed > 0).then_some((installed, false)),
                };
                if side_nav_item(ui, item.icon(), item.title(), item == section, false, count).clicked() {
                    section = item;
                }
            }
        });
    ui.data_mut(|d| d.insert_temp(section_id, section));

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 4,
            bottom: 4,
        }))
        .show(ui, |ui| {
            cap_width(ui, 920.0);
            ui.label(
                egui::RichText::new(section.title())
                    .color(THEME.text_primary)
                    .size(20.0)
                    .strong(),
            );
            ui.add_space(8.0);
            match section {
                Section::Batch => render_batch_section(app, ui),
                Section::Search => render_search_section(app, ui),
                Section::Installed => render_installed_translations_column(app, ui),
            }
        });
}

fn render_installed_translations_column(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    // Same list that "Delete all translations" would act on: selected language only.
    let translation_mods: Vec<&ModInfo> = app.translations_for_target_lang();
    let mut delete_all = false;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} ({})",
                t.get(TranslationKey::TransFoundTitle),
                translation_mods.len()
            ))
            .color(THEME.text_muted),
        );
        if !translation_mods.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::TransBtnDeleteAll)).clicked() {
                    delete_all = true;
                }
            });
        }
    });
    ui.add_space(6.0);

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

            for m in translation_mods.iter() {
                let frame = egui::Frame {
                    inner_margin: egui::Margin::symmetric(12, 8),
                    outer_margin: egui::Margin::symmetric(0, 3),
                    shadow: egui::epaint::Shadow::NONE,
                    ..raised_frame(8)
                };

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            // Room for the Workshop ID badge and the delete button on the right.
                            ui.set_max_width((ui.available_width() - 130.0).max(120.0));
                            ui.add(
                                egui::Label::new(egui::RichText::new(&m.name).color(THEME.text_primary).strong())
                                    .wrap(),
                            );
                            if let Some(ref parent_id) = m.parent_package_id {
                                ui.label(
                                    egui::RichText::new(tr!(
                                        "Перевод для: {}",
                                        "Translation of: {}",
                                        "翻译对象：{}",
                                        parent_id
                                    ))
                                    .color(THEME.accent_cyan)
                                    .size(12.0),
                                );
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, ICON_TRASH, egui::vec2(14.0, 14.0), THEME.text_muted)
                                .on_hover_text(tr!("Удалить и отписаться", "Delete and unsubscribe", "删除并取消订阅"))
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
    if delete_all {
        app.request_delete_translations();
    }
}

/// The modpack-wide search: start / stop with its progress, then the results.
fn render_batch_section(app: &mut App, ui: &mut Ui) {
    // Modpack-wide search (main way) and search for the mod selected in the list.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if app.is_batch_searching() {
            if danger_button_icon(ui, ICON_STOP, tr!("Остановить", "Stop", "停止")).clicked() {
                app.stop_batch_translation_search();
            }
            let (done, total) = app.translations.batch_progress;
            ui.add(
                egui::ProgressBar::new(if total > 0 { done as f32 / total as f32 } else { 0.0 })
                    .desired_width(240.0)
                    .fill(THEME.accent_purple)
                    .text(format!("{done} / {total}")),
            );
        } else {
            let lang = app.translations.target_lang.label();
            if primary_button_icon(ui, ICON_SEARCH, tr!("Найти переводы для сборки", "Find translations for my mods", "为我的模组查找翻译"))
                .on_hover_text(tr!(
                    "Ищет перевод на {} для каждого активного мода. Показывает только переводы, в названии которых есть название мода. Моды, где этот язык уже есть, пропускаются.",
                    "Looks for a {} translation of every active mod. Shows only translations whose title has the mod's name. Mods that already have this language are skipped.", "为每个已启用的模组查找 {} 翻译。只显示标题中含有该模组名称的翻译。已经有该语言的模组会被跳过。",
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
                Some(name) => tr!("Искать перевод только для «{}» (выбран на вкладке «Моды»)", "Search only for \"{}\" (selected on the Mods tab)", "只搜索「{}」（在「模组」页中所选）", name),
                None => tr!("Сначала выберите мод на вкладке «Моды»", "Select a mod on the Mods tab first", "请先在「模组」页选择一个模组").to_string(),
            };
            let searching = app.translations.search_rx.is_some();
            if button_if(
                ui,
                !searching && selected_name.is_some(),
                None,
                tr!("Для выбранного мода", "For the selected mod", "针对所选模组"),
                ButtonStyle::Secondary,
            )
            .on_hover_text(&hint)
            .clicked()
            {
                app.search_workshop_translations_for_selected();
            }
            if searching {
                ui.spinner();
            }
        }
    });

    ui.add_space(10.0);
    render_batch_results(app, ui);
}

/// Search the Workshop by text (or for the mod selected on the Mods tab) and the results.
fn render_search_section(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    // Search bar for workshop
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // Leave room for the "Search" button (icon + label + padding), whatever the language.
        let button_w = 12.0 * 2.0 + 16.0 + 8.0 + badge_width(ui, t.get(TranslationKey::TransBtnSearch)) + 4.0;
        let field_w = (ui.available_width() - button_w - 16.0).max(120.0);
        search_input_width(
            ui,
            &mut app.translations.search_query,
            t.get(TranslationKey::TransSearchPlaceholder),
            field_w,
        );
        let searching = app.translations.search_rx.is_some();
        ui.add_enabled_ui(!searching, |ui| {
            if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::TransBtnSearch)).clicked() {
                app.search_workshop_custom();
            }
        });
    });

    ui.add_space(6.0);

    let mut to_subscribe_id = None;
    // "Download" when SteamCMD does it, "Subscribe" for the Steam client.
    let steamcmd = app.steamcmd_active();

    egui::ScrollArea::vertical()
        .id_salt("workshop_search_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.translations.search_results.is_empty() {
                card_container(ui, |ui| {
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::TransSearchHint))
                            .color(THEME.text_muted)
                            .size(13.0),
                    );
                });
                return;
            }

            for item in &app.translations.search_results {
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
                            let label = if steamcmd {
                                t.get(TranslationKey::TransBtnDownload)
                            } else {
                                t.get(TranslationKey::TransBtnSubscribe)
                            };
                            if success_button_icon(ui, ICON_PLUS, label).clicked() {
                                to_subscribe_id = Some(item.id.clone());
                            }
                            if secondary_button_icon(ui, ICON_STEAM, "Workshop").clicked() {
                                open_in_workshop_tab(ui.ctx(), &item.id);
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
    let lang = app.translations.batch_lang.label();
    let (done, total) = app.translations.batch_progress;
    let found_items: usize = app.translations.batch_results.iter().map(|r| r.items.len()).sum();

    let mut summary = tr!(
        "Модов с переводом на {}: {} (проверено {} из {})",
        "Mods with a {} translation: {} (checked {} of {})",
        "有 {} 翻译的模组：{}（已检查 {}/{}）",
        lang,
        app.translations.batch_results.len(),
        done,
        total
    );
    if app.translations.batch_skipped > 0 {
        summary.push_str(&tr!(
            " · пропущено {}: язык уже есть",
            " · {} skipped: language already there",
            " · 已跳过 {} 个：已有该语言",
            app.translations.batch_skipped
        ));
    }
    ui.label(egui::RichText::new(summary).color(THEME.text_secondary).size(12.0));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let n = app.translations.batch_checked.len();
        let key = if app.steamcmd_active() {
            TranslationKey::NameSearchDownload
        } else {
            TranslationKey::NameSearchSubscribe
        };
        let label = fmt(app.i18n.get(key), &[&n.to_string()]);
        ui.add_enabled_ui(n > 0 && !app.steam_busy(), |ui| {
            if success_button_icon(ui, ICON_PLUS, &label).clicked() {
                app.subscribe_checked_translations();
            }
        });
        if found_items > 0 && !app.is_batch_searching() {
            if small_button(ui, tr!("Отметить все", "Tick all", "全选"), ButtonStyle::Secondary).clicked()
            {
                for r in &app.translations.batch_results {
                    for item in &r.items {
                        if !app.translations.batch_subscribed.contains(&item.id) {
                            app.translations.batch_checked.insert(item.id.clone());
                        }
                    }
                }
            }
            if small_button(ui, tr!("Снять все", "Untick all", "全不选"), ButtonStyle::Secondary).clicked() {
                app.translations.batch_checked.clear();
            }
        }
    });
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .id_salt("batch_translations_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.translations.batch_results.is_empty() {
                card_container(ui, |ui| {
                    let text = if app.is_batch_searching() {
                        tr!(
                            "Ищу... Найденные переводы появятся здесь сразу.",
                            "Searching... Found translations show up here right away.",
                            "搜索中... 找到的翻译会立即显示在这里。"
                        )
                    } else if total == 0 {
                        tr!(
                            "Искать нечего: у всех активных модов этот язык уже есть.",
                            "Nothing to search: every active mod already has this language.",
                            "无需搜索：所有已启用的模组都已有该语言。"
                        )
                    } else {
                        tr!(
                            "Подходящих переводов не нашлось.",
                            "No matching translations found.",
                            "未找到匹配的翻译。"
                        )
                    };
                    ui.label(egui::RichText::new(text).color(THEME.text_muted).size(13.0));
                });
                return;
            }
            for r in &app.translations.batch_results {
                card_container(ui, |ui| {
                    ui.label(egui::RichText::new(&r.mod_name).color(THEME.accent_cyan).strong());
                    for item in &r.items {
                        ui.horizontal(|ui| {
                            if app.translations.batch_subscribed.contains(&item.id) {
                                tag(
                                    ui,
                                    tr!("подписка отправлена", "subscribed", "已订阅"),
                                    THEME.accent_emerald,
                                );
                            } else {
                                let mut on = app.translations.batch_checked.contains(&item.id);
                                if ui.checkbox(&mut on, "").changed() {
                                    if on {
                                        app.translations.batch_checked.insert(item.id.clone());
                                    } else {
                                        app.translations.batch_checked.remove(&item.id);
                                    }
                                }
                            }
                            ui.add(
                                egui::Label::new(egui::RichText::new(&item.title).color(THEME.text_primary)).truncate(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, ICON_STEAM, egui::vec2(14.0, 14.0), THEME.text_muted)
                                    .on_hover_text(tr!(
                                        "Открыть в Мастерской",
                                        "Open on the Workshop",
                                        "在创意工坊中打开"
                                    ))
                                    .clicked()
                                {
                                    open_in_workshop_tab(ui.ctx(), &item.id);
                                }
                            });
                        });
                    }
                });
            }
        });
}
