use eframe::egui::{self, CornerRadius, Ui, Vec2};

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
    full_card(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::SteamCmdCardTitle))
                    .color(THEME.text_primary)
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(4.0);
            hint(ui, t.get(TranslationKey::SteamCmdCardHint));
            ui.add_space(6.0);
            if setting_row(ui, t.get(TranslationKey::SteamCmdUse), None, &mut app.use_steamcmd) {
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
            setting_row(
                ui,
                t.get(TranslationKey::SettingsWsViaSteamClient),
                Some(t.get(TranslationKey::SettingsWsViaSteamClientHint)),
                &mut app.workshop_via_steam_client,
            );
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

/// The companion mod (F11): what it is, whether it is in the game, install / remove.
fn render_companion_card(app: &mut App, ui: &mut Ui) {
    use crate::companion::State;
    let t = app.i18n;
    let state = app.companion_state();
    let enabled = app
        .active_mods
        .iter()
        .any(|id| id.eq_ignore_ascii_case(crate::companion::PACKAGE_ID));
    full_card(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::CompanionTitle))
                    .color(THEME.text_primary)
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(4.0);
            hint(ui, t.get(TranslationKey::CompanionHint));
            ui.add_space(6.0);
            let (status, color) = match (state, enabled) {
                (None, _) => (t.get(TranslationKey::SteamCmdNoMods), THEME.accent_amber),
                (Some(State::Missing), _) => (t.get(TranslationKey::CompanionStatusMissing), THEME.text_muted),
                (Some(_), false) => (t.get(TranslationKey::CompanionStatusOff), THEME.accent_amber),
                (Some(State::Outdated), true) => (t.get(TranslationKey::CompanionStatusOutdated), THEME.accent_amber),
                (Some(State::Current), true) => (t.get(TranslationKey::CompanionStatusOn), THEME.accent_emerald),
            };
            ui.add(egui::Label::new(egui::RichText::new(status).color(color).size(12.0)).wrap());
            let Some(state) = state else { return };
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::new(8.0, 8.0);
                let install_label = match state {
                    State::Missing => Some(TranslationKey::CompanionBtnInstall),
                    State::Outdated => Some(TranslationKey::CompanionBtnUpdate),
                    State::Current if !enabled => Some(TranslationKey::CompanionBtnEnable),
                    State::Current => None,
                };
                if let Some(key) = install_label {
                    if primary_button(ui, t.get(key)).clicked() {
                        app.install_companion();
                    }
                }
                if state != State::Missing {
                    if let Some(mods_dir) = app.game_mods_dir() {
                        if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SteamCmdBtnFolder)).clicked() {
                            open_external(crate::companion::install_dir(&mods_dir));
                        }
                    }
                    if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::CompanionBtnRemove)).clicked() {
                        app.remove_companion();
                    }
                }
            });
        });
    });
}

/// Which tabs the top bar shows (U1): four can be hidden, the Log Analyzer can get a
/// short label or an icon, Settings an icon. Mods and Settings always stay.
fn render_tabs_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    full_card(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(format!("{}:", t.get(TranslationKey::SettingsTabsTitle)))
                    .color(THEME.text_secondary)
                    .strong(),
            );
            let before = app.ui_prefs.tabs;
            let names: Vec<(crate::app::AppTab, &str)> = crate::app::AppTab::ALL
                .into_iter()
                .filter(|tab| *tab != crate::app::AppTab::Settings)
                .map(|tab| (tab, app.tab_name(tab)))
                .collect();
            let tabs = &mut app.ui_prefs.tabs;
            // A switch per tab; Settings always stays (hidden tabs come back from here).
            for (tab, name) in names {
                use crate::app::AppTab;
                let hidden = match tab {
                    AppTab::Mods => &mut tabs.hide_mods,
                    AppTab::Workshop => &mut tabs.hide_workshop,
                    AppTab::Sets => &mut tabs.hide_sets,
                    AppTab::Saves => &mut tabs.hide_saves,
                    AppTab::Translations => &mut tabs.hide_translations,
                    AppTab::Optimizer => &mut tabs.hide_optimizer,
                    AppTab::Analyzer => &mut tabs.hide_analyzer,
                    AppTab::Settings => continue,
                };
                let mut shown = !*hidden;
                if setting_row(ui, name, None, &mut shown) {
                    *hidden = !shown;
                }
            }
            ui.add_space(6.0);
            setting_row(
                ui,
                crate::i18n::tr!("Названия в боковой панели", "Names in the side bar", "侧边栏显示名称"),
                Some(crate::i18n::tr!(
                    "Выключено — только значки, название видно при наведении. В узком окне значки всегда без названий.",
                    "Off: icons only, the name shows on hover. A narrow window always shows icons only.",
                    "关闭时只显示图标，悬停显示名称。窄窗口始终只显示图标。"
                )),
                &mut tabs.sidebar_labels,
            );
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

/// A card that takes the whole content width, so every card on a page has the same edges.
fn full_card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    card_container(ui, |ui| {
        ui.set_width(ui.available_width());
        add_contents(ui)
    })
}

/// An on/off switch: a pill with a round knob, the accent color when on.
fn toggle_switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let off_fill = if response.hovered() {
            THEME.border_strong.gamma_multiply(1.3)
        } else {
            THEME.border_strong
        };
        let fill = off_fill.lerp_to_gamma(THEME.accent_cyan, t);
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, CornerRadius::same(radius as u8), fill);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, THEME.text_primary);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One setting as a row: bold title, a muted line under it, the switch on the right.
/// Returns whether the switch was flipped.
fn setting_row(ui: &mut Ui, title: &str, hint_text: Option<&str>, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let text_w = (ui.available_width() - 56.0).max(120.0);
        ui.allocate_ui_with_layout(Vec2::new(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(text_w);
            ui.add(egui::Label::new(egui::RichText::new(title).color(THEME.text_primary).strong()).wrap());
            if let Some(h) = hint_text {
                hint(ui, h);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = toggle_switch(ui, on).changed();
        });
    });
    ui.add_space(6.0);
    changed
}

/// A sub-heading inside a section ("Startup", "Paths", ...).
fn group_title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(THEME.text_primary).size(15.0).strong());
    ui.add_space(4.0);
}

/// The settings sections, listed on the left like a settings dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Appearance,
    Tabs,
    Paths,
    Launch,
    Steam,
    Companion,
    Order,
    Mods,
    About,
}

impl Section {
    const ALL: [Section; 9] = [
        Section::Appearance,
        Section::Tabs,
        Section::Paths,
        Section::Launch,
        Section::Steam,
        Section::Companion,
        Section::Order,
        Section::Mods,
        Section::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Appearance => crate::i18n::tr!("Внешний вид", "Appearance", "外观"),
            Section::Tabs => crate::i18n::tr!("Вкладки", "Tabs", "标签页"),
            Section::Paths => crate::i18n::tr!("Папки и файлы", "Folders and files", "文件夹和文件"),
            Section::Launch => crate::i18n::tr!("Запуск игры", "Game launch", "启动游戏"),
            Section::Steam => crate::i18n::tr!("Steam и SteamCMD", "Steam and SteamCMD", "Steam 和 SteamCMD"),
            Section::Companion => crate::i18n::tr!("Мод-компаньон", "Companion mod", "伴侣模组"),
            Section::Order => crate::i18n::tr!("Порядок загрузки", "Load order", "加载顺序"),
            Section::Mods => crate::i18n::tr!("Версии и копии", "Versions and copies", "版本和副本"),
            Section::About => crate::i18n::tr!("Программа", "App", "程序"),
        }
    }

    fn icon(self) -> egui::ImageSource<'static> {
        match self {
            Section::Appearance => ICON_CAT_RETEXTURE,
            Section::Tabs => ICON_MODS,
            Section::Paths => ICON_FOLDER,
            Section::Launch => ICON_PLAY,
            Section::Steam => ICON_STEAM,
            Section::Companion => ICON_CAT_FRAMEWORK,
            Section::Order => ICON_SORT,
            Section::Mods => ICON_HISTORY,
            Section::About => ICON_INFO,
        }
    }
}

/// The left list: section groups with their items, the app's version at the bottom.
fn groups() -> [(&'static str, &'static [Section]); 4] {
    [
        (
            crate::i18n::tr!("ИНТЕРФЕЙС", "INTERFACE", "界面"),
            &[Section::Appearance, Section::Tabs],
        ),
        (
            crate::i18n::tr!("ИГРА", "GAME", "游戏"),
            &[Section::Paths, Section::Launch, Section::Steam, Section::Companion],
        ),
        (
            crate::i18n::tr!("МОДЫ", "MODS", "模组"),
            &[Section::Order, Section::Mods],
        ),
        (crate::i18n::tr!("ОБЩЕЕ", "GENERAL", "常规"), &[Section::About]),
    ]
}

/// One item of the left list: icon and name; the chosen one on an accent-tinted pill.
fn nav_item(ui: &mut Ui, section: Section, selected: bool) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 32.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), THEME.accent_cyan.gamma_multiply(0.18));
        } else if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), THEME.bg_surface_hover);
        }
        let color = if selected {
            THEME.accent_cyan
        } else if hovered {
            THEME.text_primary
        } else {
            THEME.text_secondary
        };
        let icon_rect =
            egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(16.0));
        egui::Image::new(section.icon()).tint(color).paint_at(ui, icon_rect);
        ui.painter().text(
            egui::pos2(rect.left() + 38.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            section.title(),
            egui::FontId::proportional(13.5),
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Where the open section is kept (egui's memory, not the app).
fn section_id() -> egui::Id {
    egui::Id::unique("settings_section")
}

/// The sections for the app-wide search: (index, name).
pub fn search_sections() -> Vec<(usize, &'static str)> {
    Section::ALL.iter().enumerate().map(|(i, s)| (i, s.title())).collect()
}

/// Opens a section by its index from `search_sections` (the tab itself is switched by the caller).
pub fn open_section(ctx: &egui::Context, index: usize) {
    if let Some(section) = Section::ALL.get(index) {
        ctx.data_mut(|d| d.insert_temp(section_id(), *section));
    }
}

pub fn render_settings_view(app: &mut App, ui: &mut Ui) {
    let section_id = section_id();
    let mut section: Section = ui.data(|d| d.get_temp(section_id).unwrap_or_default());

    egui::Panel::left("settings_nav")
        .resizable(false)
        .default_size(244.0)
        .size_range(244.0..=244.0)
        .show_separator_line(false)
        .frame(
            // Raised like the cards; the outer margin leaves room for its shadow (the panel
            // clips at its own edge).
            egui::Frame {
                outer_margin: egui::Margin {
                    left: 0,
                    right: 12,
                    top: 0,
                    bottom: 12,
                },
                inner_margin: egui::Margin::same(10),
                ..raised_frame(10)
            },
        )
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(app.i18n.get(TranslationKey::SettingsTitle))
                    .color(THEME.text_primary)
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(10.0);
            for (name, items) in groups() {
                ui.label(egui::RichText::new(name).color(THEME.text_muted).size(11.0).strong());
                ui.add_space(2.0);
                for &item in items {
                    if nav_item(ui, item, item == section).clicked() {
                        section = item;
                    }
                }
                ui.add_space(10.0);
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
            egui::ScrollArea::vertical()
                .id_salt(("settings_section_scroll", section as u8))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(780.0);
                    ui.label(
                        egui::RichText::new(section.title())
                            .color(THEME.text_primary)
                            .size(20.0)
                            .strong(),
                    );
                    ui.add_space(10.0);
                    match section {
                        Section::Appearance => appearance_section(app, ui),
                        Section::Tabs => render_tabs_card(app, ui),
                        Section::Paths => paths_section(app, ui),
                        Section::Launch => launch_section(app, ui),
                        Section::Steam => render_steamcmd_card(app, ui),
                        Section::Companion => render_companion_card(app, ui),
                        Section::Order => {
                            community_db_card(app, ui);
                            ui.add_space(8.0);
                            own_order_rules_card(app, ui);
                        }
                        Section::Mods => mods_section(app, ui),
                        Section::About => about_section(app, ui),
                    }
                    ui.add_space(12.0);
                });
        });
}

/// Language, theme cards with a preview, accent, font and text size.
fn appearance_section(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsLanguage));
        let mut lang = app.i18n.language();
        if segmented(
            ui,
            &mut lang,
            &[
                (Language::Russian, "Русский"),
                (Language::English, "English"),
                (Language::Chinese, "简体中文"),
            ],
        ) {
            app.set_language(lang);
        }
    });

    ui.add_space(8.0);

    // Theme and accent palette: apply at once and are saved with the font.
    full_card(ui, |ui| {
        use crate::ui::theme::{Accent, ThemeKind};
        group_title(ui, t.get(TranslationKey::SettingsTheme));
        let before = (app.ui_prefs.theme, app.ui_prefs.accent);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
            for theme in ThemeKind::ALL {
                if theme_card(ui, theme, t.get(theme.key()), app.ui_prefs.theme == theme).clicked() {
                    app.ui_prefs.theme = theme;
                }
            }
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(t.get(TranslationKey::SettingsAccent))
                    .color(THEME.text_primary)
                    .strong(),
            );
            for accent in Accent::ALL {
                let selected = app.ui_prefs.accent == accent;
                let color = accent.swatch(app.ui_prefs.theme);
                // A round swatch in the palette's color; the chosen one gets a ring.
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::click());
                let resp = resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(t.get(accent.key()));
                let r = if resp.hovered() { 9.0 } else { 8.0 };
                ui.painter().circle_filled(rect.center(), r, color);
                if selected {
                    ui.painter()
                        .circle_stroke(rect.center(), 11.0, egui::Stroke::new(1.5, THEME.text_primary));
                }
                if resp.clicked() {
                    app.ui_prefs.accent = accent;
                }
            }
        });
        hint(ui, t.get(TranslationKey::SettingsThemeHint));
        if (app.ui_prefs.theme, app.ui_prefs.accent) != before {
            crate::ui::theme::set_theme(app.ui_prefs.theme, app.ui_prefs.accent);
            crate::ui::theme::apply_theme(ui.ctx());
            if let Err(e) = app.ui_prefs.save() {
                app.status_message = fmt(t.get(TranslationKey::SettingsTabsSaveError), &[&e.to_string()]);
            }
        }
    });

    ui.add_space(8.0);

    // Interface font & text size: egui draws text without hinting, so the most comfortable
    // font depends on the monitor. Changes apply immediately and are saved.
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsFont));
        let before = app.ui_prefs;
        let fonts: Vec<(UiFont, &str)> = UiFont::ALL.iter().map(|f| (*f, f.label())).collect();
        segmented(ui, &mut app.ui_prefs.font, &fonts);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(t.get(TranslationKey::SettingsTextSize));
            ui.add(
                egui::Slider::new(&mut app.ui_prefs.text_scale, TEXT_SCALE_RANGE)
                    .step_by(0.05)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        });
        hint(ui, t.get(TranslationKey::SettingsFontHint));
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
}

/// A theme as a card: a small window drawn in its colors, then a radio dot and its name.
fn theme_card(ui: &mut Ui, theme: crate::ui::theme::ThemeKind, name: &str, selected: bool) -> egui::Response {
    let c = theme.colors();
    let size = Vec2::new(150.0, 104.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = response.hovered();
        p.rect_filled(rect, CornerRadius::same(10), THEME.bg_card);
        // Preview: the theme's background with a card holding a square and two text lines.
        let preview = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 70.0));
        p.rect_filled(
            preview,
            CornerRadius {
                nw: 10,
                ne: 10,
                sw: 0,
                se: 0,
            },
            c.bg_dark,
        );
        let window = egui::Rect::from_center_size(preview.center(), Vec2::new(96.0, 40.0));
        p.rect_filled(window, CornerRadius::same(6), c.bg_card);
        let square = egui::Rect::from_min_size(window.min + Vec2::new(8.0, 10.0), Vec2::splat(20.0));
        p.rect_filled(square, CornerRadius::same(4), c.bg_surface_hover);
        let line1 = egui::Rect::from_min_size(window.min + Vec2::new(36.0, 13.0), Vec2::new(50.0, 5.0));
        p.rect_filled(line1, CornerRadius::same(2), c.text_primary);
        let line2 = egui::Rect::from_min_size(window.min + Vec2::new(36.0, 23.0), Vec2::new(34.0, 5.0));
        p.rect_filled(line2, CornerRadius::same(2), c.accent_cyan);
        // Radio dot and name.
        let row_y = rect.bottom() - 17.0;
        let dot = egui::pos2(rect.left() + 16.0, row_y);
        p.circle_stroke(dot, 6.0, egui::Stroke::new(1.5, THEME.text_secondary));
        if selected {
            p.circle_filled(dot, 3.5, THEME.accent_cyan);
        }
        p.text(
            egui::pos2(rect.left() + 30.0, row_y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(13.0),
            if selected {
                THEME.text_primary
            } else {
                THEME.text_secondary
            },
        );
        let stroke = if selected {
            egui::Stroke::new(2.0, THEME.accent_cyan)
        } else if hovered {
            egui::Stroke::new(1.0, THEME.border_strong)
        } else {
            egui::Stroke::new(1.0, THEME.border_subtle)
        };
        p.rect_stroke(rect, CornerRadius::same(10), stroke, egui::StrokeKind::Inside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Game folder, Workshop folder and ModsConfig.xml, each with "Browse…".
fn paths_section(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsRimWorldPath));
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
                egui::RichText::new(format!(
                    "✔ {}",
                    fmt(
                        t.get(TranslationKey::SettingsGameDetected),
                        &[&version, app.steam_paths.engine.label()],
                    )
                ))
                .color(THEME.accent_emerald)
                .size(12.0),
            );
        }
    });
    ui.add_space(8.0);
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsWorkshopPath));
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
    });
    ui.add_space(8.0);
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsConfigPath));
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
        if app.mods_config_path.is_none() {
            ui.label(
                egui::RichText::new(crate::i18n::tr!(
                    "Не найден: запустите RimWorld один раз или укажите файл вручную",
                    "Not found: start RimWorld once or pick the file by hand",
                    "未找到：先启动一次 RimWorld，或手动选择文件"
                ))
                .color(THEME.accent_amber)
                .size(12.0),
            );
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
    });
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
}

/// Launch flags of the installed engine, extra arguments, a save backup before launch.
fn launch_section(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsLaunchTitle));
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
    });
    ui.add_space(8.0);
    full_card(ui, |ui| {
        let mut backup_on = app.backup_save_before_launch;
        if setting_row(
            ui,
            t.get(TranslationKey::SettingsBackupSaveBeforeLaunch),
            Some(t.get(TranslationKey::SettingsBackupSaveHint)),
            &mut backup_on,
        ) {
            app.backup_save_before_launch = backup_on;
        }
        if backup_on {
            ui.horizontal(|ui| {
                ui.label(t.get(TranslationKey::SettingsBackupSaveKeepCount));
                let mut keep = app.save_backups_to_keep;
                if ui.add(egui::DragValue::new(&mut keep).range(1..=50)).changed() {
                    app.save_backups_to_keep = keep;
                }
            });
        }
    });
    ui.add_space(8.0);
    if primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::SettingsBtnLaunchRimWorld)).clicked() {
        app.launch_game();
    }
}

/// The community load-order database: how many rules, update or import it.
fn community_db_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsDbTitle));
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
}

/// Previous versions of mods (saved before an update) and duplicates hidden with "Ignore".
fn mods_section(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsVersionsTitle));
        let mut keep = app.keep_mod_versions;
        if setting_row(
            ui,
            t.get(TranslationKey::SettingsVersionsKeep),
            Some(t.get(TranslationKey::SettingsVersionsHint)),
            &mut keep,
        ) {
            app.keep_mod_versions = keep;
        }
        let count = app.mod_versions.len();
        if count > 0 {
            let bytes: u64 = app.mod_versions.values().map(|v| v.size).sum();
            ui.horizontal_wrapped(|ui| {
                ui.label(fmt(
                    t.get(TranslationKey::SettingsVersionsCount),
                    &[&count.to_string(), &crate::workshop_browse::human_size(bytes)],
                ));
                if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::ModsBtnFolder)).clicked() {
                    open_external(crate::mod_versions::versions_dir());
                }
                if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::SettingsVersionsDelete)).clicked() {
                    app.delete_saved_versions();
                }
            });
        }
    });

    ui.add_space(8.0);
    full_card(ui, |ui| {
        group_title(
            ui,
            crate::i18n::tr!("Скрытые копии модов", "Hidden mod copies", "已隐藏的模组副本"),
        );
        let hidden = app.ignored_duplicate_count();
        if hidden > 0 {
            ui.horizontal_wrapped(|ui| {
                ui.label(fmt(
                    t.get(TranslationKey::SettingsHiddenDuplicates),
                    &[&hidden.to_string()],
                ));
                if secondary_button(ui, t.get(TranslationKey::SettingsShowDuplicates)).clicked() {
                    app.show_ignored_duplicates();
                }
            });
        } else {
            hint(
                ui,
                crate::i18n::tr!(
                    "Нет. Копию можно скрыть кнопкой «Игнорировать» в карточке дублей на вкладке «Моды».",
                    "None. A copy can be hidden with \"Ignore\" in the duplicates card on the Mods tab.",
                    "无。可在“模组”页的重复卡片中用“忽略”隐藏副本。"
                ),
            );
        }
    });
}

/// Update check and all settings in one file.
fn about_section(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    // Update check: a badge next to the version, never an automatic download.
    full_card(ui, |ui| {
        let mut on = app.check_updates;
        let hint_text = fmt(
            t.get(TranslationKey::SettingsUpdateCheckHint),
            &[crate::update_check::current_version()],
        );
        if setting_row(
            ui,
            t.get(TranslationKey::SettingsUpdateCheck),
            Some(&hint_text),
            &mut on,
        ) {
            app.check_updates = on;
            if on {
                app.start_update_check();
            } else {
                app.update_available = None;
            }
        }
    });

    // All settings in one file: to keep or to move to another computer.
    ui.add_space(8.0);
    full_card(ui, |ui| {
        group_title(ui, t.get(TranslationKey::SettingsBackupTitle));
        hint(ui, t.get(TranslationKey::SettingsBackupHint));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if secondary_button_icon(ui, ICON_SAVE, t.get(TranslationKey::SettingsBackupExport)).clicked() {
                if let Some(target) = rfd::FileDialog::new()
                    .set_file_name("RimWorldModManager-settings.zip")
                    .add_filter("zip", &["zip"])
                    .save_file()
                {
                    app.export_settings(&target);
                }
            }
            if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::SettingsBackupImport))
                .on_hover_text(t.get(TranslationKey::SettingsBackupImportHint))
                .clicked()
            {
                if let Some(source) = rfd::FileDialog::new().add_filter("zip", &["zip"]).pick_file() {
                    app.import_settings(&source);
                }
            }
        });
    });

    // Feedback: a bug or an idea opens the matching form on GitHub, the versions filled in.
    ui.add_space(8.0);
    full_card(ui, |ui| {
        group_title(ui, crate::i18n::tr!("Обратная связь", "Feedback", "反馈"));
        hint(
            ui,
            crate::i18n::tr!(
                "Откроется форма на GitHub; версии программы и игры уже вписаны. Нужен аккаунт GitHub.",
                "Opens a form on GitHub with the app and game versions filled in. Needs a GitHub account.",
                "将在 GitHub 打开表单，程序和游戏版本已填好。需要 GitHub 账号。"
            ),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            if danger_button_icon(
                ui,
                ICON_STATUS_ERROR,
                crate::i18n::tr!("Нашёл баг", "Report a bug", "报告错误"),
            )
            .clicked()
            {
                open_external(feedback_url("bug_report.yml", app.steam_paths.game_version.as_deref()));
            }
            if primary_button_icon(
                ui,
                ICON_INFO,
                crate::i18n::tr!("Есть идея", "Suggest an idea", "提个建议"),
            )
            .clicked()
            {
                open_external(feedback_url("feature_request.yml", None));
            }
        });
    });

    // The game the app works with: its version and folder; a click opens the folder.
    ui.add_space(8.0);
    let game = app.steam_paths.game_path.clone();
    let response = full_card(ui, |ui| {
        group_title(ui, crate::i18n::tr!("Игра", "Game", "游戏"));
        match &game {
            Some(dir) => {
                let version = app
                    .steam_paths
                    .game_version
                    .clone()
                    .unwrap_or_else(|| t.get(TranslationKey::SettingsNotFound).to_string());
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, THEME.accent_emerald);
                    ui.label(
                        egui::RichText::new(format!("RimWorld {version} · {}", app.steam_paths.engine.label()))
                            .color(THEME.text_primary)
                            .strong(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(ICON_FOLDER)
                            .tint(THEME.text_muted)
                            .fit_to_exact_size(Vec2::splat(14.0)),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(dir.display().to_string())
                                .color(THEME.text_muted)
                                .monospace()
                                .size(12.0),
                        )
                        .truncate(),
                    );
                });
            }
            None => {
                ui.label(egui::RichText::new(t.get(TranslationKey::SettingsNotFound)).color(THEME.accent_amber));
            }
        }
        ui.min_rect()
    });
    if let Some(dir) = game {
        let card = ui.interact(
            response.expand(12.0),
            egui::Id::unique("settings_game_card"),
            egui::Sense::click(),
        );
        if card
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(crate::i18n::tr!(
                "Открыть папку игры",
                "Open the game folder",
                "打开游戏文件夹"
            ))
            .clicked()
        {
            open_external(dir);
        }
    }
}

/// A new GitHub issue with the given form; the app version (and the game's, for a bug) filled in.
fn feedback_url(template: &str, game_version: Option<&str>) -> String {
    let encode = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    let mut url = format!(
        "https://github.com/Kansikoru/rimworld-mod-manager/issues/new?template={template}&manager-version={}",
        encode(crate::update_check::current_version())
    );
    if let Some(game) = game_version {
        url.push_str(&format!("&game-version={}", encode(game)));
    }
    url
}

/// The player's own "this mod after that one" rules, each with a button that removes it.
fn own_order_rules_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let mut remove: Option<usize> = None;
    let mut reset_all = false;
    full_card(ui, |ui| {
        ui.vertical(|ui| {
            ui.heading(
                egui::RichText::new(t.get(TranslationKey::SettingsOrderTitle))
                    .color(THEME.text_primary)
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(6.0);
            hint(ui, t.get(TranslationKey::SettingsOrderHint));
            ui.add_space(4.0);
            let rules = &app.community_db.user_order;
            if rules.is_empty() {
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::SettingsOrderEmpty))
                        .color(THEME.text_muted)
                        .size(12.0),
                );
            }
            // Today's name of an installed mod, else the one it had when the rule was made.
            let name = |id: &str, stored: &str| {
                crate::app::lookup_mod(&app.mod_index, &app.all_mods, id).map_or_else(
                    || {
                        if stored.is_empty() {
                            id.to_string()
                        } else {
                            stored.to_string()
                        }
                    },
                    |m| m.name.clone(),
                )
            };
            for (i, rule) in rules.iter().enumerate() {
                ui.horizontal(|ui| {
                    if icon_button(ui, ICON_TRASH, egui::Vec2::splat(14.0), THEME.text_muted)
                        .on_hover_text(t.get(TranslationKey::SettingsOrderDelete))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.label(fmt(
                        t.get(TranslationKey::SettingsOrderRule),
                        &[
                            &name(&rule.after, &rule.after_name),
                            &name(&rule.before, &rule.before_name),
                        ],
                    ))
                    .on_hover_text(format!("{} → {}", rule.before, rule.after));
                });
            }
            // Reset of every rule at the bottom of the card, asked once more before it goes.
            if !rules.is_empty() {
                ui.add_space(8.0);
                let confirm_id = egui::Id::unique("reset_own_order_rules");
                let asking = ui.data(|d| d.get_temp::<bool>(confirm_id).unwrap_or(false));
                if asking {
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::SettingsOrderResetConfirm))
                            .color(THEME.accent_rose)
                            .size(12.0),
                    );
                    ui.horizontal(|ui| {
                        if danger_button_icon(ui, ICON_TRASH, t.get(TranslationKey::SettingsOrderResetYes)).clicked() {
                            reset_all = true;
                            ui.data_mut(|d| d.remove::<bool>(confirm_id));
                        }
                        if secondary_button(ui, t.get(TranslationKey::BtnCancel)).clicked() {
                            ui.data_mut(|d| d.remove::<bool>(confirm_id));
                        }
                    });
                } else if danger_button_icon(
                    ui,
                    ICON_TRASH,
                    &fmt(
                        t.get(TranslationKey::SettingsOrderResetAll),
                        &[&rules.len().to_string()],
                    ),
                )
                .clicked()
                {
                    ui.data_mut(|d| d.insert_temp(confirm_id, true));
                }
            }
        });
    });
    if reset_all {
        app.reset_order_rules();
    } else if let Some(i) = remove {
        app.remove_order_rule(i);
    }
}
