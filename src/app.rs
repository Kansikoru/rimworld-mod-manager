use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui::{self, Color32, Rounding, Stroke};

use crate::analyzer::{analyze_rimworld_log, detect_rimworld_log_paths, LogReport};
use crate::cleaner::{delete_translation_mods, unsubscribe_and_delete_single};
use crate::community_rules::CommunityDatabase;
use crate::dependencies::{validate_dependencies, DependencyProblem, DependencyReport};
use crate::categories::ModCategory;
use crate::game_install::known_launch_flags;
use crate::i18n::{fmt, tr, I18n, Language, TranslationKey};
use crate::matcher::{classify_and_link_translations, filter_and_rank_workshop_translations};
use crate::models::{is_protected_package_id, is_optimizer_blacklisted, ModInfo, TranslationTargetLang};
use crate::mods_config::{get_default_mods_config_path, load_active_mods, save_active_mods};
use crate::optimizer::{
    default_cpu_threads, dirs_from_mods, rollback_dds_async, run_optimization, texture_summary_async,
    total_system_threads, ConvertOptions, OptimizationMsg, OptimizeScope, RollbackResult, TextureSummary,
};
use crate::scanner::scan_mods;
use crate::sorter::sort_load_order_with_db;
use crate::steam::{
    detect_steam_paths, launch_rimworld, subscribe_workshop_item_native, SteamPaths,
};
use crate::ui::icons::*;
use crate::ui::theme::THEME;
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
    /// Flip textures vertically before encoding (RimPy style). Off by default.
    pub opt_flip: bool,
    /// On RimWorld 1.7 conversion is blocked unless the user explicitly allows it.
    pub opt_allow_on_waylith: bool,
    /// Background "Scan" result.
    pub opt_scan_rx: Option<Receiver<TextureSummary>>,
    /// Background "Rollback" result.
    pub opt_rollback_rx: Option<Receiver<RollbackResult>>,

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
    /// Paths picked by hand in Settings (None = auto-detected). Saved between runs.
    pub custom_game_path: Option<PathBuf>,
    pub custom_workshop_path: Option<PathBuf>,
    pub custom_config_path: Option<PathBuf>,
    /// Last settings written to disk; saved again whenever something differs.
    last_saved_settings: crate::settings::AppSettings,
    pub community_db_rules_count: usize,
    /// Built-in rules + RimSort/RimPy community rules (S14), used by the sorter.
    pub community_db: CommunityDatabase,
    /// Background download of the community rules.
    pub rules_rx: Option<Receiver<Result<usize, String>>>,
    /// Background Steam subscribe request; yields the status message.
    pub steam_rx: Option<Receiver<String>>,

    /// Deletion waiting for confirmation (modal window).
    pub pending_delete: Option<PendingDelete>,

    /// Mods found more than once on disk (same packageId in several folders).
    pub duplicate_mods: Vec<crate::scanner::DuplicateModEntry>,

    /// Categories set by the user (lowercase packageId -> categories), saved to disk.
    pub category_overrides: std::collections::HashMap<String, Vec<ModCategory>>,
    /// Mod list filter: show only mods with any of these categories (empty = all).
    pub category_filter: std::collections::HashSet<ModCategory>,

    /// Interface font and text size (Settings), saved to ui_prefs.json.
    pub ui_prefs: crate::ui_prefs::UiPrefs,

    // Background tasks (S5): the UI never waits for disk or network.
    /// Lets background threads wake the UI when they finish.
    pub repaint_ctx: Option<egui::Context>,
    pub rescan_rx: Option<Receiver<RescanResult>>,
    /// Keep the current status message when the running rescan finishes.
    rescan_keep_status: bool,
    /// A rescan was requested while one was running: run again when it finishes.
    rescan_again: bool,
    pub workshop_rx: Option<Receiver<WorkshopSearchOutcome>>,
    pub log_rx: Option<Receiver<LogReport>>,

    /// Lowercase packageId -> index in `all_mods` (M1: no linear search per row).
    pub mod_index: std::collections::HashMap<String, usize>,
    /// Measured row heights of the two mod lists (for drawing only the visible rows).
    pub active_row_height: f32,
    pub inactive_row_height: f32,

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
        // Settings from the previous run (S13). Hand-picked paths win over auto-detection.
        let saved = crate::settings::AppSettings::load_from(&crate::settings::AppSettings::path());
        let s = saved.clone().unwrap_or_default();

        let i18n = I18n::new(s.language);
        let mut steam_paths = detect_steam_paths();
        let custom_game_path = s.game_path.clone().filter(|p| p.is_dir());
        if let Some(ref game) = custom_game_path {
            steam_paths.set_game_dir(game.clone());
        }
        let custom_workshop_path = s.workshop_path.clone().filter(|p| p.is_dir());
        if let Some(ref ws) = custom_workshop_path {
            steam_paths.workshop_path = Some(ws.clone());
        }
        let custom_config_path = s.mods_config_path.clone().filter(|p| p.is_file());
        let mods_config_path = custom_config_path.clone().or_else(get_default_mods_config_path);
        let max_system_threads = total_system_threads();
        let default_threads = s
            .opt_threads
            .map(|n| n.clamp(1, max_system_threads))
            .unwrap_or_else(default_cpu_threads);
        let launch_flags: std::collections::HashSet<String> = if saved.is_some() {
            s.launch_flags.iter().cloned().collect()
        } else {
            ["-logfile".to_string()].into_iter().collect()
        };
        let community_db = crate::community_rules::load_effective_db();
        let db_rules_count = community_db.rules.len();

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
        let icon_repaint = repaint_ctx.clone();
        std::thread::spawn(move || {
            while let Ok((pkg_id, path)) = icon_thread_rx.recv() {
                if let Some(img) = crate::icon_cache::load_or_cache_image(&cache_clone, &path, 64) {
                    if icon_thread_tx.send((pkg_id, img)).is_err() {
                        break;
                    }
                    if let Some(ref ctx) = icon_repaint {
                        ctx.request_repaint();
                    }
                }
            }
        });

        let mut app = Self {
            current_tab: AppTab::Mods,
            i18n,
            is_russian: s.language == Language::Russian,

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
            opt_flip: false,
            opt_allow_on_waylith: false,
            opt_scan_rx: None,
            opt_rollback_rx: None,

            log_report: None,
            custom_log_path: None,

            steam_paths,
            mods_config_path,
            launch_flags,
            launch_extra_args: s.launch_extra_args.clone(),
            custom_game_path,
            custom_workshop_path,
            custom_config_path,
            last_saved_settings: s,
            community_db_rules_count: db_rules_count,
            community_db,
            rules_rx: None,
            steam_rx: None,

            pending_delete: None,
            duplicate_mods: Vec::new(),

            category_overrides: crate::categories::load_overrides(&crate::categories::overrides_path()),
            category_filter: std::collections::HashSet::new(),
            ui_prefs: crate::ui_prefs::UiPrefs::load(),

            repaint_ctx: repaint_ctx.clone(),
            rescan_rx: None,
            rescan_keep_status: false,
            rescan_again: false,
            workshop_rx: None,
            log_rx: None,
            mod_index: std::collections::HashMap::new(),
            active_row_height: 34.0,
            inactive_row_height: 34.0,

            status_message: tr!("Готов к работе", "Ready").to_string(),
        };

        // Scans in the background: the window opens at once and fills in when the scan is done.
        app.rescan_mods();
        app
    }

    /// Runs `job` on a background thread; the result arrives on the returned channel
    /// and the UI is woken up to show it.
    fn spawn_task<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Receiver<T> {
        let (tx, rx) = channel();
        let ctx = self.repaint_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        rx
    }

    /// Picks up finished background tasks (called every frame).
    fn poll_background_tasks(&mut self) {
        if let Some(rx) = &self.rescan_rx {
            if let Ok(res) = rx.try_recv() {
                self.rescan_rx = None;
                self.apply_rescan(res);
            }
        }
        if let Some(rx) = &self.workshop_rx {
            if let Ok(res) = rx.try_recv() {
                self.workshop_rx = None;
                match res {
                    Ok((items, status)) => {
                        self.workshop_search_results = items;
                        self.status_message = status;
                    }
                    Err(e) => {
                        self.status_message = tr!("Ошибка поиска в Workshop: {}", "Workshop search failed: {}", e)
                    }
                }
            }
        }
        if let Some(rx) = &self.steam_rx {
            if let Ok(msg) = rx.try_recv() {
                self.steam_rx = None;
                self.status_message = msg;
            }
        }
        if let Some(rx) = &self.rules_rx {
            if let Ok(res) = rx.try_recv() {
                self.rules_rx = None;
                self.finish_rules_update(res);
            }
        }
        if let Some(rx) = &self.log_rx {
            if let Ok(report) = rx.try_recv() {
                self.log_rx = None;
                self.status_message = self.crash_summary_text(&report);
                self.log_report = Some(report);
            }
        }
    }

    pub fn is_rescanning(&self) -> bool {
        self.rescan_rx.is_some()
    }

    fn rebuild_mod_index(&mut self) {
        self.mod_index = self
            .all_mods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.package_id.to_lowercase(), i))
            .collect();
    }
}

/// Result of a background Workshop search: (items, status text) or an error message.
pub type WorkshopSearchOutcome = Result<(Vec<WorkshopItemResult>, String), String>;

/// Result of a background mod scan.
pub struct RescanResult {
    pub mods: Vec<ModInfo>,
    pub duplicates: Vec<crate::scanner::DuplicateModEntry>,
}

/// Index-based lookup (M1); understands the `_steam` suffix like `find_mod_in`.
pub fn lookup_mod<'a>(
    index: &std::collections::HashMap<String, usize>,
    mods: &'a [ModInfo],
    id: &str,
) -> Option<&'a ModInfo> {
    let lower = id.to_lowercase();
    index
        .get(&lower)
        .or_else(|| lower.strip_suffix("_steam").and_then(|base| index.get(base)))
        .and_then(|&i| mods.get(i))
}

/// Adds `id` (lower case) unless the list already has it in any letter case.
pub fn add_active_id(active: &mut Vec<String>, id: &str) -> bool {
    if active.iter().any(|a| a.eq_ignore_ascii_case(id)) {
        return false;
    }
    active.push(id.to_lowercase());
    true
}

/// Enables installed-but-disabled dependencies until none are left, so the
/// dependencies of newly enabled mods are enabled too. Returns how many were added.
pub fn enable_dependencies_recursive(active: &mut Vec<String>, all_mods: &[ModInfo]) -> usize {
    let mut added = 0;
    // Each pass enables at least one mod, so the number of passes is bounded.
    for _ in 0..=all_mods.len() {
        let report = validate_dependencies(active, all_mods);
        let mut changed = false;
        for prob in report.problems {
            if let DependencyProblem::Disabled { required_id, .. } = prob {
                if add_active_id(active, &required_id) {
                    added += 1;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    added
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
        lookup_mod(&self.mod_index, &self.all_mods, id)
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
            self.status_message = tr!("Не удалось сохранить категории: {}", "Could not save categories: {}", e);
        }
    }

    /// Switches the game folder from Settings: recomputes engine, exe, Steam DLL, Mods/Data and rescans.
    pub fn set_game_path(&mut self, folder: PathBuf) {
        self.custom_game_path = Some(folder.clone());
        self.steam_paths.set_game_dir(folder);
        self.rescan_mods();
    }

    pub fn set_workshop_path(&mut self, folder: PathBuf) {
        self.custom_workshop_path = Some(folder.clone());
        self.steam_paths.workshop_path = Some(folder);
        self.rescan_mods();
    }

    pub fn set_mods_config_path(&mut self, file: PathBuf) {
        self.custom_config_path = Some(file.clone());
        self.mods_config_path = Some(file);
        self.rescan_mods();
    }

    /// Forgets hand-picked paths and detects everything again via Steam.
    pub fn reset_paths_to_auto(&mut self) {
        self.custom_game_path = None;
        self.custom_workshop_path = None;
        self.custom_config_path = None;
        self.steam_paths = detect_steam_paths();
        self.mods_config_path = get_default_mods_config_path();
        self.rescan_mods();
    }

    fn settings_snapshot(&self) -> crate::settings::AppSettings {
        let mut flags: Vec<String> = self.launch_flags.iter().cloned().collect();
        flags.sort();
        crate::settings::AppSettings {
            game_path: self.custom_game_path.clone(),
            workshop_path: self.custom_workshop_path.clone(),
            mods_config_path: self.custom_config_path.clone(),
            language: self.i18n.lang,
            launch_flags: flags,
            launch_extra_args: self.launch_extra_args.clone(),
            opt_threads: Some(self.opt_threads),
        }
    }

    /// Saves settings when anything changed (called every frame; writes only on change).
    fn save_settings_if_changed(&mut self) {
        let snap = self.settings_snapshot();
        let mut last = self.last_saved_settings.clone();
        last.launch_flags.sort();
        if snap != last {
            match snap.save_to(&crate::settings::AppSettings::path()) {
                Ok(()) => self.last_saved_settings = snap,
                Err(e) => {
                    self.status_message = tr!("Не удалось сохранить настройки: {}", "Could not save settings: {}", e);
                    self.last_saved_settings = snap; // don't retry every frame
                }
            }
        }
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

    /// Starts a background scan of the mod folders (S5). If one is already running,
    /// another scan follows right after it, so the newest disk state always wins.
    pub fn rescan_mods(&mut self) {
        if self.rescan_rx.is_some() {
            self.rescan_again = true;
            return;
        }
        if !self.rescan_keep_status {
            self.status_message = tr!("Сканирование модов на диске...", "Scanning mods on disk...").to_string();
        }

        let workshop = self.steam_paths.workshop_path.clone();
        let local = self.steam_paths.local_mods_path.clone();
        let data = self.steam_paths.game_data_path.clone();
        let overrides = self.category_overrides.clone();
        let game_mm = self.steam_paths.game_major_minor();
        self.rescan_rx = Some(self.spawn_task(move || {
            let scanned = scan_mods(workshop.as_deref(), local.as_deref(), data.as_deref());
            // Two copies with the same packageId show up twice in the game's mod list: keep one, warn about it.
            let (mut unique, duplicates) = crate::scanner::dedupe_mods(scanned);
            // `*ByVersion` blocks of About.xml for the installed game version (S16).
            crate::scanner::resolve_versioned_fields(&mut unique, game_mm.as_deref());
            classify_and_link_translations(&mut unique);
            crate::categories::assign_categories(&mut unique, &overrides);
            RescanResult { mods: unique, duplicates }
        }));
    }

    fn apply_rescan(&mut self, res: RescanResult) {
        // Overrides may have changed while scanning: apply the current ones.
        let mut mods = res.mods;
        crate::categories::assign_categories(&mut mods, &self.category_overrides);
        self.all_mods = mods;
        self.duplicate_mods = res.duplicates;
        self.rebuild_mod_index();

        // Drop cached thumbnails of mods that are gone (the cache folder doesn't grow forever).
        let previews: std::collections::HashSet<PathBuf> =
            self.all_mods.iter().filter_map(|m| m.preview_path.clone()).collect();
        std::thread::spawn(move || {
            crate::icon_cache::IconDiskCache::new(crate::icon_cache::IconDiskCache::default_cache_dir())
                .cleanup_unreferenced(&previews);
        });

        if let Some(ref config_p) = self.mods_config_path {
            // Keep IDs that were not found on disk: they stay in ModsConfig.xml on save
            // and are shown as "not on disk" instead of being silently dropped.
            self.active_mods = load_active_mods(config_p);
        }

        self.validate_deps();
        if self.rescan_keep_status {
            self.rescan_keep_status = false;
        } else {
            self.status_message = self.mods_count_status();
        }

        if self.rescan_again {
            self.rescan_again = false;
            self.rescan_mods();
        }
    }

    pub fn validate_deps(&mut self) {
        self.dependency_report = validate_dependencies(&self.active_mods, &self.all_mods);
    }

    pub fn sort_active_mods(&mut self) {
        self.active_mods = sort_load_order_with_db(&self.active_mods, &self.all_mods, Some(&self.community_db));
        self.validate_deps();
        self.status_message = self.i18n.get(TranslationKey::AppSortedMsg).to_string();
    }

    pub fn save_config(&mut self) {
        if let Some(ref config_p) = self.mods_config_path {
            match save_active_mods(config_p, &self.active_mods) {
                Ok(_) => {
                    self.status_message = tr!("ModsConfig.xml сохранён", "ModsConfig.xml saved").to_string();
                }
                Err(e) => {
                    self.status_message = tr!("Ошибка сохранения ModsConfig.xml: {}", "Could not save ModsConfig.xml: {}", e);
                }
            }
        }
    }

    /// Adds a mod to the end of the load order unless it is already there (IDs are
    /// compared case-insensitively and stored in lower case, like ModsConfig.xml does).
    /// Returns whether it was added.
    pub fn activate_mod(&mut self, id: &str) -> bool {
        add_active_id(&mut self.active_mods, id)
    }

    pub fn activate_all_mods(&mut self) {
        let ids: Vec<String> = self.all_mods.iter().map(|m| m.package_id.clone()).collect();
        for id in ids {
            self.activate_mod(&id);
        }
        self.validate_deps();
    }

    pub fn deactivate_all_mods(&mut self) {
        self.active_mods.retain(|id| is_protected_package_id(id));
        self.validate_deps();
    }

    /// Enables every missing dependency, including dependencies of the mods it just
    /// enabled, then sorts the load order once.
    pub fn auto_fix_dependencies(&mut self) {
        let added = enable_dependencies_recursive(&mut self.active_mods, &self.all_mods);
        self.active_mods = sort_load_order_with_db(&self.active_mods, &self.all_mods, Some(&self.community_db));
        self.validate_deps();
        self.status_message = if added > 0 {
            tr!(
                "Включено зависимостей: {}, порядок загрузки отсортирован",
                "Enabled dependencies: {}, load order sorted",
                added
            )
        } else {
            self.i18n.get(TranslationKey::AppSortedMsg).to_string()
        };
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
            self.status_message = tr!(
                "Защищённый мод '{}' нельзя удалить",
                "Protected mod '{}' can't be deleted",
                m.package_id
            );
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
                let mut msg = tr!(
                    "Перемещено в Корзину: {} | Отписано в Steam: {}",
                    "Moved to Recycle Bin: {} | Unsubscribed in Steam: {}",
                    res.deleted_count,
                    res.unsubscribed_count
                );
                if let Some(err) = res.errors.first() {
                    msg.push_str(&tr!(" | Ошибок: {} ({})", " | Errors: {} ({})", res.errors.len(), err));
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
                        self.status_message =
                            tr!("Мод {} перемещён в Корзину", "Mod {} moved to the Recycle Bin", name);
                        self.rescan_mods_keep_status();
                    }
                    Err(e) => {
                        self.status_message = tr!("Ошибка удаления: {}", "Delete failed: {}", e);
                    }
                }
            }
        }
    }

    /// Rescan without replacing the current status message (e.g. "moved to Recycle Bin").
    fn rescan_mods_keep_status(&mut self) {
        self.rescan_keep_status = true;
        self.rescan_mods();
    }

    /// Searches the Workshop for translations of the selected mod — in the background (S5).
    pub fn search_workshop_translations_for_selected(&mut self) {
        if self.workshop_rx.is_some() {
            return;
        }
        let Some(m) = self.selected_mod_id.as_deref().and_then(|id| self.find_mod(id)) else {
            self.status_message = tr!(
                "Выберите мод в списке, чтобы найти для него перевод",
                "Select a mod in the list to find a translation for it"
            )
            .to_string();
            return;
        };
        let name = m.name.clone();
        let steam_id = m.steam_item_id.clone();
        let lang = self.trans_target_lang;
        let query = format!("{} {}", name, lang.search_suffix());
        self.workshop_search_query = query.clone();
        self.status_message = tr!("Поиск в Workshop: {}...", "Searching the Workshop: {}...", query);

        self.workshop_rx = Some(self.spawn_task(move || {
            let raw = search_steam_workshop(&query)?;
            let found = raw.len();
            // Keep only translations of this mod in the selected language,
            // drop the mod itself and unrelated items, best matches first.
            let filtered = filter_and_rank_workshop_translations(raw, &name, steam_id.as_deref(), lang);
            let status = tr!(
                "Workshop: найдено {}, подходящих переводов «{}»: {}",
                "Workshop: found {}, matching translations of \"{}\": {}",
                found,
                name,
                filtered.len()
            );
            Ok((filtered, status))
        }));
    }

    /// Free-text Workshop search — in the background (S5).
    pub fn search_workshop_custom(&mut self) {
        if self.workshop_rx.is_some() || self.workshop_search_query.trim().is_empty() {
            return;
        }
        let query = self.workshop_search_query.clone();
        self.status_message = tr!("Поиск в Workshop: {}...", "Searching the Workshop: {}...", query);
        self.workshop_rx = Some(self.spawn_task(move || {
            let res = search_steam_workshop(&query)?;
            let status = tr!(
                "Поиск в Workshop «{}»: результатов {}",
                "Workshop search \"{}\": {} results",
                query,
                res.len()
            );
            Ok((res, status))
        }));
    }

    /// Read-only Steam API diagnostics (Settings): starts the API, counts subscriptions.
    pub fn check_steam_api(&mut self) {
        if self.steam_rx.is_some() {
            return;
        }
        let Some(dll) = self.steam_paths.steam_dll_path.clone() else {
            self.status_message = tr!(
                "steam_api64.dll не найден — проверьте папку игры",
                "steam_api64.dll not found — check the game folder"
            )
            .to_string();
            return;
        };
        self.status_message = tr!("Проверка Steam API...", "Checking the Steam API...").to_string();
        self.steam_rx = Some(self.spawn_task(move || match crate::steam::steam_api_check(&dll) {
            Ok(n) => tr!(
                "Steam API работает: подписок на моды RimWorld — {}",
                "Steam API works: RimWorld mod subscriptions — {}",
                n
            ),
            Err(e) => tr!("Steam API не работает: {}", "Steam API doesn't work: {}", e),
        }));
    }

    /// Subscribes via the Steam API in the background (the request needs ~1.5 s to reach Steam).
    pub fn subscribe_steam_mod(&mut self, item_id: &str) {
        if self.steam_rx.is_some() {
            return;
        }
        if let Some(dll) = self.steam_paths.steam_dll_path.clone() {
            let id = item_id.to_string();
            self.status_message = tr!("Отправка запроса на подписку #{}...", "Sending subscribe request #{}...", id);
            self.steam_rx = Some(self.spawn_task(move || match subscribe_workshop_item_native(&dll, &id) {
                Ok(_) => tr!(
                    "Запрос на подписку #{} отправлен в Steam — мод скачается в клиенте Steam",
                    "Subscribe request #{} sent to Steam — the Steam client will download the mod",
                    id
                ),
                Err(e) => tr!("Ошибка подписки: {}", "Subscribe failed: {}", e),
            }));
        } else {
            crate::ui::components::open_external(crate::steam::workshop_url(item_id));
            self.status_message =
                tr!("Открыта страница Steam Workshop #{}", "Opened Steam Workshop page #{}", item_id);
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
            self.status_message =
                tr!("Нет выбранных модов для оптимизации текстур", "No mods selected for texture optimization")
                    .to_string();
            return;
        }
        if self.optimizer_blocked() {
            self.status_message = tr!(
                "RimWorld 1.7 сжимает текстуры сам — конвертация отключена (можно разрешить вручную)",
                "RimWorld 1.7 compresses textures itself — conversion is off (can be allowed manually)"
            )
            .to_string();
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
            ConvertOptions { flip_vertical: self.opt_flip },
            self.opt_threads,
            tx,
            self.opt_stop_flag.clone(),
        );

        self.status_message = tr!(
            "Запущена конвертация текстур (BC7 / BC1 с мипмапами)...",
            "Converting textures (BC7 / BC1 with mipmaps)..."
        )
        .to_string();
    }

    /// Conversion is blocked on 1.7 (the game compresses textures itself) unless explicitly allowed.
    pub fn optimizer_blocked(&self) -> bool {
        self.steam_paths.engine == crate::game_install::GameEngine::Waylith && !self.opt_allow_on_waylith
    }

    /// RimWorld 1.7: moves TextureCache and ShaderCache to the Recycle Bin (game rebuilds them).
    pub fn clear_game_caches(&mut self) {
        let size_mb = crate::game_install::game_cache_size() as f64 / (1024.0 * 1024.0);
        self.status_message = match crate::game_install::clear_game_caches() {
            Ok(cleared) if cleared.is_empty() => tr!("Кэш игры уже пуст", "The game cache is already empty").to_string(),
            Ok(cleared) => tr!(
                "Кэш игры перемещён в Корзину ({}, {:.0} МБ). Игра пересоберёт его при следующем запуске.",
                "Game cache moved to the Recycle Bin ({}, {:.0} MB). The game rebuilds it on the next start.",
                cleared.join(", "),
                size_mb
            ),
            Err(e) => tr!("Не удалось очистить кэш: {}", "Could not clear the cache: {}", e),
        };
    }

    pub fn cancel_texture_optimization(&mut self) {
        self.opt_stop_flag.store(true, Ordering::Relaxed);
        self.status_message = tr!("Отмена оптимизации...", "Cancelling optimization...").to_string();
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

    /// "Scan": counts in a background thread (no UI freeze on big mod folders).
    pub fn scan_texture_counts(&mut self) {
        let dirs = dirs_from_mods(&self.optimizable_mods());
        let (tx, rx) = channel();
        texture_summary_async(dirs, self.opt_force, tx);
        self.opt_scan_rx = Some(rx);
        self.status_message = tr!("Подсчёт текстур...", "Counting textures...").to_string();
    }

    /// Rolls back every mod except Core/DLC — including ignored/blacklisted ones,
    /// because older versions of the optimizer did not respect those lists. Runs in background.
    pub fn rollback_textures(&mut self) {
        let mods: Vec<ModInfo> = self
            .all_mods
            .iter()
            .filter(|m| !self.is_official_mod(m))
            .cloned()
            .collect();
        let (tx, rx) = channel();
        rollback_dds_async(dirs_from_mods(&mods), tx);
        self.opt_rollback_rx = Some(rx);
        self.status_message = tr!("Откат текстур...", "Rolling back textures...").to_string();
    }

    /// Whether a background optimizer task (scan, rollback, conversion) is running.
    pub fn optimizer_busy(&self) -> bool {
        self.opt_is_running || self.opt_scan_rx.is_some() || self.opt_rollback_rx.is_some()
    }

    fn poll_optimizer_tasks(&mut self) {
        if let Some(rx) = &self.opt_scan_rx {
            if let Ok(s) = rx.try_recv() {
                self.status_message = tr!(
                    "Будет сконвертировано: {} | уже готово: {} | чужих DDS (не трогаются): {}",
                    "To convert: {} | already done: {} | author DDS (left alone): {}",
                    s.to_convert,
                    s.up_to_date,
                    s.foreign_dds
                );
                self.opt_scan_rx = None;
            }
        }
        if let Some(rx) = &self.opt_rollback_rx {
            if let Ok(res) = rx.try_recv() {
                self.status_message = tr!(
                    "Откат: удалено .dds: {} | восстановлено исходников из бэкапа: {} | ошибок: {}",
                    "Rollback: .dds removed: {} | sources restored from backup: {} | errors: {}",
                    res.removed,
                    res.restored,
                    res.errors
                );
                self.opt_rollback_rx = None;
            }
        }
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

        if self.log_rx.is_some() {
            return;
        }
        if let Some(path) = log_target {
            // Matching stack frames to mods walks every mod folder: done in the background (S5).
            let mods = self.all_mods.clone();
            self.status_message = tr!("Анализ лога {}...", "Analyzing log {}...", path.display());
            self.log_rx = Some(self.spawn_task(move || analyze_rimworld_log(&path, &mods)));
        } else {
            self.status_message = self.i18n.get(TranslationKey::CrashFileNotFound).to_string();
        }
    }

    fn crash_summary_text(&self, report: &LogReport) -> String {
        if report.has_hard_crash {
            fmt(self.i18n.get(TranslationKey::CrashSummaryHardCrash), &[&report.culprits.len().to_string()])
        } else if !report.culprits.is_empty() {
            fmt(self.i18n.get(TranslationKey::CrashSummaryCulprits), &[&report.culprits.len().to_string()])
        } else if report.total_errors > 0 {
            fmt(self.i18n.get(TranslationKey::CrashSummaryErrorsOnly), &[&report.total_errors.to_string()])
        } else {
            self.i18n.get(TranslationKey::CrashSummaryClean).to_string()
        }
    }

    pub fn launch_game(&mut self) {
        let args = self.launch_args();
        match launch_rimworld(&self.steam_paths, &args) {
            Ok(_) => {
                self.status_message = tr!("RimWorld запущен", "RimWorld started").to_string();
            }
            Err(e) => {
                self.status_message = tr!("Ошибка запуска игры: {}", "Could not start the game: {}", e);
            }
        }
    }

    /// Downloads the RimSort community rules in the background (S14).
    pub fn update_community_rules(&mut self) {
        if self.rules_rx.is_some() {
            return;
        }
        self.status_message =
            tr!("Загрузка базы правил RimSort с GitHub...", "Downloading RimSort rules from GitHub...").to_string();
        self.rules_rx = Some(self.spawn_task(crate::community_rules::download_rules));
    }

    /// Imports a communityRules.json (RimPy / RimSort) from disk (S14).
    pub fn import_community_rules(&mut self, file: &std::path::Path) {
        let res = crate::community_rules::import_rules_file(file);
        self.finish_rules_update(res);
    }

    fn finish_rules_update(&mut self, res: Result<usize, String>) {
        match res {
            Ok(imported) => {
                self.community_db = crate::community_rules::load_effective_db();
                self.community_db_rules_count = self.community_db.rules.len();
                self.status_message = tr!(
                    "База правил обновлена: правил сообщества {}, всего с встроенными {}",
                    "Rules updated: {} community rules, {} including built-in ones",
                    imported,
                    self.community_db_rules_count
                );
            }
            Err(e) => {
                self.status_message = tr!("Не удалось обновить базу правил: {}", "Could not update the rules: {}", e)
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process loaded mod icons
        while let Ok((pkg_id, img)) = self.icon_rx.try_recv() {
            let handle = ctx.load_texture(
                &pkg_id,
                img,
                Default::default()
            );
            self.mod_textures.insert(pkg_id, Some(handle));
        }

        self.poll_background_tasks();
        self.poll_optimizer_tasks();
        if self.opt_scan_rx.is_some() || self.opt_rollback_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
                    OptimizationMsg::Done(s) => {
                        self.opt_is_running = false;
                        let mut msg = tr!(
                            "Готово: BC7 {}, BC1 {} | оставлено PNG (размер): {} | уже актуально: {} | ошибок: {}",
                            "Done: BC7 {}, BC1 {} | kept as PNG (size): {} | up to date: {} | errors: {}",
                            s.bc7,
                            s.bc1,
                            s.skipped,
                            s.up_to_date,
                            s.errors
                        );
                        if s.sources_backed_up > 0 {
                            msg.push_str(&tr!(
                                " | исходников в бэкапе: {}",
                                " | sources backed up: {}",
                                s.sources_backed_up
                            ));
                        }
                        if s.foreign_dds > 0 {
                            msg.push_str(&tr!(" | чужих DDS не тронуто: {}", " | author DDS left alone: {}", s.foreign_dds));
                        }
                        self.status_message = msg;
                    }
                    OptimizationMsg::Cancelled => {
                        self.opt_is_running = false;
                        self.status_message =
                            tr!("Оптимизация остановлена", "Optimization stopped").to_string();
                    }
                }
            }
        }

        // Top Navigation Bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // App Logo / Title
                ui.add(egui::Image::new(ICON_LOGO).fit_to_exact_size(egui::vec2(24.0, 24.0)));
                ui.heading(
                    egui::RichText::new("RimWorld Mod Manager")
                        .color(THEME.accent_cyan)
                        .size(17.0)
                        .strong(),
                );

                ui.label(
                    egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                        .color(THEME.text_muted)
                        .size(12.0),
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
                            svg_icon(icon, egui::vec2(16.0, 16.0), text_color),
                            egui::RichText::new(label)
                                .color(text_color)
                                .size(13.0)
                                .strong(),
                        )
                        .fill(bg)
                        .stroke(Stroke::new(1.0_f32, border))
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
                            svg_icon(ICON_PLAY, egui::vec2(16.0, 16.0), Color32::WHITE),
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
                        .size(12.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(fmt(
                            self.i18n.get(TranslationKey::AppStatusBarStats),
                            &[&self.all_mods.len().to_string(), &self.active_mods.len().to_string()],
                        ))
                        .color(THEME.text_muted)
                        .size(12.0),
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
        self.save_settings_if_changed();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_by_index_understands_case_and_steam_suffix() {
        let mods = vec![
            ModInfo { package_id: "Brrainz.Harmony".into(), ..Default::default() },
            ModInfo { package_id: "UnlimitedHugs.HugsLib".into(), ..Default::default() },
        ];
        let index: std::collections::HashMap<String, usize> =
            mods.iter().enumerate().map(|(i, m)| (m.package_id.to_lowercase(), i)).collect();
        assert_eq!(lookup_mod(&index, &mods, "brrainz.harmony").map(|m| m.package_id.as_str()), Some("Brrainz.Harmony"));
        assert_eq!(lookup_mod(&index, &mods, "unlimitedhugs.hugslib_steam").map(|m| m.package_id.as_str()), Some("UnlimitedHugs.HugsLib"));
        assert!(lookup_mod(&index, &mods, "someone.missing").is_none());
    }

    fn with_deps(id: &str, deps: &[&str]) -> ModInfo {
        ModInfo {
            package_id: id.into(),
            name: id.into(),
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn add_active_id_ignores_case_and_stores_lowercase() {
        let mut active = vec!["ludeon.rimworld".to_string()];
        assert!(!add_active_id(&mut active, "Ludeon.RimWorld"));
        assert!(add_active_id(&mut active, "Author.CoolMod"));
        assert!(!add_active_id(&mut active, "author.coolmod"));
        assert_eq!(active, ["ludeon.rimworld", "author.coolmod"]);
    }

    #[test]
    fn dependencies_are_enabled_on_every_level() {
        // A needs B, B needs C, C needs D: enabling A's dependencies must bring all three.
        let mods = vec![
            with_deps("Test.A", &["Test.B"]),
            with_deps("Test.B", &["Test.C"]),
            with_deps("Test.C", &["Test.D"]),
            with_deps("Test.D", &[]),
            with_deps("Test.Unrelated", &[]),
        ];
        let mut active = vec!["test.a".to_string()];
        assert_eq!(enable_dependencies_recursive(&mut active, &mods), 3);
        for id in ["test.b", "test.c", "test.d"] {
            assert!(active.iter().any(|a| a == id), "{id} not enabled: {active:?}");
        }
        assert!(!active.iter().any(|a| a == "test.unrelated"));
        // Nothing left to enable: a second run adds nothing.
        assert_eq!(enable_dependencies_recursive(&mut active, &mods), 0);
    }
}
