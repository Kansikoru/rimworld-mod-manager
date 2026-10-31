use eframe::egui::{self, Ui};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::game_install::{known_launch_flags, GameEngine};
use crate::i18n::{fmt, Language, TranslationKey};
use crate::ui_prefs::{UiFont, TEXT_SCALE_RANGE};

/// A path and its "Browse…" button on one line: a long path is cut with "…" (the whole
/// path on hover) so the button always fits. Returns whether "Browse…" was clicked.
fn path_row(ui: &mut Ui, path: &str, browse: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let button_w = styled_button_width(ui, true, browse) + 8.0;
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - button_w).max(80.0));
            ui.add(egui::Label::new(egui::RichText::new(path).color(THEME.text_muted).monospace()).truncate())
                .on_hover_text(path);
        });
        clicked = secondary_button_icon(ui, ICON_FOLDER, browse).clicked();
    });
    clicked
}

/// A muted hint that wraps in a narrow window.
fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).color(THEME.text_muted).size(12.0)).wrap());
}

/// SteamCMD: on/off, where it lives, install or update, the last download's window.
fn render_steamcmd_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let dir = app.steamcmd_dir();
    let installed = crate::steamcmd::is_installed(&dir);
    let busy = app.steamcmd_busy();
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::SteamCmdCardTitle))
                    .color(THEME.accent_cyan)
                    .size(15.0),
            );
            ui.add_space(4.0);
            hint(ui, t.get(TranslationKey::SteamCmdCardHint));
            ui.add_space(6.0);
            if ui
                .checkbox(&mut app.use_steamcmd, t.get(TranslationKey::SteamCmdUse))
                .changed()
            {
                app.use_steamcmd_set = true;
            }
            if app.steam_paths.steam_path.is_none() && !app.use_steamcmd {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t.get(TranslationKey::SteamCmdAuto))
                            .color(THEME.accent_amber)
                            .size(12.0),
                    )
                    .wrap(),
                );
            }
            ui.add_space(6.0);
            if installed {
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::SteamCmdStatusInstalled))
                        .color(THEME.accent_emerald)
                        .size(12.0),
                );
            } else {
                hint(ui, t.get(TranslationKey::SteamCmdStatusMissing));
            }
            if path_row(ui, &dir.display().to_string(), t.get(TranslationKey::SteamCmdBtnPick)) {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    app.custom_steamcmd_dir = Some(folder);
                }
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::new(8.0, 8.0);
                let label = if installed {
                    t.get(TranslationKey::SteamCmdBtnUpdate)
                } else {
                    t.get(TranslationKey::SteamCmdBtnInstall)
                };
                if ui
                    .add_enabled_ui(!busy, |ui| secondary_button_icon(ui, ICON_STEAM, label))
                    .inner
                    .clicked()
                {
                    app.install_steamcmd();
                }
                if installed
                    && secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SteamCmdBtnFolder)).clicked()
                {
                    open_external(dir.clone());
                }
                if secondary_button(ui, t.get(TranslationKey::SteamCmdBtnConsole)).clicked() {
                    app.open_steamcmd_console();
                }
                if app.steamcmd_job.is_some()
                    && secondary_button(ui, t.get(TranslationKey::SteamCmdBtnWindow)).clicked()
                {
                    app.steamcmd_console = false;
                    app.steamcmd_window = true;
                }
                if busy {
                    ui.spinner();
                }
            });
        });
    });
}

/// Which tabs the top bar shows (U1): four can be hidden, the Log Analyzer can get a
/// short label or an icon, Settings an icon. Mods and Settings always stay.
fn render_tabs_card(app: &mut App, ui: &mut Ui) {
    use crate::ui_prefs::TabLabel;
    let t = app.i18n;
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(format!("{}:", t.get(TranslationKey::SettingsTabsTitle)))
                    .color(THEME.text_secondary)
                    .strong(),
            );
            let before = app.ui_prefs.tabs;
            let tabs = &mut app.ui_prefs.tabs;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for (key, hidden) in [
                    (TranslationKey::TabWorkshop, &mut tabs.hide_workshop),
                    (TranslationKey::TabSets, &mut tabs.hide_sets),
                    (TranslationKey::TabSaves, &mut tabs.hide_saves),
                    (TranslationKey::TabTranslations, &mut tabs.hide_translations),
                    (TranslationKey::TabOptimizer, &mut tabs.hide_optimizer),
                ] {
                    let mut shown = !*hidden;
                    if ui.checkbox(&mut shown, t.get(key)).changed() {
                        *hidden = !shown;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(t.get(TranslationKey::SettingsAnalyzerLabel));
                ui.selectable_value(
                    &mut tabs.analyzer_label,
                    TabLabel::Full,
                    t.get(TranslationKey::TabAnalyzer),
                );
                ui.selectable_value(
                    &mut tabs.analyzer_label,
                    TabLabel::Short,
                    t.get(TranslationKey::TabAnalyzerShort),
                );
                ui.selectable_value(
                    &mut tabs.analyzer_label,
                    TabLabel::Icon,
                    t.get(TranslationKey::SettingsTabIcon),
                );
            });
            ui.checkbox(&mut tabs.settings_icon, t.get(TranslationKey::SettingsSettingsIcon));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(t.get(TranslationKey::SettingsTabsHint))
                        .color(THEME.text_muted)
                        .size(12.0),
                )
                .wrap(),
            );
            if app.ui_prefs.tabs != before {
                if let Err(e) = app.ui_prefs.save() {
                    app.status_message = fmt(t.get(TranslationKey::SettingsTabsSaveError), &[&e.to_string()]);
                }
            }
        });
    });
}

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
                if ui
                    .selectable_label(app.i18n.language() == Language::Russian, "Русский")
                    .clicked()
                {
                    app.set_language(Language::Russian);
                }
                if ui
                    .selectable_label(app.i18n.language() == Language::English, "English")
                    .clicked()
                {
                    app.set_language(Language::English);
                }
                if ui
                    .selectable_label(app.i18n.language() == Language::Chinese, "简体中文")
                    .clicked()
                {
                    app.set_language(Language::Chinese);
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
                        "无法保存字体设置：{}",
                        e
                    );
                }
            }
        });
    });

    ui.add_space(8.0);
    render_tabs_card(app, ui);
    ui.add_space(8.0);

    // Update check (F2): a badge next to the version, never an automatic download.
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            let mut on = app.check_updates;
            if ui
                .checkbox(&mut on, t.get(TranslationKey::SettingsUpdateCheck))
                .changed()
            {
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
            let path_str = app
                .steam_paths
                .game_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
            if path_row(ui, &path_str, t.get(TranslationKey::SettingsBrowse)) {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    app.set_game_path(folder);
                }
            }
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
            let path_str = app
                .steam_paths
                .workshop_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
            if path_row(ui, &path_str, t.get(TranslationKey::SettingsBrowse)) {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    app.set_workshop_path(folder);
                }
            }

            ui.add_space(6.0);

            // ModsConfig.xml Path
            ui.label(t.get(TranslationKey::SettingsConfigPath));
            let path_str = app
                .mods_config_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
            if path_row(ui, &path_str, t.get(TranslationKey::SettingsBrowse)) {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("ModsConfig.xml", &["xml"])
                    .pick_file()
                {
                    app.set_mods_config_path(file);
                }
            }
            // Config outside the game's own AppData folder: "Play" points the game at it.
            if let Some(dir) = app.save_data_folder() {
                hint(
                    ui,
                    &fmt(
                        t.get(TranslationKey::SettingsSaveDataFolder),
                        &[&dir.display().to_string()],
                    ),
                );
            }

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::SettingsAutoDetect)).clicked() {
                    app.reset_paths_to_auto();
                }
                ui.add_enabled_ui(app.steam_rx.is_none(), |ui| {
                    if secondary_button_icon(ui, ICON_STEAM, t.get(TranslationKey::SettingsCheckSteam)).clicked() {
                        app.check_steam_api();
                    }
                });
            });
            hint(ui, t.get(TranslationKey::SettingsSavedHint));
        });
    });

    ui.add_space(8.0);
    render_steamcmd_card(app, ui);
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

            ui.horizontal_wrapped(|ui| {
                ui.label(t.get(TranslationKey::SettingsLaunchExtraArgs));
                let field_w = ui.available_width().clamp(160.0, 320.0);
                ui.add(egui::TextEdit::singleline(&mut app.launch_extra_args).desired_width(field_w));
            });
            if app.steam_paths.engine == GameEngine::Waylith {
                hint(ui, t.get(TranslationKey::SettingsLaunchWaylithHint));
            }

            ui.add_space(6.0);

            // Save backup before launch (F12)
            ui.horizontal_wrapped(|ui| {
                let mut backup_on = app.backup_save_before_launch;
                if ui
                    .checkbox(&mut backup_on, t.get(TranslationKey::SettingsBackupSaveBeforeLaunch))
                    .changed()
                {
                    app.backup_save_before_launch = backup_on;
                }
                if backup_on {
                    ui.add_space(8.0);
                    ui.label(t.get(TranslationKey::SettingsBackupSaveKeepCount));
                    let mut keep = app.save_backups_to_keep;
                    if ui.add(egui::DragValue::new(&mut keep).range(1..=50)).changed() {
                        app.save_backups_to_keep = keep;
                    }
                }
            });
            hint(ui, t.get(TranslationKey::SettingsBackupSaveHint));

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

            ui.label(
                t.get(TranslationKey::SettingsDbCount)
                    .replace("{}", &app.community_db_rules_count.to_string()),
            );

            hint(ui, t.get(TranslationKey::SettingsDbHint));

            ui.add_space(4.0);

            ui.horizontal_wrapped(|ui| {
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
