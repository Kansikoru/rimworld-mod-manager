use eframe::egui::{self, Ui};

use crate::app::App;
use crate::i18n::{Language, TranslationKey};
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
                if ui.selectable_label(app.is_russian, "🇷🇺 Русский").clicked() {
                    app.set_language(Language::Russian);
                }
                if ui.selectable_label(!app.is_russian, "🇬🇧 English").clicked() {
                    app.set_language(Language::English);
                }
            });
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
                        app.steam_paths.game_path = Some(folder);
                    }
                }
            });

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
                        app.steam_paths.workshop_path = Some(folder);
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
                        app.mods_config_path = Some(file);
                    }
                }
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

            ui.horizontal(|ui| {
                ui.checkbox(&mut app.launch_quicktest, "-quicktest");
                ui.add_space(12.0);
                ui.checkbox(&mut app.launch_logfile, "-logfile");
                ui.add_space(12.0);
                ui.checkbox(&mut app.launch_popupwindow, "-popupwindow");
            });

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

            ui.add_space(4.0);

            if secondary_button_icon(ui, ICON_REFRESH, t.get(TranslationKey::SettingsBtnUpdateDb)).clicked() {
                app.update_community_rules();
            }
        });
    });
}
