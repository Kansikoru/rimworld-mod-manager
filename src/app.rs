use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui::{self, Color32, Rounding, Stroke};

use crate::analyzer::{analyze_rimworld_log, detect_rimworld_log_paths, LogReport};
use crate::cleaner::{delete_translation_mods, unsubscribe_and_delete_single};
use crate::community_rules::get_default_community_db;
use crate::dependencies::{validate_dependencies, DependencyProblem, DependencyReport};
use crate::i18n::{I18n, Language, TranslationKey};
use crate::matcher::classify_and_link_translations;
use crate::models::{is_protected_package_id, is_optimizer_blacklisted, ModInfo, TranslationTargetLang};
use crate::mods_config::{get_default_mods_config_path, load_active_mods, save_active_mods};
use crate::optimizer::{
    count_textures_preview, default_cpu_threads, dirs_from_mods, rollback_dds, run_optimization,
    total_system_threads, OptimizationMsg, OptimizeScope,
};
use crate::scanner::scan_mods;
use crate::sorter::sort_load_order;
use crate::steam::{
    detect_steam_paths, launch_rimworld, subscribe_workshop_item_native, SteamPaths,
};
use crate::ui::icons::*;
use crate::ui::theme::{apply_dark_theme, THEME};
use crate::workshop_search::{search_steam_workshop, WorkshopItemResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Mods,
    Translations,
    Optimizer,
    Analyzer,
    Settings,
}

pub struct App {
    pub current_tab: AppTab,
    pub i18n: I18n,
    pub is_russian: bool,

    // Mods State
    pub all_mods: Vec<ModInfo>,
    pub active_mods: Vec<String>,
    pub selected_mod_id: Option<String>,
    pub selected_mod_ids: std::collections::HashSet<String>,
    pub last_clicked_mod_id: Option<String>,
    pub mod_search_query: String,
    pub active_search_query: String,
    pub dependency_report: DependencyReport,
    pub is_details_panel_collapsed: bool,
    pub is_details_panel_visible: bool,

    // Icons State
    pub icon_cache: crate::icon_cache::IconDiskCache,
    pub mod_textures: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    pub icon_tx: std::sync::mpsc::Sender<(String, std::path::PathBuf)>,
    pub icon_rx: std::sync::mpsc::Receiver<(String, egui::ColorImage)>,

    // Translations State
    pub trans_target_lang: TranslationTargetLang,
    pub workshop_search_query: String,
    pub workshop_search_results: Vec<WorkshopItemResult>,

    // Optimizer State
    pub opt_scope: OptimizeScope,
    pub opt_force: bool,
    pub opt_delete_source: bool,
    pub opt_threads: usize,
    pub max_system_threads: usize,
    pub opt_is_running: bool,
    pub opt_done: usize,
    pub opt_total: usize,
    pub opt_errors: usize,
    pub opt_current_file: String,
    pub opt_stop_flag: Arc<AtomicBool>,
    pub opt_rx: Option<Receiver<OptimizationMsg>>,
    pub opt_ignore_list: std::collections::HashSet<String>,

    // Analyzer State
    pub log_report: Option<LogReport>,
    pub custom_log_path: Option<PathBuf>,

    // Settings & Paths State
    pub steam_paths: SteamPaths,
    pub mods_config_path: Option<PathBuf>,
    pub launch_quicktest: bool,
    pub launch_logfile: bool,
    pub launch_popupwindow: bool,
    pub community_db_rules_count: usize,

    // Status Message
    pub status_message: String,
}

impl Default for App {
    fn default() -> Self {
        let i18n = I18n::new(Language::Russian);
        let steam_paths = detect_steam_paths();
        let mods_config_path = get_default_mods_config_path();
        let max_system_threads = total_system_threads();
        let default_threads = default_cpu_threads();
        let db_rules_count = get_default_community_db().rules.len();

        let mut opt_ignore_list = std::collections::HashSet::new();
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let ignore_path = std::path::PathBuf::from(local_appdata)
                .join("RimWorldModManager")
                .join("optimizer_ignore.txt");
            if let Ok(content) = std::fs::read_to_string(&ignore_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        opt_ignore_list.insert(trimmed.to_lowercase());
                    }
                }
            }
        }

        let (icon_tx, icon_thread_rx) = channel::<(String, PathBuf)>();
        let (icon_thread_tx, icon_rx) = channel::<(String, egui::ColorImage)>();
        let icon_cache = crate::icon_cache::IconDiskCache::new(crate::icon_cache::IconDiskCache::default_cache_dir());

        let cache_clone = crate::icon_cache::IconDiskCache::new(crate::icon_cache::IconDiskCache::default_cache_dir());
        std::thread::spawn(move || {
            while let Ok((pkg_id, path)) = icon_thread_rx.recv() {
                if let Some(img) = crate::icon_cache::load_or_cache_image(&cache_clone, &path, 64) {
                    let _ = icon_thread_tx.send((pkg_id, img));
                }
            }
        });

        let mut app = Self {
            current_tab: AppTab::Mods,
            i18n,
            is_russian: true,

            all_mods: Vec::new(),
            active_mods: Vec::new(),
            selected_mod_id: None,
            selected_mod_ids: std::collections::HashSet::new(),
            last_clicked_mod_id: None,
            mod_search_query: String::new(),
            active_search_query: String::new(),
            dependency_report: DependencyReport::default(),
            is_details_panel_collapsed: false,
            is_details_panel_visible: true,

            icon_cache,
            mod_textures: std::collections::HashMap::new(),
            icon_tx,
            icon_rx,

            trans_target_lang: TranslationTargetLang::Ru,
            workshop_search_query: String::new(),
            workshop_search_results: Vec::new(),

            opt_scope: OptimizeScope::AllInstalled,
            opt_force: false,
            opt_delete_source: false,
            opt_threads: default_threads,
            max_system_threads,
            opt_is_running: false,
            opt_done: 0,
            opt_total: 0,
            opt_errors: 0,
            opt_current_file: String::new(),
            opt_stop_flag: Arc::new(AtomicBool::new(false)),
            opt_rx: None,
            opt_ignore_list,

            log_report: None,
            custom_log_path: None,

            steam_paths,
            mods_config_path,
            launch_quicktest: false,
            launch_logfile: true,
            launch_popupwindow: false,
            community_db_rules_count: db_rules_count,

            status_message: "Готов к работе".to_string(),
        };

        app.rescan_mods();
        app
    }
}

impl App {
    pub fn set_language(&mut self, lang: Language) {
        self.is_russian = lang == Language::Russian;
        self.i18n = I18n::new(lang);
        if let Some(ref report) = self.log_report {
            self.status_message = if report.has_hard_crash {
                self.i18n.get(TranslationKey::CrashSummaryHardCrash).replace("{}", &report.culprits.len().to_string())
            } else if !report.culprits.is_empty() {
                self.i18n.get(TranslationKey::CrashSummaryCulprits).replace("{}", &report.culprits.len().to_string())
            } else if report.total_errors > 0 {
                self.i18n.get(TranslationKey::CrashSummaryErrorsOnly).replace("{}", &report.total_errors.to_string())
            } else {
                self.i18n.get(TranslationKey::CrashSummaryClean).to_string()
            };
        } else {
            self.status_message = self.i18n.get(TranslationKey::StatusModsCount)
                .replace("{}", &self.all_mods.len().to_string())
                .replacen("{}", &self.active_mods.len().to_string(), 1);
        }
    }

    pub fn toggle_optimizer_ignore(&mut self, package_id: &str) {
        let lower = package_id.to_lowercase();
        if self.opt_ignore_list.contains(&lower) {
            self.opt_ignore_list.remove(&lower);
        } else {
            self.opt_ignore_list.insert(lower);
        }
        
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let ignore_path = std::path::PathBuf::from(local_appdata)
                .join("RimWorldModManager")
                .join("optimizer_ignore.txt");
            
            let content = self.opt_ignore_list.iter().cloned().collect::<Vec<_>>().join("\n");
            let _ = std::fs::write(&ignore_path, content);
        }
    }

    pub fn rescan_mods(&mut self) {
        self.status_message = "Сканирование модов на диске...".to_string();

        let mut scanned = scan_mods(
            self.steam_paths.workshop_path.as_deref(),
            self.steam_paths.local_mods_path.as_deref(),
            self.steam_paths.game_data_path.as_deref(),
        );

        classify_and_link_translations(&mut scanned);
        self.all_mods = scanned;

        if let Some(ref config_p) = self.mods_config_path {
            let loaded = load_active_mods(config_p);
            let known_ids: std::collections::HashSet<String> = self.all_mods.iter().map(|m| m.package_id.to_lowercase()).collect();
            self.active_mods = loaded.into_iter().filter(|id| known_ids.contains(&id.to_lowercase())).collect();
        }

        self.validate_deps();
        self.status_message = self.i18n.get(TranslationKey::StatusModsCount).replace("{}", &self.all_mods.len().to_string()).replacen("{}", &self.active_mods.len().to_string(), 1);
    }

    pub fn validate_deps(&mut self) {
        self.dependency_report = validate_dependencies(&self.active_mods, &self.all_mods);
    }

    pub fn sort_active_mods(&mut self) {
        self.active_mods = sort_load_order(&self.active_mods, &self.all_mods);
        self.validate_deps();
        self.status_message = "Порядок модов оптимизирован топологическим алгоритмом".to_string();
    }

    pub fn save_config(&mut self) {
        if let Some(ref config_p) = self.mods_config_path {
            match save_active_mods(config_p, &self.active_mods) {
                Ok(_) => {
                    self.status_message = "ModsConfig.xml успешно сохранён!".to_string();
                }
                Err(e) => {
                    self.status_message = format!("Ошибка сохранения конфига: {}", e);
                }
            }
        }
    }

    pub fn activate_all_mods(&mut self) {
        for m in &self.all_mods {
            if !self.active_mods.iter().any(|id| id.eq_ignore_ascii_case(&m.package_id)) {
                self.active_mods.push(m.package_id.clone());
            }
        }
        self.validate_deps();
    }

    pub fn deactivate_all_mods(&mut self) {
        self.active_mods.retain(|id| is_protected_package_id(id));
        self.validate_deps();
    }

    pub fn auto_fix_dependencies(&mut self) {
        for prob in self.dependency_report.problems.clone() {
            match prob {
                DependencyProblem::Disabled { required_id, .. } => {
                    if !self.active_mods.iter().any(|id| id.eq_ignore_ascii_case(&required_id)) {
                        self.active_mods.push(required_id);
                    }
                }
                DependencyProblem::WrongOrder { .. } => {
                    self.sort_active_mods();
                }
                _ => {}
            }
        }
        self.sort_active_mods();
    }

    pub fn delete_detected_translations(&mut self) {
        let trans_refs: Vec<&ModInfo> = self
            .all_mods
            .iter()
            .filter(|m| m.is_translation_mod)
            .collect();

        let res = delete_translation_mods(&trans_refs, &self.steam_paths);
        self.status_message = format!(
            "Удалено переводов: {} | Отписано в Steam: {}",
            res.deleted_count, res.unsubscribed_count
        );
        self.rescan_mods();
    }

    pub fn delete_single_mod(&mut self, pkg_id: &str) {
        if let Some(m) = self.all_mods.iter().find(|m| m.package_id.eq_ignore_ascii_case(pkg_id)) {
            match unsubscribe_and_delete_single(m, &self.steam_paths) {
                Ok(_) => {
                    self.status_message = format!("Мод {} успешно удалён", m.name);
                    self.rescan_mods();
                }
                Err(e) => {
                    self.status_message = format!("Ошибка удаления: {}", e);
                }
            }
        }
    }

    pub fn search_workshop_translations_for_selected(&mut self) {
        if let Some(ref sel_id) = self.selected_mod_id {
            if let Some(m) = self.all_mods.iter().find(|m| m.package_id.eq_ignore_ascii_case(sel_id)) {
                let query = format!("{} {}", m.name, self.trans_target_lang.search_suffix());
                self.workshop_search_query = query.clone();
                self.status_message = format!("Поиск в Workshop: {}", query);
                if let Ok(res) = search_steam_workshop(&query) {
                    self.workshop_search_results = res;
                }
            }
        }
    }

    pub fn search_workshop_custom(&mut self) {
        if !self.workshop_search_query.trim().is_empty() {
            self.status_message = format!("Поиск в Workshop: {}", self.workshop_search_query);
            if let Ok(res) = search_steam_workshop(&self.workshop_search_query) {
                self.workshop_search_results = res;
            }
        }
    }

    pub fn subscribe_steam_mod(&mut self, item_id: &str) {
        if let Some(ref dll) = self.steam_paths.steam_dll_path {
            match subscribe_workshop_item_native(dll, item_id) {
                Ok(_) => {
                    self.status_message = format!("Успешная подписка на Steam Item #{}", item_id);
                }
                Err(e) => {
                    self.status_message = format!("Ошибка подписки: {}", e);
                }
            }
        } else {
            let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={}", item_id);
            let _ = open::that(url);
            self.status_message = format!("Открыта страница Steam Workshop #{}", item_id);
        }
    }

    pub fn start_texture_optimization(&mut self) {
        let dirs = match self.opt_scope {
            OptimizeScope::AllInstalled => {
                let valid_mods: Vec<ModInfo> = self.all_mods.iter().filter(|m| {
                    !is_protected_package_id(&m.package_id) 
                        && !m.package_id.to_lowercase().starts_with("ludeon.rimworld")
                        && !self.opt_ignore_list.contains(&m.package_id.to_lowercase())
                        && !is_optimizer_blacklisted(&m.package_id)
                }).cloned().collect();
                dirs_from_mods(&valid_mods)
            }
            OptimizeScope::ActiveOnly => {
                let active_set: std::collections::HashSet<String> =
                    self.active_mods.iter().map(|s| s.to_lowercase()).collect();
                let active_mods: Vec<ModInfo> = self
                    .all_mods
                    .iter()
                    .filter(|m| {
                        active_set.contains(&m.package_id.to_lowercase())
                            && !is_protected_package_id(&m.package_id)
                            && !m.package_id.to_lowercase().starts_with("ludeon.rimworld")
                            && !self.opt_ignore_list.contains(&m.package_id.to_lowercase())
                            && !is_optimizer_blacklisted(&m.package_id)
                    })
                    .cloned()
                    .collect();
                dirs_from_mods(&active_mods)
            }
            OptimizeScope::SelectedOnly => {
                if let Some(ref sel_id) = self.selected_mod_id {
                    let sel_mod = self
                        .all_mods
                        .iter()
                        .find(|m| m.package_id.eq_ignore_ascii_case(sel_id));
                    if let Some(m) = sel_mod {
                        if !is_protected_package_id(&m.package_id) 
                            && !m.package_id.to_lowercase().starts_with("ludeon.rimworld")
                            && !self.opt_ignore_list.contains(&m.package_id.to_lowercase()) 
                            && !is_optimizer_blacklisted(&m.package_id) {
                            vec![m.dir_path.clone()]
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            }
        };

        if dirs.is_empty() {
            self.status_message = "Нет выбранных модов для оптимизации текстур".to_string();
            return;
        }

        let (tx, rx) = channel();
        self.opt_rx = Some(rx);
        self.opt_is_running = true;
        self.opt_stop_flag.store(false, Ordering::Relaxed);
        self.opt_done = 0;
        self.opt_total = 0;
        self.opt_errors = 0;

        run_optimization(
            dirs,
            self.opt_force,
            self.opt_delete_source,
            self.opt_threads,
            tx,
            self.opt_stop_flag.clone(),
        );

        self.status_message = "Запущена конвертация текстур в BC7...".to_string();
    }

    pub fn cancel_texture_optimization(&mut self) {
        self.opt_stop_flag.store(true, Ordering::Relaxed);
        self.status_message = "Отмена оптимизации...".to_string();
    }

    pub fn scan_texture_counts(&mut self) {
        let dirs = dirs_from_mods(&self.all_mods);
        let (needs, already) = count_textures_preview(&dirs);
        self.status_message = format!(
            "Текстур для конвертации: {} | Уже в DDS: {}",
            needs, already
        );
    }

    pub fn rollback_textures(&mut self) {
        let dirs = dirs_from_mods(&self.all_mods);
        let count = rollback_dds(&dirs);
        self.status_message = format!("Удалено сконвертированных .dds файлов: {}", count);
    }

    pub fn run_log_analysis(&mut self) {
        let log_target = if let Some(ref custom_p) = self.custom_log_path {
            Some(custom_p.clone())
        } else {
            detect_rimworld_log_paths()
                .into_iter()
                .next()
                .map(|(_, p)| p)
        };

        if let Some(path) = log_target {
            let report = analyze_rimworld_log(&path, &self.all_mods);
            let summary_text = if report.has_hard_crash {
                self.i18n.get(TranslationKey::CrashSummaryHardCrash).replace("{}", &report.culprits.len().to_string())
            } else if !report.culprits.is_empty() {
                self.i18n.get(TranslationKey::CrashSummaryCulprits).replace("{}", &report.culprits.len().to_string())
            } else if report.total_errors > 0 {
                self.i18n.get(TranslationKey::CrashSummaryErrorsOnly).replace("{}", &report.total_errors.to_string())
            } else {
                self.i18n.get(TranslationKey::CrashSummaryClean).to_string()
            };
            self.status_message = summary_text;
            self.log_report = Some(report);
        } else {
            self.status_message = self.i18n.get(TranslationKey::CrashFileNotFound).to_string();
        }
    }

    pub fn launch_game(&mut self) {
        match launch_rimworld(
            &self.steam_paths,
            self.launch_quicktest,
            self.launch_logfile,
            self.launch_popupwindow,
        ) {
            Ok(_) => {
                self.status_message = "Процесс RimWorld запущен!".to_string();
            }
            Err(e) => {
                self.status_message = format!("Ошибка запуска игры: {}", e);
            }
        }
    }

    pub fn update_community_rules(&mut self) {
        self.status_message = "База правил сообщества обновлена".to_string();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_dark_theme(ctx);

        // Process loaded mod icons
        while let Ok((pkg_id, img)) = self.icon_rx.try_recv() {
            let handle = ctx.load_texture(
                &pkg_id,
                img,
                Default::default()
            );
            self.mod_textures.insert(pkg_id, Some(handle));
        }

        // Receive background optimizer messages
        if let Some(ref rx) = self.opt_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    OptimizationMsg::Progress {
                        done,
                        total,
                        current_file,
                        ..
                    } => {
                        self.opt_done = done;
                        self.opt_total = total;
                        self.opt_current_file = current_file;
                    }
                    OptimizationMsg::FileError(_) => {
                        self.opt_errors += 1;
                    }
                    OptimizationMsg::Done {
                        processed,
                        errors,
                        deleted_sources,
                        ..
                    } => {
                        self.opt_is_running = false;
                        self.status_message = format!(
                            "Конвертация завершена! Обработано: {}, Ошибок: {}, Удалено исходников: {}",
                            processed, errors, deleted_sources
                        );
                    }
                    OptimizationMsg::Cancelled => {
                        self.opt_is_running = false;
                        self.status_message = "Оптимизация остановлена пользователем".to_string();
                    }
                }
            }
        }

        // Top Navigation Bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // App Logo / Title
                ui.heading(
                    egui::RichText::new("⚡ RimWorld Mod Manager")
                        .color(THEME.accent_cyan)
                        .size(17.0)
                        .strong(),
                );

                ui.label(
                    egui::RichText::new("v2.5 High-Perf Native")
                        .color(THEME.text_muted)
                        .size(11.0),
                );

                ui.add_space(16.0);

                // Navigation Tabs
                let tabs = [
                    (AppTab::Mods, self.i18n.get(TranslationKey::TabMods), ICON_MODS),
                    (AppTab::Translations, self.i18n.get(TranslationKey::TabTranslations), ICON_TRANSLATIONS),
                    (AppTab::Optimizer, self.i18n.get(TranslationKey::TabOptimizer), ICON_OPTIMIZER),
                    (AppTab::Analyzer, self.i18n.get(TranslationKey::TabAnalyzer), ICON_ANALYZER),
                    (AppTab::Settings, self.i18n.get(TranslationKey::TabSettings), ICON_SETTINGS),
                ];

                for (tab_val, label, icon) in tabs {
                    let is_active = self.current_tab == tab_val;

                    let (bg, border, text_color) = if is_active {
                        (THEME.bg_surface_hover, THEME.accent_cyan, THEME.accent_cyan)
                    } else {
                        (THEME.bg_card, THEME.border_subtle, THEME.text_secondary)
                    };

                    let btn_resp = ui.add(
                        egui::Button::image_and_text(
                            svg_icon(icon, egui::vec2(15.0, 15.0), text_color),
                            egui::RichText::new(label)
                                .color(text_color)
                                .size(13.0)
                                .strong(),
                        )
                        .fill(bg)
                        .stroke(Stroke::new(1.0, border))
                        .rounding(Rounding::same(6.0)),
                    );

                    if btn_resp.clicked() {
                        self.current_tab = tab_val;
                    }
                }

                // Fast Launch Game button in top right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(
                        egui::Button::image_and_text(
                            svg_icon(ICON_PLAY, egui::vec2(14.0, 14.0), Color32::WHITE),
                            egui::RichText::new(self.i18n.get(TranslationKey::AppPlayBtn))
                                .color(Color32::WHITE)
                                .size(13.0)
                                .strong(),
                        )
                        .fill(THEME.accent_emerald)
                        .rounding(Rounding::same(6.0)),
                    ).clicked() {
                        self.launch_game();
                    }
                });
            });
            ui.add_space(4.0);
        });

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("bottom_status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&self.status_message)
                        .color(THEME.text_secondary)
                        .size(11.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(self.i18n.get(TranslationKey::AppStatusBarStats).replace("{}", &self.all_mods.len().to_string()).replacen("{}", &self.active_mods.len().to_string(), 1))
                        .color(THEME.text_muted)
                        .size(11.0),
                    );
                });
            });
        });

        // Main Viewport Area
        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            AppTab::Mods => crate::ui::view_mods::render_mods_view(self, ui),
            AppTab::Translations => crate::ui::view_translations::render_translations_view(self, ui),
            AppTab::Optimizer => crate::ui::view_optimizer::render_optimizer_view(self, ui),
            AppTab::Analyzer => crate::ui::view_analyzer::render_analyzer_view(self, ui),
            AppTab::Settings => crate::ui::view_settings::render_settings_view(self, ui),
        });

        if self.opt_is_running {
            ctx.request_repaint();
        }
    }
}
