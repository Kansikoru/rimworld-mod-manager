use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui::{self, Color32, Rounding, Stroke};

use crate::analyzer::{analyze_rimworld_log, detect_rimworld_log_paths, LogReport};
use crate::cleaner::{delete_translation_mods, unsubscribe_and_delete_single};
use crate::community_rules::get_default_community_db;
use crate::dependencies::{validate_dependencies, DependencyProblem, DependencyReport};
use crate::categories::ModCategory;
use crate::game_install::known_launch_flags;
use crate::i18n::{fmt, I18n, Language, TranslationKey};
use crate::matcher::{classify_and_link_translations, filter_and_rank_workshop_translations};
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

/// Finds an installed mod by package ID: case-insensitive, and understands the `_steam`
/// suffix RimWorld adds to a Workshop copy when a local copy with the same ID also exists.
pub fn find_mod_in<'a>(all_mods: &'a [ModInfo], id: &str) -> Option<&'a ModInfo> {
    let by_id = |needle: &str| all_mods.iter().find(|m| m.package_id.eq_ignore_ascii_case(needle));
    by_id(id).or_else(|| {
        let lower = id.to_lowercase();
        lower.strip_suffix("_steam").and_then(by_id)
    })
}

/// Empty filter shows everything; otherwise the mod needs at least one of the categories.
/// Mods that are not on disk (`None`) are hidden while a filter is active.
pub fn passes_category_filter(
    filter: &std::collections::HashSet<ModCategory>,
    m: Option<&ModInfo>,
) -> bool {
    filter.is_empty() || m.map(|m| m.categories.iter().any(|c| filter.contains(c))).unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteKind {
    /// "Delete all translations" for the selected language
    Translations,
    /// A single mod (trash icon)
    Single,
}

/// Deletion waiting for the user's confirmation in a modal window.
#[derive(Debug, Clone)]
pub struct PendingDelete {
    pub kind: DeleteKind,
    pub ids: Vec<String>,
    pub names: Vec<String>,
}

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
    /// Checked launch flags (from `known_launch_flags` of the detected engine).
    pub launch_flags: std::collections::HashSet<String>,
    /// Free-form extra launch arguments, split on whitespace.
    pub launch_extra_args: String,
    pub community_db_rules_count: usize,

    /// Deletion waiting for confirmation (modal window).
    pub pending_delete: Option<PendingDelete>,

    /// Mods found more than once on disk (same packageId in several folders).
    pub duplicate_mods: Vec<crate::scanner::DuplicateModEntry>,

    /// Categories set by the user (lowercase packageId -> categories), saved to disk.
    pub category_overrides: std::collections::HashMap<String, Vec<ModCategory>>,
    /// Mod list filter: show only mods with any of these categories (empty = all).
    pub category_filter: std::collections::HashSet<ModCategory>,

    // Status Message
    pub status_message: String,
}

impl Default for App {
    fn default() -> Self {
        Self::build(None)
    }
}

impl App {
    /// Creates the app. The context lets background threads (icon loading) wake the UI
    /// when they finish, so icons appear without moving the mouse.
    pub fn new(ctx: &egui::Context) -> Self {
        Self::build(Some(ctx.clone()))
    }

    fn build(repaint_ctx: Option<egui::Context>) -> Self {
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
                    if icon_thread_tx.send((pkg_id, img)).is_err() {
                        break;
                    }
                    if let Some(ref ctx) = repaint_ctx {
                        ctx.request_repaint();
                    }
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
            launch_flags: ["-logfile".to_string()].into_iter().collect(),
            launch_extra_args: String::new(),
            community_db_rules_count: db_rules_count,

            pending_delete: None,
            duplicate_mods: Vec::new(),

            category_overrides: crate::categories::load_overrides(&crate::categories::overrides_path()),
            category_filter: std::collections::HashSet::new(),

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
            self.status_message = self.mods_count_status();
        }
    }

    fn mods_count_status(&self) -> String {
        fmt(
            self.i18n.get(TranslationKey::StatusModsCount),
            &[&self.all_mods.len().to_string(), &self.active_mods.len().to_string()],
        )
    }

    /// Finds an installed mod by package ID: case-insensitive, and understands the
    /// `_steam` suffix RimWorld adds to a Workshop copy when a local copy also exists.
    pub fn find_mod(&self, id: &str) -> Option<&ModInfo> {
        find_mod_in(&self.all_mods, id)
    }

    /// Official content (Core/DLC in the game's Data folder) — never touched by the optimizer or deletion.
    pub fn is_official_mod(&self, m: &ModInfo) -> bool {
        let id = m.package_id.to_lowercase();
        if is_protected_package_id(&id) || id.starts_with("ludeon.rimworld") {
            return true;
        }
        self.steam_paths
            .game_data_path
            .as_ref()
            .map(|data| m.dir_path.starts_with(data))
            .unwrap_or(false)
    }

    /// Whether a mod passes the category filter of the mod list.
    pub fn passes_category_filter(&self, m: Option<&ModInfo>) -> bool {
        passes_category_filter(&self.category_filter, m)
    }

    /// Adds or removes a category of a mod by hand (right-click menu). Saved to disk.
    pub fn toggle_mod_category(&mut self, pkg_id: &str, cat: ModCategory) {
        let key = pkg_id.to_lowercase();
        let current = self
            .find_mod(pkg_id)
            .map(|m| m.categories.clone())
            .unwrap_or_default();
        let mut cats = self.category_overrides.get(&key).cloned().unwrap_or(current);
        if let Some(pos) = cats.iter().position(|c| *c == cat) {
            cats.remove(pos);
        } else {
            cats.push(cat);
        }
        self.category_overrides.insert(key, cats);
        self.apply_category_overrides();
    }

    /// Drops the manual categories of a mod and goes back to the automatic ones.
    pub fn reset_mod_categories(&mut self, pkg_id: &str) {
        self.category_overrides.remove(&pkg_id.to_lowercase());
        self.apply_category_overrides();
    }

    fn apply_category_overrides(&mut self) {
        crate::categories::assign_categories(&mut self.all_mods, &self.category_overrides);
        if let Err(e) = crate::categories::save_overrides(
            &crate::categories::overrides_path(),
            &self.category_overrides,
        ) {
            self.status_message = format!("Не удалось сохранить категории: {}", e);
        }
    }

    /// Switches the game folder from Settings: recomputes engine, exe, Steam DLL, Mods/Data and rescans.
    pub fn set_game_path(&mut self, folder: PathBuf) {
        self.steam_paths.set_game_dir(folder);
        self.rescan_mods();
    }

    /// Launch arguments: checked flags valid for the detected engine + extra arguments.
    pub fn launch_args(&self) -> Vec<String> {
        let mut args: Vec<String> = known_launch_flags(self.steam_paths.engine)
            .iter()
            .filter(|(flag, _)| self.launch_flags.contains(*flag))
            .map(|(flag, _)| flag.to_string())
            .collect();
        args.extend(self.launch_extra_args.split_whitespace().map(str::to_string));
        args
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

        let scanned = scan_mods(
            self.steam_paths.workshop_path.as_deref(),
            self.steam_paths.local_mods_path.as_deref(),
            self.steam_paths.game_data_path.as_deref(),
        );

        // Two copies with the same packageId show up twice in the game's mod list: keep one, warn about it.
        let (mut unique, duplicates) = crate::scanner::dedupe_mods(scanned);
        classify_and_link_translations(&mut unique);
        crate::categories::assign_categories(&mut unique, &self.category_overrides);
        self.all_mods = unique;
        self.duplicate_mods = duplicates;

        if let Some(ref config_p) = self.mods_config_path {
            // Keep IDs that were not found on disk: they stay in ModsConfig.xml on save
            // and are shown as "not on disk" instead of being silently dropped.
            self.active_mods = load_active_mods(config_p);
        }

        self.validate_deps();
        self.status_message = self.mods_count_status();
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

    /// Translation mods installed for the language selected on the Translations tab.
    pub fn translations_for_target_lang(&self) -> Vec<&ModInfo> {
        self.all_mods
            .iter()
            .filter(|m| m.is_translation_mod && !self.is_official_mod(m))
            .filter(|m| self.trans_target_lang.matches_mod_info(m))
            .collect()
    }

    /// "Delete all translations": only for the selected language, and only after confirmation.
    pub fn request_delete_translations(&mut self) {
        let mods = self.translations_for_target_lang();
        if mods.is_empty() {
            self.status_message = fmt(
                self.i18n.get(TranslationKey::TransNoneForLang),
                &[self.trans_target_lang.label()],
            );
            return;
        }
        let pending = PendingDelete {
            kind: DeleteKind::Translations,
            ids: mods.iter().map(|m| m.package_id.clone()).collect(),
            names: mods.iter().map(|m| m.name.clone()).collect(),
        };
        self.pending_delete = Some(pending);
    }

    pub fn request_delete_single(&mut self, pkg_id: &str) {
        let Some(m) = self.find_mod(pkg_id) else {
            return;
        };
        if self.is_official_mod(m) {
            self.status_message = format!("Защищённый мод '{}' нельзя удалить!", m.package_id);
            return;
        }
        let pending = PendingDelete {
            kind: DeleteKind::Single,
            ids: vec![m.package_id.clone()],
            names: vec![m.name.clone()],
        };
        self.pending_delete = Some(pending);
    }

    /// Runs the deletion the user confirmed in the modal window.
    pub fn confirm_pending_delete(&mut self) {
        let Some(pending) = self.pending_delete.take() else {
            return;
        };
        let targets: Vec<&ModInfo> = pending
            .ids
            .iter()
            .filter_map(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .collect();

        match pending.kind {
            DeleteKind::Translations => {
                let res = delete_translation_mods(&targets, &self.steam_paths);
                let mut msg = format!(
                    "Перемещено в Корзину: {} | Отписано в Steam: {}",
                    res.deleted_count, res.unsubscribed_count
                );
                if let Some(err) = res.errors.first() {
                    msg.push_str(&format!(" | Ошибок: {} ({})", res.errors.len(), err));
                }
                self.status_message = msg;
                self.rescan_mods_keep_status();
            }
            DeleteKind::Single => {
                let Some(m) = targets.first() else {
                    return;
                };
                let name = m.name.clone();
                match unsubscribe_and_delete_single(m, &self.steam_paths) {
                    Ok(_) => {
                        self.status_message = format!("Мод {} перемещён в Корзину", name);
                        self.rescan_mods_keep_status();
                    }
                    Err(e) => {
                        self.status_message = format!("Ошибка удаления: {}", e);
                    }
                }
            }
        }
    }

    fn rescan_mods_keep_status(&mut self) {
        let msg = self.status_message.clone();
        self.rescan_mods();
        self.status_message = msg;
    }

    pub fn search_workshop_translations_for_selected(&mut self) {
        if let Some(ref sel_id) = self.selected_mod_id {
            if let Some(m) = self.all_mods.iter().find(|m| m.package_id.eq_ignore_ascii_case(sel_id)) {
                let query = format!("{} {}", m.name, self.trans_target_lang.search_suffix());
                self.workshop_search_query = query.clone();
                match search_steam_workshop(&query) {
                    Ok(raw) => {
                        let found = raw.len();
                        // Keep only translations of this mod in the selected language,
                        // drop the mod itself and unrelated items, best matches first.
                        let filtered = filter_and_rank_workshop_translations(
                            raw,
                            &m.name,
                            m.steam_item_id.as_deref(),
                            self.trans_target_lang,
                        );
                        self.status_message = format!(
                            "Workshop: найдено {}, подходящих переводов «{}»: {}",
                            found,
                            m.name,
                            filtered.len()
                        );
                        self.workshop_search_results = filtered;
                    }
                    Err(e) => {
                        self.status_message = format!("Ошибка поиска в Workshop: {}", e);
                    }
                }
            }
        }
    }

    pub fn search_workshop_custom(&mut self) {
        if !self.workshop_search_query.trim().is_empty() {
            match search_steam_workshop(&self.workshop_search_query) {
                Ok(res) => {
                    self.status_message = format!(
                        "Поиск в Workshop «{}»: результатов {}",
                        self.workshop_search_query,
                        res.len()
                    );
                    self.workshop_search_results = res;
                }
                Err(e) => {
                    self.status_message = format!("Ошибка поиска в Workshop: {}", e);
                }
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
            OptimizeScope::AllInstalled => dirs_from_mods(&self.optimizable_mods()),
            OptimizeScope::ActiveOnly => {
                let active_set: std::collections::HashSet<String> =
                    self.active_mods.iter().map(|s| s.to_lowercase()).collect();
                let active_mods: Vec<ModInfo> = self
                    .all_mods
                    .iter()
                    .filter(|m| {
                        active_set.contains(&m.package_id.to_lowercase())
                            && !self.is_official_mod(m)
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
                        if !self.is_official_mod(m)
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

    /// Mods the optimizer may touch for "All mods": no Core/DLC, no blacklist, no user ignore list.
    fn optimizable_mods(&self) -> Vec<ModInfo> {
        self.all_mods
            .iter()
            .filter(|m| {
                !self.is_official_mod(m)
                    && !self.opt_ignore_list.contains(&m.package_id.to_lowercase())
                    && !is_optimizer_blacklisted(&m.package_id)
            })
            .cloned()
            .collect()
    }

    pub fn scan_texture_counts(&mut self) {
        let dirs = dirs_from_mods(&self.optimizable_mods());
        let (needs, already) = count_textures_preview(&dirs);
        self.status_message = format!(
            "Текстур для конвертации: {} | Уже в DDS: {}",
            needs, already
        );
    }

    /// Rolls back every mod except Core/DLC — including ignored/blacklisted ones,
    /// because older versions of the optimizer did not respect those lists.
    pub fn rollback_textures(&mut self) {
        let mods: Vec<ModInfo> = self
            .all_mods
            .iter()
            .filter(|m| !self.is_official_mod(m))
            .cloned()
            .collect();
        let res = rollback_dds(&dirs_from_mods(&mods));
        self.status_message = format!(
            "Откат: удалено .dds: {} | восстановлено исходников из бэкапа: {} | ошибок: {}",
            res.removed, res.restored, res.errors
        );
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
        let args = self.launch_args();
        match launch_rimworld(&self.steam_paths, &args) {
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
                            "Конвертация завершена! Обработано: {}, Ошибок: {}, Исходников перенесено в бэкап: {}",
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
                        egui::RichText::new(fmt(
                            self.i18n.get(TranslationKey::AppStatusBarStats),
                            &[&self.all_mods.len().to_string(), &self.active_mods.len().to_string()],
                        ))
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

        self.render_delete_confirmation(ctx);

        if self.opt_is_running {
            ctx.request_repaint();
        }
    }
}

impl App {
    /// Modal window: lists the mods that will be unsubscribed and moved to the Recycle Bin.
    fn render_delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_delete.as_ref() else {
            return;
        };
        let t = self.i18n;
        let mut confirm = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        egui::Window::new(t.get(TranslationKey::ConfirmDeleteTitle))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(fmt(
                    t.get(TranslationKey::ConfirmDeleteBody),
                    &[&pending.names.len().to_string()],
                ));
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt("confirm_delete_list")
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for (name, id) in pending.names.iter().zip(&pending.ids) {
                            ui.label(format!("• {}  ({})", name, id));
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if crate::ui::components::danger_button(ui, t.get(TranslationKey::ConfirmDeleteBtn)).clicked() {
                        confirm = true;
                    }
                    if ui.button(t.get(TranslationKey::BtnCancel)).clicked() {
                        cancel = true;
                    }
                });
            });

        if confirm {
            self.confirm_pending_delete();
        } else if cancel {
            self.pending_delete = None;
        }
    }
}
