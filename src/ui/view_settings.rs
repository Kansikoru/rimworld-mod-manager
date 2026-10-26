use eframe::egui::{self, Ui};

use crate::app::App;
use crate::game_install::{known_launch_flags, GameEngine};
use crate::i18n::{fmt, Language, TranslationKey};
use crate::ui_prefs::{UiFont, TEXT_SCALE_RANGE};
use super::components::*;
use super::icons::*;
use super::theme::THEME;

pub fn render_settings_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    section_header(
        ui,
        t.get(TranslationKey::SettingsTitle),
        t.get(TranslationKey::SettingsSubtitle),
        None,
    );

    ui.add_space(8.0);

    // Language Card
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(format!("{}:", t.get(TranslationKey::SettingsLanguage)))
                    .color(THEME.text_secondary)
                    .strong(),
            );

            ui.horizontal(|ui| {
                if ui.selectable_label(app.is_russian, "Русский").clicked() {
                    app.set_language(Language::Russian);
                }
                if ui.selectable_label(!app.is_russian, "English").clicked() {
                    app.set_language(Language::English);
                }
            });
        });
    });

    ui.add_space(8.0);

    // Interface font & text size: egui draws text without hinting, so the most comfortable
    // font depends on the monitor. Changes apply immediately and are saved.
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(format!("{}:", t.get(TranslationKey::SettingsFont)))
                    .color(THEME.text_secondary)
                    .strong(),
            );
            let before = app.ui_prefs;
            ui.horizontal_wrapped(|ui| {
                for font in UiFont::ALL {
                    ui.selectable_value(&mut app.ui_prefs.font, font, font.label());
                }
            });
            ui.horizontal(|ui| {
                ui.label(t.get(TranslationKey::SettingsTextSize));
                ui.add(
                    egui::Slider::new(&mut app.ui_prefs.text_scale, TEXT_SCALE_RANGE)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            ui.label(
                egui::RichText::new(t.get(TranslationKey::SettingsFontHint))
                    .color(THEME.text_muted)
                    .size(12.0),
            );
            if app.ui_prefs != before {
                crate::fonts::setup_custom_fonts(ui.ctx(), &app.ui_prefs);
                if let Err(e) = app.ui_prefs.save() {
                    app.status_message = crate::i18n::tr!(
                        "Не удалось сохранить настройки шрифта: {}",
                        "Could not save font settings: {}",
                        e
                    );
                }
            }
        });
    });

    ui.add_space(8.0);

    // Update check (F2): a badge next to the version, never an automatic download.
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            let mut on = app.check_updates;
            if ui.checkbox(&mut on, t.get(TranslationKey::SettingsUpdateCheck)).changed() {
                app.check_updates = on;
                if on {
                    app.start_update_check();
                } else {
                    app.update_available = None;
                }
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(fmt(
                        t.get(TranslationKey::SettingsUpdateCheckHint),
                        &[crate::update_check::current_version()],
                    ))
                    .color(THEME.text_muted)
                    .size(12.0),
                )
                .wrap(),
            );
        });
    });

    ui.add_space(8.0);

    // Paths Configuration Card
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::SettingsPathsTitle))
                    .color(THEME.accent_cyan)
                    .size(15.0),
            );

            ui.add_space(6.0);

            // RimWorld Path
            ui.label(t.get(TranslationKey::SettingsRimWorldPath));
            ui.horizontal(|ui| {
                let path_str = app
                    .steam_paths
                    .game_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
                ui.label(
                    egui::RichText::new(path_str)
                        .color(THEME.text_muted)
                        .monospace(),
                );
                if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SettingsBrowse)).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        app.set_game_path(folder);
                    }
                }
            });
            if app.steam_paths.game_path.is_some() {
                let version = app
                    .steam_paths
                    .game_version
                    .clone()
                    .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
                ui.label(
                    egui::RichText::new(fmt(
                        t.get(TranslationKey::SettingsGameDetected),
                        &[&version, app.steam_paths.engine.label()],
                    ))
                    .color(THEME.accent_cyan)
                    .size(12.0),
                );
            }

            ui.add_space(6.0);

            // Workshop Path
            ui.label(t.get(TranslationKey::SettingsWorkshopPath));
            ui.horizontal(|ui| {
                let path_str = app
                    .steam_paths
                    .workshop_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
                ui.label(
                    egui::RichText::new(path_str)
                        .color(THEME.text_muted)
                        .monospace(),
                );
                if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SettingsBrowse)).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        app.set_workshop_path(folder);
                    }
                }
            });

            ui.add_space(6.0);

            // ModsConfig.xml Path
            ui.label(t.get(TranslationKey::SettingsConfigPath));
            ui.horizontal(|ui| {
                let path_str = app
                    .mods_config_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
                ui.label(
                    egui::RichText::new(path_str)
                        .color(THEME.text_muted)
                        .monospace(),
                );
                if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SettingsBrowse)).clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("ModsConfig.xml", &["xml"])
                        .pick_file()
                    {
                        app.set_mods_config_path(file);
                    }
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::SettingsAutoDetect)).clicked() {
                    app.reset_paths_to_auto();
                }
                ui.add_enabled_ui(app.steam_rx.is_none(), |ui| {
                    if secondary_button_icon(ui, ICON_STEAM, t.get(TranslationKey::SettingsCheckSteam)).clicked() {
                        app.check_steam_api();
                    }
                });
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::SettingsSavedHint))
                        .color(THEME.text_muted)
                        .size(12.0),
                );
            });
        });
    });

    ui.add_space(8.0);

    // Launch Game Parameters Card
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::SettingsLaunchTitle))
                    .color(THEME.accent_cyan)
                    .size(15.0),
            );

            ui.add_space(6.0);

            // Flags depend on the engine: Unity (1.6) and Waylith (1.7) take different arguments.
            ui.horizontal_wrapped(|ui| {
                for (flag, verified) in known_launch_flags(app.steam_paths.engine) {
                    let mut checked = app.launch_flags.contains(*flag);
                    let resp = ui.checkbox(&mut checked, *flag);
                    let resp = if *verified {
                        resp
                    } else {
                        resp.on_hover_text(t.get(TranslationKey::SettingsLaunchUnverified))
                    };
                    if resp.changed() {
                        if checked {
                            app.launch_flags.insert(flag.to_string());
                        } else {
                            app.launch_flags.remove(*flag);
                        }
                    }
                    ui.add_space(12.0);
                }
            });

            ui.horizontal(|ui| {
                ui.label(t.get(TranslationKey::SettingsLaunchExtraArgs));
                ui.add(egui::TextEdit::singleline(&mut app.launch_extra_args).desired_width(320.0));
            });
            if app.steam_paths.engine == GameEngine::Waylith {
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::SettingsLaunchWaylithHint))
                        .color(THEME.text_muted)
                        .size(12.0),
                );
            }

            ui.add_space(8.0);

            if primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::SettingsBtnLaunchRimWorld)).clicked() {
                app.launch_game();
            }
        });
    });

    ui.add_space(8.0);

    // Community Database Card
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::SettingsDbTitle))
                    .color(THEME.accent_purple)
                    .size(15.0),
            );

            ui.add_space(6.0);

            ui.label(t.get(TranslationKey::SettingsDbCount).replace("{}", &app.community_db_rules_count.to_string()));

            ui.label(
                egui::RichText::new(t.get(TranslationKey::SettingsDbHint))
                    .color(THEME.text_muted)
                    .size(12.0),
            );

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let downloading = app.rules_rx.is_some();
                ui.add_enabled_ui(!downloading, |ui| {
                    if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::SettingsBtnUpdateDb)).clicked() {
                        app.update_community_rules();
                    }
                    if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SettingsBtnImportDb)).clicked() {
                        let mut dialog = rfd::FileDialog::new().add_filter("communityRules.json", &["json"]);
                        if let Some(dir) = crate::community_rules::find_rimpy_rules_dir() {
                            dialog = dialog.set_directory(dir);
                        }
                        if let Some(file) = dialog.pick_file() {
                            app.import_community_rules(&file);
                        }
                    }
                });
                if downloading {
                    ui.spinner();
                }
            });
        });
    });
}
