use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui::{self, Rounding};

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

fn bisect_session_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("bisect.json")
}

/// An unfinished broken-mod search from an earlier run, if any.
fn load_bisect_session() -> Option<crate::bisect::Bisect> {
    let text = std::fs::read_to_string(bisect_session_path()).ok()?;
    serde_json::from_str(&text).ok()
}

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

/// Question shown in a modal window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingQuestion {
    /// "Play" pressed with an unsaved load order.
    LaunchUnsaved,
    /// Window closed with an unsaved load order.
    CloseUnsaved,
    /// Rollback found DDS files that are not in the manager's log (old optimizer, RimPy
    /// or the mod author). Each has a PNG/JPG next to it.
    LegacyDds(Vec<PathBuf>),
}

/// Window for a mod set action that needs a name or a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModListDialog {
    /// Save the current load order as a new set.
    SaveAs { name: String, error: Option<String> },
    Rename { from: String, name: String, error: Option<String> },
    Delete { name: String },
}

/// Translations of one mod found by the modpack-wide search.
#[derive(Debug, Clone)]
pub struct ModTranslations {
    pub mod_name: String,
    pub items: Vec<WorkshopItemResult>,
}

/// Messages from the modpack-wide translation search.
pub enum BatchSearchMsg {
    Progress { done: usize, total: usize },
    Found(ModTranslations),
    Done { searched: usize, errors: usize, cancelled: bool },
}

/// Mods being dragged with the mouse (F1).
#[derive(Debug, Clone)]
pub struct ModDrag {
    /// Dragged mods in list order: IDs from the active list, or package IDs from the available list.
    pub ids: Vec<String>,
    pub from_active: bool,
    /// Text of the label that follows the cursor: the mod name, "+N" for a group.
    pub label: String,
}

/// Where the dragged mods land if the mouse button is released now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    /// Insert before this index of the active list (`len` = at the end).
    Active(usize),
    /// Back to the available list (disable).
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Mods,
    Sets,
    Saves,
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
    /// Modpack-wide translation search: messages, stop flag, progress (done, total).
    pub batch_rx: Option<Receiver<BatchSearchMsg>>,
    pub batch_stop: Arc<AtomicBool>,
    pub batch_progress: (usize, usize),
    /// Mods with translations found, by name; the language they were searched for.
    pub batch_results: Vec<ModTranslations>,
    pub batch_lang: TranslationTargetLang,
    /// Active mods not searched: the language is already in the mod or a translation is installed.
    pub batch_skipped: usize,
    /// Workshop IDs ticked for "Subscribe", and the ones already sent to Steam.
    pub batch_checked: std::collections::HashSet<String>,
    pub batch_subscribed: std::collections::HashSet<String>,
    /// The right column shows the modpack results (true) or the manual search (false).
    pub show_batch_results: bool,
    /// RWMM_TAB=translations-batch: start the modpack search once the mods are scanned (screenshots).
    batch_search_after_scan: bool,
    /// RWMM_TAB=bisect: open the broken-mod search once the mods are scanned (screenshots).
    bisect_after_scan: bool,

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
    /// Optimizer DDS on disk stored the wrong way up for the installed game (counted on every
    /// rescan): made for 1.6 and now played on 1.7, or the other way round.
    pub opt_wrong_orientation: usize,
    /// Background "Flip textures" run.
    pub opt_orient_rx: Option<Receiver<crate::optimizer::OrientationMsg>>,
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
    /// Active list as it is in ModsConfig.xml on disk (lower case): detects unsaved changes.
    pub saved_active_mods: Vec<String>,
    /// The next rescan reads the active list from disk even with unsaved changes
    /// (another ModsConfig.xml was picked).
    reload_active_from_disk: bool,
    /// Question shown in a modal window.
    pub pending_question: Option<PendingQuestion>,
    /// Closing the window was confirmed: don't ask again.
    allow_close: bool,
    /// Background delete (Steam unsubscribe + Recycle Bin); yields the status message.
    pub delete_rx: Option<Receiver<String>>,
    /// Background reset of the 1.7 game cache.
    pub cache_rx: Option<Receiver<String>>,
    /// Background removal of DDS files that are not in the manager's log.
    pub legacy_rx: Option<Receiver<String>>,
    /// Mod sets from the game's ModLists folder (F4), by name.
    pub modlists: Vec<crate::modlists::ModList>,
    /// The set last loaded or saved here.
    pub current_modlist: Option<String>,
    /// Set selected on the Sets tab, and its search query.
    pub selected_modlist: Option<String>,
    pub sets_search: String,
    pub modlist_dialog: Option<ModListDialog>,

    // Saves (F7): headers of the game's saves, read in the background.
    pub saves: Vec<crate::saves::SaveInfo>,
    saves_rx: Option<Receiver<Vec<crate::saves::SaveInfo>>>,
    saves_loaded_at: Option<std::time::Instant>,
    pub selected_save: Option<String>,
    pub saves_search: String,

    // Broken-mod search (F8). The session is saved to bisect.json after every change,
    // so it survives closing the program in the middle.
    pub bisect: Option<crate::bisect::Bisect>,
    pub bisect_open: bool,
    /// Setup screen: mods ticked as "never turn off" (lowercase IDs).
    pub bisect_keep: Option<std::collections::HashSet<String>>,
    pub bisect_filter: String,
    /// Why the last wizard action didn't run (game running, unsaved order...).
    pub bisect_note: Option<String>,
    /// "History" window: ModsConfig.xml backups, newest first (`Some` = window open).
    pub history: Option<Vec<crate::mods_config::ConfigBackup>>,
    /// Look for a new version on GitHub once a day (Settings).
    pub check_updates: bool,
    /// A newer release than this build: shown as a badge next to the version.
    pub update_available: Option<crate::update_check::LatestRelease>,
    pub update_rx: Option<Receiver<Option<crate::update_check::LatestRelease>>>,

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

    /// Drag and drop of mods (F1): what is dragged and where it lands this frame.
    pub mod_drag: Option<ModDrag>,
    pub drop_target: Option<DropTarget>,
    /// Scroll position of the active list, and a new one requested by auto-scroll while dragging.
    pub active_scroll_offset: f32,
    pub active_scroll_to: Option<f32>,

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
        let last_check_updates = s.check_updates;
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

        // RWMM_TAB=settings|optimizer|analyzer|translations|history opens that tab (or the
        // History window) first, for screenshots.
        let start_view = std::env::var("RWMM_TAB").unwrap_or_default().to_lowercase();
        let start_tab = match start_view.as_str() {
            "sets" => AppTab::Sets,
            "saves" => AppTab::Saves,
            "translations" | "translations-batch" => AppTab::Translations,
            "bisect" => AppTab::Analyzer,
            "optimizer" => AppTab::Optimizer,
            "analyzer" => AppTab::Analyzer,
            "settings" => AppTab::Settings,
            _ => AppTab::Mods,
        };

        let bisect_session = load_bisect_session();
        let mut app = Self {
            current_tab: start_tab,
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
            batch_rx: None,
            batch_stop: Arc::new(AtomicBool::new(false)),
            batch_progress: (0, 0),
            batch_results: Vec::new(),
            batch_lang: TranslationTargetLang::Ru,
            batch_skipped: 0,
            batch_checked: std::collections::HashSet::new(),
            batch_subscribed: std::collections::HashSet::new(),
            show_batch_results: false,
            batch_search_after_scan: false,
            bisect_after_scan: false,

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
            opt_wrong_orientation: 0,
            opt_orient_rx: None,
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
            saved_active_mods: Vec::new(),
            reload_active_from_disk: true,
            pending_question: None,
            allow_close: false,
            delete_rx: None,
            cache_rx: None,
            legacy_rx: None,
            history: None,
            modlists: Vec::new(),
            current_modlist: None,
            // RWMM_SET=<name> opens that modpack on the Modpacks tab (screenshots).
            selected_modlist: std::env::var("RWMM_SET").ok().filter(|s| !s.is_empty()),
            sets_search: String::new(),
            modlist_dialog: None,
            saves: Vec::new(),
            saves_rx: None,
            saves_loaded_at: None,
            // RWMM_SAVE=<name> opens that save on the Saves tab (screenshots).
            selected_save: std::env::var("RWMM_SAVE").ok().filter(|s| !s.is_empty()),
            saves_search: String::new(),
            // An unfinished search opens again where it stopped.
            bisect_open: bisect_session.is_some(),
            bisect: bisect_session,
            bisect_keep: None,
            bisect_filter: String::new(),
            bisect_note: None,
            check_updates: last_check_updates,
            update_available: None,
            update_rx: None,
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
            mod_drag: None,
            drop_target: None,
            active_scroll_offset: 0.0,
            active_scroll_to: None,

            status_message: tr!("Готов к работе", "Ready").to_string(),
        };

        // Scans in the background: the window opens at once and fills in when the scan is done.
        app.rescan_mods();
        app.start_update_check();
        if start_view == "history" {
            app.open_history();
        }
        app.batch_search_after_scan = start_view == "translations-batch";
        app.bisect_after_scan = start_view == "bisect";
        app
    }

    /// Shows the last known newer release at once and asks GitHub again if a day has passed.
    pub fn start_update_check(&mut self) {
        use crate::update_check::{cache_path, current_version, is_newer, load_cache, needs_check, now_secs};
        if !self.check_updates || self.update_rx.is_some() {
            return;
        }
        let path = cache_path();
        let cache = load_cache(&path);
        self.update_available = cache.latest.clone().filter(|r| is_newer(&r.tag, current_version()));
        if needs_check(&cache, now_secs()) {
            self.update_rx = Some(self.spawn_task(move || crate::update_check::check_now(&path)));
        }
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
        if let Some(rx) = &self.saves_rx {
            if let Ok(saves) = rx.try_recv() {
                self.saves_rx = None;
                self.saves = saves;
                if let Some(sel) = &self.selected_save {
                    if !self.saves.iter().any(|s| &s.name == sel) {
                        self.selected_save = None;
                    }
                }
            }
        }
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
        if let Some(rx) = &self.delete_rx {
            if let Ok(msg) = rx.try_recv() {
                self.delete_rx = None;
                self.status_message = msg;
                self.rescan_mods_keep_status();
            }
        }
        if let Some(rx) = &self.cache_rx {
            if let Ok(msg) = rx.try_recv() {
                self.cache_rx = None;
                self.status_message = msg;
            }
        }
        if let Some(rx) = &self.legacy_rx {
            if let Ok(msg) = rx.try_recv() {
                self.legacy_rx = None;
                self.status_message = msg;
            }
        }
        self.poll_batch_search();
        if let Some(rx) = &self.update_rx {
            if let Ok(res) = rx.try_recv() {
                self.update_rx = None;
                // Offline (None): keep what the cache said.
                if let Some(latest) = res {
                    let newer = crate::update_check::is_newer(&latest.tag, crate::update_check::current_version());
                    self.update_available = (newer && self.check_updates).then_some(latest);
                }
            }
        }
    }

    /// The load order differs from ModsConfig.xml on disk.
    pub fn has_unsaved_changes(&self) -> bool {
        self.active_mods.len() != self.saved_active_mods.len()
            || self
                .active_mods
                .iter()
                .zip(&self.saved_active_mods)
                .any(|(a, s)| !a.eq_ignore_ascii_case(s))
    }

    fn remember_saved_order(&mut self, on_disk: &[String]) {
        self.saved_active_mods = on_disk.iter().map(|s| s.to_lowercase()).collect();
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
    /// Optimizer DDS on disk stored the wrong way up for the installed game.
    pub optimizer_dds: usize,
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

/// Active list after a rescan: an unsaved load order survives (rescan button, mod deleted, …);
/// without unsaved changes, or when another ModsConfig.xml was picked, the file wins, so
/// outside edits show up. `None` = keep the current list.
pub fn active_after_rescan(unsaved: bool, force_reload: bool, on_disk: &[String]) -> Option<Vec<String>> {
    (force_reload || !unsaved).then(|| on_disk.to_vec())
}

/// Core and DLC: Ludeon package IDs or anything in the game's Data folder.
pub fn is_official_content(m: &ModInfo, game_data: Option<&std::path::Path>) -> bool {
    let id = m.package_id.to_lowercase();
    if is_protected_package_id(&id) || id.starts_with("ludeon.rimworld") {
        return true;
    }
    game_data.is_some_and(|data| m.dir_path.starts_with(data))
}

/// Adds `id` (lower case) unless the list already has it in any letter case.
pub fn add_active_id(active: &mut Vec<String>, id: &str) -> bool {
    if active.iter().any(|a| a.eq_ignore_ascii_case(id)) {
        return false;
    }
    active.push(id.to_lowercase());
    true
}

/// Moves `ids` so they start at `to` (an index in the list before the move), keeping their
/// order. IDs that are not in the list yet are inserted in lower case, so the same call
/// enables mods dragged in from the available list. Case of IDs doesn't matter.
pub fn move_mods(active: &mut Vec<String>, ids: &[String], to: usize) {
    let mut wanted: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids {
        let lower = id.to_lowercase();
        if !wanted.contains(&lower) {
            wanted.push(lower);
        }
    }
    let to = to.min(active.len());
    let before_target = active[..to]
        .iter()
        .filter(|a| wanted.contains(&a.to_lowercase()))
        .count();
    let mut moving = Vec::with_capacity(wanted.len());
    for id in &wanted {
        match active.iter().position(|a| a.eq_ignore_ascii_case(id)) {
            Some(pos) => moving.push(active.remove(pos)),
            None => moving.push(id.clone()),
        }
    }
    let at = to - before_target;
    active.splice(at..at, moving);
}

/// Removes `ids` from the active list; Core and DLC stay. Returns how many were removed.
pub fn remove_mods(active: &mut Vec<String>, ids: &[String]) -> usize {
    let before = active.len();
    active.retain(|a| is_protected_package_id(a) || !ids.iter().any(|id| id.eq_ignore_ascii_case(a)));
    before - active.len()
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
        is_official_content(m, self.steam_paths.game_data_path.as_deref())
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
        self.reload_active_from_disk = true;
        self.rescan_mods();
    }

    /// Forgets hand-picked paths and detects everything again via Steam.
    pub fn reset_paths_to_auto(&mut self) {
        self.custom_game_path = None;
        self.custom_workshop_path = None;
        self.custom_config_path = None;
        self.steam_paths = detect_steam_paths();
        self.mods_config_path = get_default_mods_config_path();
        self.reload_active_from_disk = true;
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
            check_updates: self.check_updates,
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
        let want_flipped = crate::optimizer::engine_wants_flipped(self.steam_paths.engine);
        self.rescan_rx = Some(self.spawn_task(move || {
            let scanned = scan_mods(workshop.as_deref(), local.as_deref(), data.as_deref());
            // Two copies with the same packageId show up twice in the game's mod list: keep one, warn about it.
            let (mut unique, duplicates) = crate::scanner::dedupe_mods(scanned);
            // `*ByVersion` blocks of About.xml for the installed game version (S16).
            crate::scanner::resolve_versioned_fields(&mut unique, game_mm.as_deref());
            classify_and_link_translations(&mut unique);
            crate::categories::assign_categories(&mut unique, &overrides);
            let optimizer_dds = crate::optimizer::count_wrong_orientation(&crate::optimizer::manifest_path(), want_flipped);
            RescanResult { mods: unique, duplicates, optimizer_dds }
        }));
    }

    fn apply_rescan(&mut self, res: RescanResult) {
        // Overrides may have changed while scanning: apply the current ones.
        let mut mods = res.mods;
        crate::categories::assign_categories(&mut mods, &self.category_overrides);
        self.all_mods = mods;
        self.duplicate_mods = res.duplicates;
        self.opt_wrong_orientation = res.optimizer_dds;
        // Sets can be added or changed in the game itself: re-read them with the mods.
        self.refresh_modlists();
        self.rebuild_mod_index();

        // Drop cached thumbnails of mods that are gone (the cache folder doesn't grow forever).
        let previews: std::collections::HashSet<PathBuf> =
            self.all_mods.iter().filter_map(|m| m.preview_path.clone()).collect();
        std::thread::spawn(move || {
            crate::icon_cache::IconDiskCache::new(crate::icon_cache::IconDiskCache::default_cache_dir())
                .cleanup_unreferenced(&previews);
        });

        if let Some(config_p) = self.mods_config_path.clone() {
            // Keep IDs that were not found on disk: they stay in ModsConfig.xml on save
            // and are shown as "not on disk" instead of being silently dropped.
            let on_disk = load_active_mods(&config_p);
            if let Some(list) = active_after_rescan(self.has_unsaved_changes(), self.reload_active_from_disk, &on_disk) {
                self.active_mods = list;
            }
            self.reload_active_from_disk = false;
            self.remember_saved_order(&on_disk);
        }

        self.validate_deps();
        if std::mem::take(&mut self.batch_search_after_scan) {
            self.start_batch_translation_search();
        }
        if std::mem::take(&mut self.bisect_after_scan) {
            self.open_bisect();
        }
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

    /// Writes the load order to ModsConfig.xml. Returns whether it was saved.
    pub fn save_config(&mut self) -> bool {
        let Some(config_p) = self.mods_config_path.clone() else {
            self.status_message =
                tr!("ModsConfig.xml не найден — укажите его в Настройках", "ModsConfig.xml not found — set it in Settings")
                    .to_string();
            return false;
        };
        match save_active_mods(&config_p, &self.active_mods) {
            Ok(_) => {
                let saved = self.active_mods.clone();
                self.remember_saved_order(&saved);
                self.status_message = if crate::game_install::is_game_running() {
                    tr!(
                        "ModsConfig.xml сохранён, но RimWorld запущен: порядок применится после перезапуска игры, а изменения модов в самой игре перезапишут этот файл",
                        "ModsConfig.xml saved, but RimWorld is running: the order applies after a game restart, and changing mods inside the game will overwrite this file"
                    )
                    .to_string()
                } else {
                    tr!("ModsConfig.xml сохранён", "ModsConfig.xml saved").to_string()
                };
                true
            }
            Err(e) => {
                self.status_message =
                    tr!("Ошибка сохранения ModsConfig.xml: {}", "Could not save ModsConfig.xml: {}", e);
                false
            }
        }
    }

    /// Mouse button released while dragging mods: move, enable or disable them,
    /// depending on where they were dropped. Dropped anywhere else: nothing changes.
    pub fn finish_mod_drag(&mut self) {
        let Some(drag) = self.mod_drag.take() else {
            return;
        };
        match self.drop_target.take() {
            Some(DropTarget::Active(to)) => move_mods(&mut self.active_mods, &drag.ids, to),
            Some(DropTarget::Available) if drag.from_active => {
                remove_mods(&mut self.active_mods, &drag.ids);
            }
            _ => {}
        }
    }

    /// The game's ModLists folder (next to the Config folder with ModsConfig.xml).
    pub fn modlists_folder(&self) -> Option<PathBuf> {
        self.mods_config_path.as_deref().and_then(crate::modlists::modlists_dir)
    }

    pub fn refresh_modlists(&mut self) {
        self.modlists = self
            .modlists_folder()
            .map(|dir| crate::modlists::list_modlists(&dir))
            .unwrap_or_default();
        if let Some(cur) = &self.current_modlist {
            if !self.modlists.iter().any(|l| &l.name == cur) {
                self.current_modlist = None;
            }
        }
        if let Some(sel) = &self.selected_modlist {
            if !self.modlists.iter().any(|l| &l.name == sel) {
                self.selected_modlist = None;
            }
        }
    }

    /// How the set differs from what the game loads now (ModsConfig.xml on disk).
    pub fn modlist_diff(&self, list: &crate::modlists::ModList) -> crate::mods_config::ListDiff {
        crate::mods_config::diff_lists(&self.saved_active_mods, &list.ids)
    }

    /// Mods of the set that are not on disk.
    pub fn modlist_missing(&self, list: &crate::modlists::ModList) -> usize {
        list.ids.iter().filter(|id| self.find_mod(id).is_none()).count()
    }

    /// Workshop IDs of the set's mods that are not on disk (local mods and DLC have none).
    pub fn modlist_missing_workshop_ids(&self, list: &crate::modlists::ModList) -> Vec<String> {
        let mut ids: Vec<String> = list
            .ids
            .iter()
            .filter(|id| self.find_mod(id).is_none())
            .filter_map(|id| list.steam_id(id).map(str::to_string))
            .collect();
        ids.dedup();
        ids
    }

    /// F6: subscribes to every missing mod of the set that has a Workshop ID, in one
    /// Steam session in the background. Steam downloads them; then the user rescans.
    pub fn subscribe_modlist_missing(&mut self, name: &str) {
        if self.steam_rx.is_some() {
            return;
        }
        let Some(list) = self.modlists.iter().find(|l| l.name == name) else { return };
        let ids = self.modlist_missing_workshop_ids(list);
        self.subscribe_workshop_ids(ids);
    }

    /// Subscribes to Workshop items in one Steam session in the background.
    fn subscribe_workshop_ids(&mut self, ids: Vec<String>) {
        if self.steam_rx.is_some() || ids.is_empty() {
            return;
        }
        let t = self.i18n;
        let Some(dll) = self.steam_paths.steam_dll_path.clone() else {
            self.status_message = t.get(TranslationKey::SetsNoSteamDll).to_string();
            return;
        };
        self.status_message = fmt(t.get(TranslationKey::SetsSubscribing), &[&ids.len().to_string()]);
        self.steam_rx = Some(self.spawn_task(move || {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            match crate::steam::subscribe_workshop_items_batch_native(&dll, &refs) {
                Ok(n) => fmt(t.get(TranslationKey::SetsSubscribed), &[&n.to_string()]),
                Err(e) => fmt(t.get(TranslationKey::SetsSubscribeError), &[&e]),
            }
        }));
    }

    /// F5: adds a shared `.rml` or `ModsConfig.xml` as a new set (a free name is picked)
    /// and selects it. Names and Workshop IDs come from the file, else from installed mods.
    pub fn import_modlist(&mut self, file: &Path) {
        let t = self.i18n;
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let file_name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Some(list) = crate::modlists::read_shared(file) else {
            self.status_message = fmt(t.get(TranslationKey::SetsImportBad), &[&file_name]);
            return;
        };
        let base = crate::modlists::check_name(&list.name).unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let name = crate::modlists::unique_name(&dir, &base);
        let entries: Vec<crate::modlists::ModListEntry> = list
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let m = self.find_mod(id);
                crate::modlists::ModListEntry {
                    id: id.to_lowercase(),
                    name: m
                        .map(|m| m.name.clone())
                        .or_else(|| list.names.get(i).cloned())
                        .unwrap_or_else(|| id.clone()),
                    steam_id: list
                        .steam_id(id)
                        .map(str::to_string)
                        .or_else(|| m.and_then(|m| m.steam_item_id.clone())),
                }
            })
            .collect();
        let version = list.game_version.clone().unwrap_or_else(|| self.game_version_for_sets());
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &name), &version, &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.selected_modlist = Some(name.clone());
                self.status_message = fmt(t.get(TranslationKey::SetsImported), &[&name, &entries.len().to_string()]);
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// F5: copies the set's `.rml` file to where the user chose.
    pub fn export_modlist(&mut self, name: &str, target: &Path) {
        let t = self.i18n;
        let Some(list) = self.modlists.iter().find(|l| l.name == name) else { return };
        self.status_message = match std::fs::copy(&list.path, target) {
            Ok(_) => fmt(t.get(TranslationKey::SetsExported), &[name, &target.display().to_string()]),
            Err(e) => fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        };
    }

    /// F5: the set as text with Workshop links, for a chat or forum.
    pub fn modlist_share_text(&self, list: &crate::modlists::ModList) -> String {
        let mods: Vec<(String, Option<String>)> = list
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let m = self.find_mod(id);
                let name = m
                    .map(|m| m.name.clone())
                    .or_else(|| list.names.get(i).cloned())
                    .unwrap_or_else(|| id.clone());
                let steam = list
                    .steam_id(id)
                    .map(str::to_string)
                    .or_else(|| m.and_then(|m| m.steam_item_id.clone()));
                (name, steam)
            })
            .collect();
        crate::modlists::share_text(&list.name, &mods)
    }

    /// The game's Saves folder (next to the Config folder with ModsConfig.xml).
    pub fn saves_folder(&self) -> Option<PathBuf> {
        self.mods_config_path.as_deref().and_then(crate::saves::saves_dir)
    }

    /// Reads the save headers in the background (the first read of big files can be slow).
    pub fn refresh_saves(&mut self) {
        if self.saves_rx.is_some() {
            return;
        }
        let Some(dir) = self.saves_folder() else { return };
        self.saves_loaded_at = Some(std::time::Instant::now());
        self.saves_rx = Some(self.spawn_task(move || crate::saves::list_saves(&dir)));
    }

    /// Reads the saves when the Saves tab is shown, and again every 30 s while it is
    /// open, so new saves and autosaves appear by themselves.
    pub fn ensure_saves_loaded(&mut self) {
        const REREAD: std::time::Duration = std::time::Duration::from_secs(30);
        if self.saves_loaded_at.is_none_or(|t| t.elapsed() > REREAD) {
            self.refresh_saves();
        }
    }

    pub fn saves_loading(&self) -> bool {
        self.saves_rx.is_some()
    }

    /// The save's mods compared with the current load order.
    pub fn save_compare(&self, save: &crate::saves::SaveInfo) -> crate::saves::SaveCompare {
        crate::saves::compare(&save.ids, &self.active_mods, |id| self.find_mod(id).is_some())
    }

    /// The save was made on another major game version than the installed one.
    pub fn save_other_version(&self, save: &crate::saves::SaveInfo) -> bool {
        let installed = self.steam_paths.game_version.as_deref().and_then(crate::game_install::major_minor);
        let saved = save.game_version.as_deref().and_then(crate::game_install::major_minor);
        matches!((installed, saved), (Some(a), Some(b)) if a != b)
    }

    /// "Enable as in the save": the save's mods in its order, as unsaved changes
    /// (ModsConfig.xml changes only on "Save Order"; mods not on disk are kept).
    pub fn load_save_mods(&mut self, name: &str) {
        let Some(save) = self.saves.iter().find(|s| s.name == name) else { return };
        let mut ids = Vec::with_capacity(save.ids.len());
        for id in &save.ids {
            add_active_id(&mut ids, id);
        }
        self.active_mods = ids;
        self.current_modlist = None;
        self.validate_deps();
        self.status_message = fmt(self.i18n.get(TranslationKey::SavesLoaded), &[name]);
    }

    /// Keeps the save's mod list as a modpack named after the save.
    pub fn save_to_modlist(&mut self, name: &str) {
        let t = self.i18n;
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let Some(save) = self.saves.iter().find(|s| s.name == name) else { return };
        let entries: Vec<crate::modlists::ModListEntry> = save
            .ids
            .iter()
            .map(|id| {
                let m = self.find_mod(id);
                crate::modlists::ModListEntry {
                    id: id.to_lowercase(),
                    name: m
                        .map(|m| m.name.clone())
                        .or_else(|| save.name_of(id).map(str::to_string))
                        .unwrap_or_else(|| id.clone()),
                    steam_id: save
                        .steam_id(id)
                        .map(str::to_string)
                        .or_else(|| m.and_then(|m| m.steam_item_id.clone())),
                }
            })
            .collect();
        let base = crate::modlists::check_name(name).unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let set_name = crate::modlists::unique_name(&dir, &base);
        let version = save.game_version.clone().unwrap_or_else(|| self.game_version_for_sets());
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &set_name), &version, &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.selected_modlist = Some(set_name.clone());
                self.status_message = fmt(t.get(TranslationKey::SavesToSetDone), &[&set_name]);
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// Subscribes to the save's mods that are not installed and have a Workshop ID.
    pub fn subscribe_save_missing(&mut self, name: &str) {
        let Some(save) = self.saves.iter().find(|s| s.name == name) else { return };
        let ids = self.save_missing_workshop_ids(save);
        self.subscribe_workshop_ids(ids);
    }

    pub fn save_missing_workshop_ids(&self, save: &crate::saves::SaveInfo) -> Vec<String> {
        let mut ids: Vec<String> = save
            .ids
            .iter()
            .filter(|id| self.find_mod(id).is_none())
            .filter_map(|id| save.steam_id(id).map(str::to_string))
            .collect();
        ids.dedup();
        ids
    }

    // ----- Broken-mod search (F8) -----

    /// Opens the search window: the running search, or the setup for a new one.
    pub fn open_bisect(&mut self) {
        self.bisect_open = true;
        self.bisect_note = None;
        if self.bisect.is_none() && self.bisect_keep.is_none() {
            self.bisect_keep = Some(self.bisect_default_keep());
        }
    }

    /// Never turned off by default: Core and DLC, Harmony / Prepatcher, frameworks.
    fn bisect_default_keep(&self) -> std::collections::HashSet<String> {
        self.saved_active_mods
            .iter()
            .filter(|id| {
                let lower = id.to_lowercase();
                lower == "ludeon.rimworld"
                    || lower.starts_with("ludeon.rimworld.")
                    || lower == "brrainz.harmony"
                    || lower == "zetrith.prepatcher"
                    || self.find_mod(id).is_some_and(|m| {
                        self.is_official_mod(m) || m.categories.contains(&ModCategory::Framework)
                    })
            })
            .map(|id| id.to_lowercase())
            .collect()
    }

    /// What each mod of `ids` needs among `ids` (alternatives and Prepatcher-for-Harmony count).
    pub fn bisect_requirements(&self, ids: &[String]) -> std::collections::BTreeMap<String, Vec<String>> {
        let present: std::collections::HashSet<String> = ids.iter().map(|i| i.to_lowercase()).collect();
        let mut out = std::collections::BTreeMap::new();
        for id in ids {
            let Some(m) = self.find_mod(id) else { continue };
            let mut needs: Vec<String> = Vec::new();
            for dep in &m.dependencies {
                let dep = dep.trim().to_lowercase();
                let declared = m.dependency_alternatives.get(&dep).cloned().unwrap_or_default();
                let choice = std::iter::once(dep.clone())
                    .chain(declared.into_iter().map(|a| a.to_lowercase()))
                    .chain(crate::dependencies::dependency_substitutes(&dep).iter().map(|s| s.to_string()))
                    .find(|candidate| present.contains(candidate));
                if let Some(c) = choice {
                    needs.push(c);
                }
            }
            if !needs.is_empty() {
                out.insert(id.to_lowercase(), needs);
            }
        }
        out
    }

    fn persist_bisect(&self) {
        let path = bisect_session_path();
        match &self.bisect {
            Some(search) => {
                if let Ok(text) = serde_json::to_string(search) {
                    let _ = std::fs::write(path, text);
                }
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Why a step can't run now: the game is open, or the list has unsaved changes.
    fn bisect_blocker(&self, check_unsaved: bool) -> Option<String> {
        let t = self.i18n;
        if self.mods_config_path.is_none() {
            return Some(t.get(TranslationKey::BisectNoConfig).to_string());
        }
        if check_unsaved && self.has_unsaved_changes() {
            return Some(t.get(TranslationKey::BisectUnsaved).to_string());
        }
        if crate::game_install::is_game_running() {
            return Some(t.get(TranslationKey::BisectCloseGame).to_string());
        }
        None
    }

    /// Starts a search from the load order the game uses now, keeping the ticked mods on.
    /// The current order is saved as a modpack first, so it can always be brought back.
    pub fn start_bisect(&mut self) {
        if let Some(why) = self.bisect_blocker(true) {
            self.bisect_note = Some(why);
            return;
        }
        let keep: Vec<String> = self.bisect_keep.clone().unwrap_or_default().into_iter().collect();
        let original = self.saved_active_mods.clone();
        let requires = self.bisect_requirements(&original);
        let search = crate::bisect::Bisect::new(&original, &keep, &requires);

        // "Before the search" modpack (overwritten each time).
        if let Some(dir) = self.modlists_folder() {
            let name = self.i18n.get(TranslationKey::BisectBackupSetName);
            let entries = self.modlist_entries();
            let version = self.game_version_for_sets();
            if crate::modlists::write_rml(&crate::modlists::path_for(&dir, name), &version, &entries).is_ok() {
                self.refresh_modlists();
            }
        }
        self.bisect = Some(search);
        self.bisect_keep = None;
        self.bisect_note = None;
        self.persist_bisect();
    }

    /// Writes the current test's load order to ModsConfig.xml, and starts the game if asked.
    pub fn bisect_apply(&mut self, launch: bool) {
        if let Some(why) = self.bisect_blocker(false) {
            self.bisect_note = Some(why);
            return;
        }
        let (Some(search), Some(config)) = (self.bisect.as_ref(), self.mods_config_path.clone()) else { return };
        let enabled = search.enabled();
        // One backup at the first launch; later steps don't push real backups out.
        let first = search.history.is_empty() && !search.applied;
        let written = if first {
            save_active_mods(&config, &enabled)
        } else {
            crate::mods_config::write_active_mods(&config, &enabled)
        };
        if let Err(e) = written {
            self.bisect_note = Some(e);
            return;
        }
        self.active_mods = enabled.clone();
        self.remember_saved_order(&enabled);
        self.validate_deps();
        if let Some(search) = self.bisect.as_mut() {
            search.applied = true;
        }
        self.bisect_note = None;
        self.persist_bisect();
        if launch {
            self.launch_game_now();
        }
    }

    pub fn bisect_answer(&mut self, problem: bool) {
        if let Some(search) = self.bisect.as_mut() {
            search.answer(problem);
            self.bisect_note = None;
            self.persist_bisect();
        }
    }

    pub fn bisect_step_back(&mut self) {
        if let Some(search) = self.bisect.as_mut() {
            search.step_back();
            self.bisect_note = None;
            self.persist_bisect();
        }
    }

    /// Ends the search: brings the original load order back, without the found mods and
    /// what needs them if `turn_off_found`.
    pub fn bisect_finish(&mut self, turn_off_found: bool) {
        if let Some(why) = self.bisect_blocker(false) {
            self.bisect_note = Some(why);
            return;
        }
        let Some(search) = self.bisect.as_ref() else { return };
        let Some(config) = self.mods_config_path.clone() else { return };
        let t = self.i18n;
        let mut order = search.original.clone();
        let mut message = t.get(TranslationKey::BisectRestored).to_string();
        if turn_off_found {
            if let Some(crate::bisect::Outcome::Found(found)) = search.outcome() {
                let mut off: Vec<String> = found.clone();
                off.extend(search.dependents_of(found));
                order.retain(|id| !off.contains(id));
                message = fmt(t.get(TranslationKey::BisectTurnedOff), &[&off.len().to_string()]);
            }
        }
        if let Err(e) = save_active_mods(&config, &order) {
            self.bisect_note = Some(e);
            return;
        }
        self.active_mods = order.clone();
        self.remember_saved_order(&order);
        self.validate_deps();
        self.bisect = None;
        self.bisect_open = false;
        self.bisect_note = None;
        self.persist_bisect();
        self.status_message = message;
    }

    /// The setup screen is closed without starting.
    pub fn close_bisect_window(&mut self) {
        self.bisect_open = false;
        self.bisect_keep = None;
        self.bisect_note = None;
    }

    /// A mod's display name: the installed mod's name, else its ID.
    pub fn mod_display_name(&self, id: &str) -> String {
        self.find_mod(id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string())
    }

    /// Whether a Steam request (subscribe, check) is running now.
    pub fn steam_busy(&self) -> bool {
        self.steam_rx.is_some()
    }

    /// "Enable": writes the set to ModsConfig.xml (a backup of the old order is made),
    /// and starts the game too if asked.
    pub fn apply_modlist(&mut self, name: &str, launch: bool) {
        self.load_modlist(name);
        if self.save_config() {
            self.status_message = fmt(self.i18n.get(TranslationKey::SetsApplied), &[name]);
            if launch {
                self.launch_game_now();
            }
        }
    }

    /// Copies a set to a free "Name (copy)" and selects the copy.
    pub fn duplicate_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        let copy = crate::modlists::copy_name(&dir, name, self.i18n.get(TranslationKey::SetsCopyWord));
        match std::fs::copy(crate::modlists::path_for(&dir, name), crate::modlists::path_for(&dir, &copy)) {
            Ok(_) => {
                self.refresh_modlists();
                self.selected_modlist = Some(copy.clone());
                self.status_message = fmt(self.i18n.get(TranslationKey::SetsDuplicated), &[name, &copy]);
            }
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// The current load order with names and Workshop IDs, as the game writes it.
    fn modlist_entries(&self) -> Vec<crate::modlists::ModListEntry> {
        self.active_mods
            .iter()
            .map(|id| {
                let m = self.find_mod(id);
                // A mod not on disk keeps the Workshop ID a set remembers for it,
                // so "subscribe to missing" still works for the new file.
                let steam_id = m
                    .and_then(|m| m.steam_item_id.clone())
                    .or_else(|| self.modlists.iter().find_map(|l| l.steam_id(id).map(str::to_string)));
                crate::modlists::ModListEntry {
                    id: id.to_lowercase(),
                    name: m.map(|m| m.name.clone()).unwrap_or_else(|| id.clone()),
                    steam_id,
                }
            })
            .collect()
    }

    fn game_version_for_sets(&self) -> String {
        self.steam_paths.game_version.clone().unwrap_or_else(|| "1.6".into())
    }

    fn name_problem_text(&self, problem: crate::modlists::NameProblem) -> String {
        use crate::modlists::NameProblem;
        self.i18n
            .get(match problem {
                NameProblem::Empty => TranslationKey::SetsNameEmpty,
                NameProblem::BadCharacters => TranslationKey::SetsNameBadChars,
                NameProblem::TooLong => TranslationKey::SetsNameTooLong,
            })
            .to_string()
    }

    /// Loads a set into the list as unsaved changes: ModsConfig.xml changes only on "Save Order".
    pub fn load_modlist(&mut self, name: &str) {
        let Some(list) = self.modlists.iter().find(|l| l.name == name).cloned() else { return };
        let mut ids = Vec::with_capacity(list.ids.len());
        for id in &list.ids {
            add_active_id(&mut ids, id);
        }
        self.active_mods = ids;
        self.validate_deps();
        self.current_modlist = Some(list.name.clone());
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsLoaded), &[&list.name]);
    }

    /// Saves the current load order as a new set. The error text is shown in the dialog.
    pub fn save_modlist_as(&mut self, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self.modlists_folder().ok_or_else(|| self.i18n.get(TranslationKey::SetsNoFolder).to_string())?;
        let path = crate::modlists::path_for(&dir, &name);
        if path.exists() {
            return Err(self.i18n.get(TranslationKey::SetsNameExists).to_string());
        }
        self.write_modlist(&name, &path)
    }

    /// Overwrites an existing set with the current load order.
    pub fn overwrite_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        let path = crate::modlists::path_for(&dir, name);
        if let Err(e) = self.write_modlist(name, &path) {
            self.status_message = e;
        }
    }

    fn write_modlist(&mut self, name: &str, path: &std::path::Path) -> Result<(), String> {
        let entries = self.modlist_entries();
        crate::modlists::write_rml(path, &self.game_version_for_sets(), &entries)
            .map_err(|e| fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]))?;
        self.refresh_modlists();
        self.current_modlist = Some(name.to_string());
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsSaved), &[name, &entries.len().to_string()]);
        Ok(())
    }

    pub fn rename_modlist(&mut self, from: &str, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self.modlists_folder().ok_or_else(|| self.i18n.get(TranslationKey::SetsNoFolder).to_string())?;
        let target = crate::modlists::path_for(&dir, &name);
        // Changing only the letter case is fine; another existing set is not.
        if target.exists() && !name.eq_ignore_ascii_case(from) {
            return Err(self.i18n.get(TranslationKey::SetsNameExists).to_string());
        }
        std::fs::rename(crate::modlists::path_for(&dir, from), &target)
            .map_err(|e| fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]))?;
        if self.current_modlist.as_deref() == Some(from) {
            self.current_modlist = Some(name.clone());
        }
        self.refresh_modlists();
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsRenamed), &[from, &name]);
        Ok(())
    }

    /// Moves a set file to the Recycle Bin. Mods and ModsConfig.xml are not touched.
    pub fn delete_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        match trash::delete(crate::modlists::path_for(&dir, name)) {
            Ok(()) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsDeleted), &[name]),
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
        self.refresh_modlists();
    }

    /// Opens the "History" window with the ModsConfig.xml backups.
    pub fn open_history(&mut self) {
        self.history = Some(crate::mods_config::list_backups(&crate::mods_config::backups_dir()));
    }

    /// Loads a backup into the list as unsaved changes: ModsConfig.xml changes only on "Save".
    pub fn restore_backup(&mut self, backup: &crate::mods_config::ConfigBackup) {
        self.active_mods = backup.mods.clone();
        self.validate_deps();
        self.history = None;
        self.status_message = tr!(
            "Загружен порядок от {} — нажмите «Сохранить порядок», чтобы его увидела игра",
            "Loaded the order from {} — press \"Save Order\" so the game sees it",
            crate::ui::components::local_date_time(backup.saved_at)
        );
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
    /// Runs the deletion the user confirmed in the modal window — in the background: the
    /// Steam request and moving big folders to the Recycle Bin take seconds (R4).
    pub fn confirm_pending_delete(&mut self) {
        let Some(pending) = self.pending_delete.take() else {
            return;
        };
        if self.delete_rx.is_some() {
            return;
        }
        let targets: Vec<ModInfo> = pending
            .ids
            .iter()
            .filter_map(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let steam_paths = self.steam_paths.clone();
        self.status_message = tr!("Удаление...", "Deleting...").to_string();

        self.delete_rx = Some(self.spawn_task(move || match pending.kind {
            DeleteKind::Translations => {
                let refs: Vec<&ModInfo> = targets.iter().collect();
                let res = delete_translation_mods(&refs, &steam_paths);
                let mut msg = tr!(
                    "Перемещено в Корзину: {} | Отписано в Steam: {}",
                    "Moved to Recycle Bin: {} | Unsubscribed in Steam: {}",
                    res.deleted_count,
                    res.unsubscribed_count
                );
                if let Some(err) = res.errors.first() {
                    msg.push_str(&tr!(" | Ошибок: {} ({})", " | Errors: {} ({})", res.errors.len(), err));
                }
                msg
            }
            DeleteKind::Single => {
                let m = &targets[0];
                match unsubscribe_and_delete_single(m, &steam_paths) {
                    Ok(()) => tr!("Мод {} перемещён в Корзину", "Mod {} moved to the Recycle Bin", m.name),
                    Err(e) => tr!("Мод {} не удалён: {}", "Mod {} not deleted: {}", m.name, e),
                }
            }
        }));
    }

    pub fn is_deleting(&self) -> bool {
        self.delete_rx.is_some()
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
        self.show_batch_results = false;
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

    pub fn is_batch_searching(&self) -> bool {
        self.batch_rx.is_some()
    }

    /// "Find translations for the whole modpack" (T1): searches the Workshop for a translation
    /// of every active mod into the selected language, in the background (3 requests at a time).
    /// Skipped: Core/DLC, translation mods, mods that already ship the language, mods that
    /// already have a translation installed. Shown: only items whose title names the mod.
    pub fn start_batch_translation_search(&mut self) {
        if self.batch_rx.is_some() {
            return;
        }
        let lang = self.trans_target_lang;
        let installed_steam: std::collections::HashSet<String> =
            self.all_mods.iter().filter_map(|m| m.steam_item_id.clone()).collect();
        let translated: std::collections::HashSet<String> = self
            .all_mods
            .iter()
            .filter(|m| m.is_translation_mod && lang.matches_mod_info(m))
            .filter_map(|m| m.parent_package_id.as_deref().map(str::to_lowercase))
            .collect();

        let mut skipped = 0;
        let mut seen = std::collections::HashSet::new();
        let mut candidates: Vec<(String, Option<String>)> = Vec::new();
        for id in &self.active_mods {
            let Some(m) = self.find_mod(id) else { continue };
            if self.is_official_mod(m) || m.is_translation_mod || !seen.insert(m.package_id.to_lowercase()) {
                continue;
            }
            if lang.is_built_into(m) || translated.contains(&m.package_id.to_lowercase()) {
                skipped += 1;
                continue;
            }
            candidates.push((m.name.clone(), m.steam_item_id.clone()));
        }

        self.batch_results.clear();
        self.batch_checked.clear();
        self.batch_lang = lang;
        self.batch_skipped = skipped;
        self.batch_progress = (0, candidates.len());
        self.show_batch_results = true;
        if candidates.is_empty() {
            self.status_message = tr!(
                "Все активные моды уже переведены на {} — искать нечего",
                "Every active mod already has {} — nothing to search",
                lang.label()
            );
            return;
        }
        self.status_message = tr!(
            "Поиск переводов на {} для {} модов...",
            "Searching {} translations for {} mods...",
            lang.label(),
            candidates.len()
        );

        let stop = Arc::new(AtomicBool::new(false));
        self.batch_stop = stop.clone();
        let (tx, rx) = channel();
        self.batch_rx = Some(rx);
        let ctx = self.repaint_ctx.clone();
        std::thread::spawn(move || {
            use std::sync::atomic::AtomicUsize;
            let total = candidates.len();
            let next = AtomicUsize::new(0);
            let done = AtomicUsize::new(0);
            let errors = AtomicUsize::new(0);
            std::thread::scope(|s| {
                // A few requests at a time: fast enough, and gentle with Steam.
                for _ in 0..3 {
                    s.spawn(|| loop {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((name, steam_id)) = candidates.get(i) else { break };
                        let query = format!("{} {}", name, lang.search_suffix());
                        match search_steam_workshop(&query) {
                            Ok(raw) => {
                                let items = crate::matcher::translations_for_mod(
                                    raw,
                                    name,
                                    steam_id.as_deref(),
                                    lang,
                                    &installed_steam,
                                );
                                if !items.is_empty() {
                                    let _ = tx.send(BatchSearchMsg::Found(ModTranslations { mod_name: name.clone(), items }));
                                }
                            }
                            Err(_) => {
                                errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        let _ = tx.send(BatchSearchMsg::Progress { done: d, total });
                        if let Some(ctx) = &ctx {
                            ctx.request_repaint();
                        }
                    });
                }
            });
            let _ = tx.send(BatchSearchMsg::Done {
                searched: done.load(Ordering::Relaxed),
                errors: errors.load(Ordering::Relaxed),
                cancelled: stop.load(Ordering::Relaxed),
            });
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        });
    }

    pub fn stop_batch_translation_search(&mut self) {
        self.batch_stop.store(true, Ordering::Relaxed);
        self.status_message = tr!("Останавливаю поиск...", "Stopping the search...").to_string();
    }

    fn poll_batch_search(&mut self) {
        let Some(rx) = &self.batch_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BatchSearchMsg::Progress { done, total } => self.batch_progress = (done, total),
                BatchSearchMsg::Found(found) => {
                    // The best match of each mod is ticked; results stay sorted by mod name.
                    if let Some(best) = found.items.first() {
                        self.batch_checked.insert(best.id.clone());
                    }
                    let key = found.mod_name.to_lowercase();
                    let pos = self.batch_results.partition_point(|r| r.mod_name.to_lowercase() < key);
                    self.batch_results.insert(pos, found);
                }
                BatchSearchMsg::Done { searched, errors, cancelled } => finished = Some((searched, errors, cancelled)),
            }
        }
        if let Some((searched, errors, cancelled)) = finished {
            self.batch_rx = None;
            let mut msg = tr!(
                "Проверено модов: {} | с переводом на {}: {}",
                "Mods checked: {} | with a {} translation: {}",
                searched,
                self.batch_lang.label(),
                self.batch_results.len()
            );
            if cancelled {
                msg.push_str(tr!(" | остановлено", " | stopped"));
            }
            if errors > 0 {
                msg.push_str(&tr!(" | без ответа Steam: {}", " | no answer from Steam: {}", errors));
            }
            self.status_message = msg;
        }
    }

    /// Subscribes to the ticked translations in one Steam session (background).
    pub fn subscribe_checked_translations(&mut self) {
        if self.steam_rx.is_some() || self.batch_checked.is_empty() {
            return;
        }
        let Some(dll) = self.steam_paths.steam_dll_path.clone() else {
            self.status_message = tr!(
                "steam_api64.dll не найден — проверьте папку игры в Настройках",
                "steam_api64.dll not found — check the game folder in Settings"
            )
            .to_string();
            return;
        };
        let ids: Vec<String> = self.batch_checked.drain().collect();
        self.batch_subscribed.extend(ids.iter().cloned());
        self.status_message = tr!("Подписка на переводы: {}...", "Subscribing to translations: {}...", ids.len());
        self.steam_rx = Some(self.spawn_task(move || {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            match crate::steam::subscribe_workshop_items_batch_native(&dll, &refs) {
                Ok(n) => tr!(
                    "Steam принял подписок: {} — переводы скачаются в клиенте Steam, потом нажмите «Пересканировать»",
                    "Steam accepted {} subscriptions — the Steam client downloads them, then press \"Rescan Mods\"",
                    n
                ),
                Err(e) => tr!("Ошибка подписки: {}", "Subscribe failed: {}", e),
            }
        }));
    }

    /// Free-text Workshop search — in the background (S5).
    pub fn search_workshop_custom(&mut self) {
        if self.workshop_rx.is_some() || self.workshop_search_query.trim().is_empty() {
            return;
        }
        self.show_batch_results = false;
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

    /// Mod folders for the chosen scope (All / Active / Selected), without Core/DLC,
    /// the blacklist and the user's ignore list. Used by both "Start" and "Scan" (R7).
    fn optimizer_target_dirs(&self) -> Vec<PathBuf> {
        let active: std::collections::HashSet<String> =
            self.active_mods.iter().map(|s| s.to_lowercase()).collect();
        let selected = self.selected_mod_id.as_deref().map(str::to_lowercase);
        let mods: Vec<ModInfo> = self
            .optimizable_mods()
            .into_iter()
            .filter(|m| {
                let id = m.package_id.to_lowercase();
                match self.opt_scope {
                    OptimizeScope::AllInstalled => true,
                    OptimizeScope::ActiveOnly => active.contains(&id),
                    OptimizeScope::SelectedOnly => selected.as_deref() == Some(id.as_str()),
                }
            })
            .collect();
        dirs_from_mods(&mods)
    }

    pub fn start_texture_optimization(&mut self) {
        let dirs = self.optimizer_target_dirs();

        if dirs.is_empty() {
            self.status_message =
                tr!("Нет выбранных модов для оптимизации текстур", "No mods selected for texture optimization")
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
            // The side the installed game needs: flipped on 1.7, as the PNG on 1.6.
            ConvertOptions { flip_vertical: crate::optimizer::engine_wants_flipped(self.steam_paths.engine) },
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

    /// RimWorld 1.7: moves TextureCache and ShaderCache to the Recycle Bin (game rebuilds them).
    /// In the background: the cache can be gigabytes, moving it to the Recycle Bin takes a while (R4).
    pub fn clear_game_caches(&mut self) {
        if self.cache_rx.is_some() {
            return;
        }
        self.status_message = tr!("Очистка кэша игры...", "Clearing the game cache...").to_string();
        self.cache_rx = Some(self.spawn_task(|| {
            let size_mb = crate::game_install::game_cache_size() as f64 / (1024.0 * 1024.0);
            match crate::game_install::clear_game_caches() {
                Ok(cleared) if cleared.is_empty() => {
                    tr!("Кэш игры уже пуст", "The game cache is already empty").to_string()
                }
                Ok(cleared) => tr!(
                    "Кэш игры перемещён в Корзину ({}, {:.0} МБ). Игра пересоберёт его при следующем запуске.",
                    "Game cache moved to the Recycle Bin ({}, {:.0} MB). The game rebuilds it on the next start.",
                    cleared.join(", "),
                    size_mb
                ),
                Err(e) => tr!("Не удалось очистить кэш: {}", "Could not clear the cache: {}", e),
            }
        }));
    }

    /// "Flip textures": compares every optimizer DDS that may be the wrong way up for the
    /// installed game with its source and re-encodes the ones that are. Background, cancellable.
    pub fn fix_texture_orientation(&mut self) {
        if self.optimizer_busy() {
            return;
        }
        let want_flipped = crate::optimizer::engine_wants_flipped(self.steam_paths.engine);
        let (tx, rx) = channel();
        self.opt_orient_rx = Some(rx);
        self.opt_is_running = true;
        self.opt_stop_flag.store(false, Ordering::Relaxed);
        self.opt_done = 0;
        self.opt_total = 0;
        self.opt_errors = 0;
        self.opt_current_file.clear();
        crate::optimizer::fix_orientation_async(want_flipped, self.opt_threads, tx, self.opt_stop_flag.clone());
        self.status_message = self.i18n.get(TranslationKey::OptFlipRunning).to_string();
    }

    fn poll_orientation_fix(&mut self) {
        let Some(rx) = &self.opt_orient_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                crate::optimizer::OrientationMsg::Progress { done, total, current_file } => {
                    self.opt_done = done;
                    self.opt_total = total;
                    self.opt_current_file = current_file;
                }
                crate::optimizer::OrientationMsg::Done(summary) => finished = Some(summary),
            }
        }
        if let Some(s) = finished {
            self.opt_orient_rx = None;
            self.opt_is_running = false;
            self.opt_errors = s.errors;
            let key = if s.cancelled { TranslationKey::OptFlipStopped } else { TranslationKey::OptFlipDone };
            self.status_message = fmt(
                self.i18n.get(key),
                &[&s.fixed.to_string(), &s.already_ok.to_string(), &s.errors.to_string()],
            );
            self.rescan_mods_keep_status();
        }
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
        let dirs = self.optimizer_target_dirs();
        let (tx, rx) = channel();
        texture_summary_async(dirs, self.opt_force, crate::optimizer::engine_wants_flipped(self.steam_paths.engine), tx);
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
        self.opt_is_running
            || self.opt_scan_rx.is_some()
            || self.opt_rollback_rx.is_some()
            || self.legacy_rx.is_some()
            || matches!(self.pending_question, Some(PendingQuestion::LegacyDds(_)))
    }

    fn poll_optimizer_tasks(&mut self) {
        self.poll_orientation_fix();
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
                // Count again what is left on disk (normally nothing).
                self.rescan_mods_keep_status();
                if !res.unlogged.is_empty() {
                    self.pending_question = Some(PendingQuestion::LegacyDds(res.unlogged));
                }
            }
        }
    }

    /// Deletes the DDS files the user agreed to remove after a rollback (background).
    fn delete_unlogged_dds(&mut self, paths: Vec<PathBuf>) {
        self.status_message = tr!("Удаление DDS...", "Deleting DDS...").to_string();
        self.legacy_rx = Some(self.spawn_task(move || {
            let (removed, errors) = crate::optimizer::delete_files(&paths);
            tr!(
                "Удалено DDS не из журнала: {} | ошибок: {}",
                "Unlogged DDS deleted: {} | errors: {}",
                removed,
                errors
            )
        }));
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

    /// "Play": with an unsaved load order, asks first (save and play / play as is / cancel).
    pub fn launch_game(&mut self) {
        if self.has_unsaved_changes() {
            self.pending_question = Some(PendingQuestion::LaunchUnsaved);
        } else {
            self.launch_game_now();
        }
    }

    fn launch_game_now(&mut self) {
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
        // App bar (64 px): logo + name + version, text tabs, "Play" on the right.
        let bar_frame = egui::Frame {
            inner_margin: egui::Margin::symmetric(16.0, 12.0),
            fill: THEME.bg_surface,
            ..Default::default()
        };
        egui::TopBottomPanel::top("top_panel").frame(bar_frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(40.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                let t = self.i18n;
                let tabs = [
                    (AppTab::Mods, t.get(TranslationKey::TabMods)),
                    (AppTab::Sets, t.get(TranslationKey::TabSets)),
                    (AppTab::Saves, t.get(TranslationKey::TabSaves)),
                    (AppTab::Translations, t.get(TranslationKey::TabTranslations)),
                    (AppTab::Optimizer, t.get(TranslationKey::TabOptimizer)),
                    (AppTab::Analyzer, t.get(TranslationKey::TabAnalyzer)),
                    (AppTab::Settings, t.get(TranslationKey::TabSettings)),
                ];
                let update_text = self.update_available.as_ref().map(|l| tr!("Вышла {} →", "{} is out →", l.tag));
                let bisect_text = (self.bisect.is_some() && !self.bisect_open).then(|| {
                    let step = self.bisect.as_ref().map_or(1, |s| s.stage.answers + 1);
                    fmt(t.get(TranslationKey::BisectHeaderTag), &[&step.to_string()])
                });

                // What fits in a narrow window: first the title and version go (the logo
                // keeps them as a tooltip), then the tabs get tighter.
                let title = "RimWorld Mod Manager";
                let version = concat!("v", env!("CARGO_PKG_VERSION"));
                let text_w = |s: &str, size: f32| ui.painter().layout_no_wrap(s.to_string(), egui::FontId::proportional(size), egui::Color32::WHITE).size().x;
                let tags_w: f32 = [&update_text, &bisect_text]
                    .iter()
                    .filter_map(|s| s.as_deref())
                    .map(|s| crate::ui::components::badge_width(ui, s) + 8.0)
                    .sum();
                let tabs_w = |pad: f32| tabs.iter().map(|(_, l)| text_w(l, 13.0) + pad + 4.0).sum::<f32>();
                let play_w = crate::ui::components::styled_button_width(ui, true, t.get(TranslationKey::AppPlayBtn));
                let fixed = 32.0 + 8.0 + 16.0 + play_w + 8.0 + tags_w;
                let title_w = text_w(title, 14.0) + text_w(version, 12.0) + 16.0;
                let avail = ui.available_width();
                let show_title = fixed + title_w + tabs_w(24.0) <= avail;
                let tab_pad = if show_title || fixed + tabs_w(24.0) <= avail { 24.0 } else { 14.0 };

                // Logo in a lavender-tinted square
                let (logo_rect, logo) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                ui.painter().rect_filled(logo_rect, Rounding::same(6.0), crate::ui::theme::tint(THEME.accent_cyan));
                egui::Image::new(ICON_LOGO).paint_at(ui, logo_rect.shrink(4.0));
                if show_title {
                    ui.label(egui::RichText::new(title).color(THEME.accent_cyan).size(14.0).strong());
                    ui.label(egui::RichText::new(version).color(THEME.text_muted).size(12.0));
                } else {
                    logo.on_hover_text(format!("{title} {version}"));
                }
                if let (Some(latest), Some(text)) = (self.update_available.clone(), update_text.as_deref()) {
                    if crate::ui::components::tag(ui, text, THEME.accent_emerald)
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(tr!("Открыть страницу новой версии на GitHub", "Open the new version's page on GitHub"))
                        .clicked()
                    {
                        crate::ui::components::open_external(&latest.url);
                    }
                }
                if let Some(text) = bisect_text.as_deref() {
                    if crate::ui::components::tag(ui, text, THEME.accent_amber)
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.open_bisect();
                    }
                }

                ui.add_space(16.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tab_val, label) in tabs {
                    if crate::ui::components::nav_tab_padded(ui, label, self.current_tab == tab_val, tab_pad).clicked() {
                        self.current_tab = tab_val;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::ui::components::success_button_icon(ui, ICON_PLAY, t.get(TranslationKey::AppPlayBtn))
                        .clicked()
                    {
                        self.launch_game();
                    }
                });
            });
        });

        // Status bar (28 px): last status on the left, game version on the right.
        let status_frame = egui::Frame {
            inner_margin: egui::Margin::symmetric(16.0, 6.0),
            fill: THEME.bg_surface,
            ..Default::default()
        };
        egui::TopBottomPanel::bottom("bottom_status_bar").frame(status_frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.status_message).color(THEME.text_secondary).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let game = match self.steam_paths.game_version.as_deref() {
                        Some(v) => format!("RimWorld {v}"),
                        None => fmt(
                            self.i18n.get(TranslationKey::AppStatusBarStats),
                            &[&self.all_mods.len().to_string(), &self.active_mods.len().to_string()],
                        ),
                    };
                    ui.label(egui::RichText::new(game).color(THEME.text_muted).size(12.0));
                });
            });
        });

        // Main area: window background with 16 px side padding.
        let central_frame = egui::Frame {
            inner_margin: egui::Margin { left: 16.0, right: 16.0, top: 8.0, bottom: 12.0 },
            fill: THEME.bg_dark,
            ..Default::default()
        };
        egui::CentralPanel::default().frame(central_frame).show(ctx, |ui| match self.current_tab {
            AppTab::Mods => crate::ui::view_mods::render_mods_view(self, ui),
            AppTab::Sets => crate::ui::view_sets::render_sets_view(self, ui),
            AppTab::Saves => crate::ui::view_saves::render_saves_view(self, ui),
            AppTab::Translations => crate::ui::view_translations::render_translations_view(self, ui),
            // Long pages scroll as a whole, so nothing below the window edge is cut off.
            AppTab::Optimizer => {
                egui::ScrollArea::vertical()
                    .id_salt("optimizer_page_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| crate::ui::view_optimizer::render_optimizer_view(self, ui));
            }
            AppTab::Analyzer => crate::ui::view_analyzer::render_analyzer_view(self, ui),
            AppTab::Settings => {
                egui::ScrollArea::vertical()
                    .id_salt("settings_page_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| crate::ui::view_settings::render_settings_view(self, ui));
            }
        });

        // Closing the window with an unsaved load order: ask first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.has_unsaved_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_question = Some(PendingQuestion::CloseUnsaved);
        }

        self.render_delete_confirmation(ctx);
        self.render_question(ctx);
        self.render_history_window(ctx);
        self.render_modlist_dialog(ctx);
        crate::ui::view_bisect::render_bisect_window(self, ctx);
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
                    if crate::ui::components::secondary_button(ui, t.get(TranslationKey::BtnCancel)).clicked() {
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

    /// Modal window for `pending_question`: unsaved load order (Play / close) and DDS files
    /// that are not in the manager's log after a rollback.
    fn render_question(&mut self, ctx: &egui::Context) {
        let Some(question) = self.pending_question.clone() else {
            return;
        };
        #[derive(PartialEq)]
        enum Answer {
            None,
            Primary,
            Secondary,
            Cancel,
        }
        let mut answer = if ctx.input(|i| i.key_pressed(egui::Key::Escape)) { Answer::Cancel } else { Answer::None };

        let (title, body, primary, secondary): (&str, String, &str, &str) = match &question {
            PendingQuestion::LaunchUnsaved => (
                tr!("Порядок модов не сохранён", "Load order not saved"),
                tr!(
                    "Игра загрузит порядок из ModsConfig.xml, а ваши изменения ещё не сохранены.",
                    "The game loads the order from ModsConfig.xml, and your changes are not saved yet."
                )
                .to_string(),
                tr!("Сохранить и играть", "Save and play"),
                tr!("Играть без сохранения", "Play without saving"),
            ),
            PendingQuestion::CloseUnsaved => (
                tr!("Порядок модов не сохранён", "Load order not saved"),
                tr!(
                    "Если закрыть программу сейчас, изменения порядка модов пропадут.",
                    "If you close the app now, your load order changes are lost."
                )
                .to_string(),
                tr!("Сохранить и выйти", "Save and exit"),
                tr!("Выйти без сохранения", "Exit without saving"),
            ),
            PendingQuestion::LegacyDds(paths) => (
                tr!("DDS не из журнала программы", "DDS files not in the manager's log"),
                tr!(
                    "Откат удалил все DDS, которые создала эта программа. Найдено ещё {}: их сделала старая версия оптимизатора, RimPy или автор мода. У каждого рядом лежит PNG/JPG, поэтому моды будут работать и без них. Авторские DDS лучше оставить.",
                    "Rollback removed every DDS this manager created. {} more were found: made by an older optimizer version, RimPy or the mod author. Each has a PNG/JPG next to it, so mods work without them. Better keep the author's DDS.",
                    paths.len()
                ),
                tr!("Оставить", "Keep them"),
                tr!("Удалить их тоже", "Delete them too"),
            ),
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                ui.label(body);
                if let PendingQuestion::LegacyDds(paths) = &question {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .id_salt("unlogged_dds_list")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for p in paths {
                                ui.label(egui::RichText::new(p.display().to_string()).size(11.0).color(THEME.text_muted));
                            }
                        });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if crate::ui::components::primary_button(ui, primary).clicked() {
                        answer = Answer::Primary;
                    }
                    if crate::ui::components::secondary_button(ui, secondary).clicked() {
                        answer = Answer::Secondary;
                    }
                    if !matches!(question, PendingQuestion::LegacyDds(_))
                        && crate::ui::components::secondary_button(ui, self.i18n.get(TranslationKey::BtnCancel)).clicked()
                    {
                        answer = Answer::Cancel;
                    }
                });
            });

        if answer == Answer::None {
            return;
        }
        self.pending_question = None;
        match (question, answer) {
            (PendingQuestion::LaunchUnsaved, Answer::Primary) => {
                if self.save_config() {
                    self.launch_game_now();
                }
            }
            (PendingQuestion::LaunchUnsaved, Answer::Secondary) => self.launch_game_now(),
            (PendingQuestion::CloseUnsaved, Answer::Primary) => {
                if self.save_config() {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            (PendingQuestion::CloseUnsaved, Answer::Secondary) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (PendingQuestion::LegacyDds(paths), Answer::Secondary) => self.delete_unlogged_dds(paths),
            _ => {}
        }
    }

    /// Window for a mod set action: name field (save as new, rename) or delete confirmation.
    fn render_modlist_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.modlist_dialog.clone() else { return };
        let t = self.i18n;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut submit = false;
        let title = match &dialog {
            ModListDialog::SaveAs { .. } => t.get(TranslationKey::SetsDialogSaveTitle),
            ModListDialog::Rename { .. } => t.get(TranslationKey::SetsDialogRenameTitle),
            ModListDialog::Delete { .. } => t.get(TranslationKey::SetsDialogDeleteTitle),
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(400.0);
                match &mut dialog {
                    ModListDialog::SaveAs { name, error } | ModListDialog::Rename { name, error, .. } => {
                        ui.label(t.get(TranslationKey::SetsNameLabel));
                        let field = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                        if ui.memory(|m| m.focused().is_none()) {
                            field.request_focus();
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                        if let Some(e) = error {
                            ui.label(egui::RichText::new(e.as_str()).color(THEME.accent_rose).size(12.0));
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(t.get(TranslationKey::SetsHint)).color(THEME.text_muted).size(12.0),
                            )
                            .wrap(),
                        );
                    }
                    ModListDialog::Delete { name } => {
                        ui.add(egui::Label::new(fmt(t.get(TranslationKey::SetsDialogDeleteBody), &[name])).wrap());
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (text, style) = match &dialog {
                        ModListDialog::SaveAs { .. } => (t.get(TranslationKey::SetsBtnSave), crate::ui::components::ButtonStyle::Primary),
                        ModListDialog::Rename { .. } => (t.get(TranslationKey::SetsBtnRename), crate::ui::components::ButtonStyle::Primary),
                        ModListDialog::Delete { .. } => (t.get(TranslationKey::SetsBtnDelete), crate::ui::components::ButtonStyle::Danger),
                    };
                    if crate::ui::components::styled_button(ui, None, text, style).clicked() {
                        submit = true;
                    }
                    if crate::ui::components::secondary_button(ui, t.get(TranslationKey::BtnCancel)).clicked() {
                        close = true;
                    }
                });
            });

        if submit {
            let result = match &dialog {
                ModListDialog::SaveAs { name, .. } => self.save_modlist_as(name),
                ModListDialog::Rename { from, name, .. } => self.rename_modlist(from, name),
                ModListDialog::Delete { name } => {
                    self.delete_modlist(name);
                    Ok(())
                }
            };
            self.modlist_dialog = match (result, dialog) {
                (Ok(()), _) => None,
                (Err(e), ModListDialog::SaveAs { name, .. }) => Some(ModListDialog::SaveAs { name, error: Some(e) }),
                (Err(e), ModListDialog::Rename { from, name, .. }) => Some(ModListDialog::Rename { from, name, error: Some(e) }),
                (Err(_), other) => Some(other),
            };
        } else if close {
            self.modlist_dialog = None;
        } else {
            self.modlist_dialog = Some(dialog);
        }
    }

    /// "History" window: ModsConfig.xml backups with date, size and difference from the
    /// current list; "Restore" loads one as unsaved changes.
    fn render_history_window(&mut self, ctx: &egui::Context) {
        let Some(backups) = self.history.as_ref() else {
            return;
        };
        let mut restore: Option<usize> = None;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        egui::Window::new(tr!("История порядка модов", "Load order history"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(
                    egui::RichText::new(tr!(
                        "Перед каждым сохранением программа делает копию ModsConfig.xml (хранятся последние 30). Восстановленный список появится как несохранённый: файл изменится, только когда вы нажмёте «Сохранить порядок».",
                        "Before every save the manager copies ModsConfig.xml (the last 30 are kept). A restored list shows up as unsaved: the file changes only when you press \"Save Order\"."
                    ))
                    .color(THEME.text_secondary)
                    .size(12.0),
                );
                ui.add_space(8.0);

                if backups.is_empty() {
                    ui.label(
                        egui::RichText::new(tr!(
                            "Копий пока нет: они появятся после первого сохранения порядка в программе.",
                            "No copies yet: they appear after the first time you save the order here."
                        ))
                        .color(THEME.text_muted),
                    );
                }

                egui::ScrollArea::vertical()
                    .id_salt("history_list")
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (i, b) in backups.iter().enumerate() {
                            let diff = crate::mods_config::diff_lists(&self.active_mods, &b.mods);
                            egui::Frame {
                                inner_margin: egui::Margin::symmetric(10.0, 6.0),
                                outer_margin: egui::Margin::symmetric(0.0, 2.0),
                                rounding: Rounding::same(6.0),
                                fill: THEME.bg_card,
                                ..Default::default()
                            }
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    ui.label(
                                        egui::RichText::new(crate::ui::components::local_date_time(b.saved_at))
                                            .color(THEME.text_primary)
                                            .strong(),
                                    );
                                    ui.label(
                                        egui::RichText::new(if b.mods.len() == 1 {
                                            tr!("модов: 1", "1 mod").to_string()
                                        } else {
                                            tr!("модов: {}", "{} mods", b.mods.len())
                                        })
                                            .color(THEME.text_secondary)
                                            .size(12.0),
                                    );
                                    history_diff_label(ui, diff);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_enabled_ui(!diff.is_same(), |ui| {
                                            if crate::ui::components::small_button(
                                                ui,
                                                tr!("Восстановить", "Restore"),
                                                crate::ui::components::ButtonStyle::Secondary,
                                            )
                                            .clicked()
                                            {
                                                restore = Some(i);
                                            }
                                        });
                                    });
                                });
                            });
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if crate::ui::components::secondary_button_icon(ui, ICON_FOLDER, tr!("Открыть папку", "Open folder"))
                        .clicked()
                    {
                        let dir = crate::mods_config::backups_dir();
                        let _ = std::fs::create_dir_all(&dir);
                        crate::ui::components::open_external(dir);
                    }
                    if crate::ui::components::secondary_button(ui, tr!("Закрыть", "Close")).clicked() {
                        close = true;
                    }
                });
            });

        if let Some(i) = restore {
            if let Some(b) = self.history.as_ref().and_then(|list| list.get(i)).cloned() {
                self.restore_backup(&b);
            }
        } else if close {
            self.history = None;
        }
    }
}

/// "+3 −1 · different order" relative to the current list, or "same as now".
fn history_diff_label(ui: &mut egui::Ui, diff: crate::mods_config::ListDiff) {
    if diff.is_same() {
        ui.label(egui::RichText::new(tr!("как сейчас", "same as now")).color(THEME.text_muted).size(12.0));
        return;
    }
    let mut parts = Vec::new();
    if diff.added > 0 {
        parts.push(format!("+{}", diff.added));
    }
    if diff.removed > 0 {
        parts.push(format!("−{}", diff.removed));
    }
    if diff.reordered {
        parts.push(tr!("другой порядок", "different order").to_string());
    }
    ui.label(egui::RichText::new(parts.join(" · ")).color(THEME.accent_amber).size(12.0))
        .on_hover_text(tr!(
            "По сравнению с текущим списком: + моды, которые вернутся, − моды, которые уберутся",
            "Compared with the current list: + mods that come back, − mods that go away"
        ));
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
    fn unsaved_order_survives_rescan() {
        let disk = vec!["brrainz.harmony".to_string(), "ludeon.rimworld".to_string()];
        assert_eq!(active_after_rescan(true, false, &disk), None, "unsaved changes are kept");
        assert_eq!(active_after_rescan(false, false, &disk), Some(disk.clone()), "no changes: follow the file");
        assert_eq!(active_after_rescan(true, true, &disk), Some(disk), "other config picked: file wins");
    }

    #[test]
    fn add_active_id_ignores_case_and_stores_lowercase() {
        let mut active = vec!["ludeon.rimworld".to_string()];
        assert!(!add_active_id(&mut active, "Ludeon.RimWorld"));
        assert!(add_active_id(&mut active, "Author.CoolMod"));
        assert!(!add_active_id(&mut active, "author.coolmod"));
        assert_eq!(active, ["ludeon.rimworld", "author.coolmod"]);
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn move_one_mod_down_and_up() {
        let mut active = ids(&["a", "b", "c", "d"]);
        // "a" dropped between "c" and "d" (before index 3)
        move_mods(&mut active, &ids(&["a"]), 3);
        assert_eq!(active, ids(&["b", "c", "a", "d"]));
        // "d" dropped at the top
        move_mods(&mut active, &ids(&["D"]), 0);
        assert_eq!(active, ids(&["d", "b", "c", "a"]));
        // dropped onto its own place: nothing changes
        move_mods(&mut active, &ids(&["b"]), 1);
        assert_eq!(active, ids(&["d", "b", "c", "a"]));
        move_mods(&mut active, &ids(&["b"]), 2);
        assert_eq!(active, ids(&["d", "b", "c", "a"]));
    }

    #[test]
    fn move_group_keeps_its_order() {
        let mut active = ids(&["a", "b", "c", "d", "e"]);
        // "b" and "d" dropped at the end
        move_mods(&mut active, &ids(&["b", "d"]), 5);
        assert_eq!(active, ids(&["a", "c", "e", "b", "d"]));
        // and back to the top, past the end index
        move_mods(&mut active, &ids(&["b", "d"]), 0);
        assert_eq!(active, ids(&["b", "d", "a", "c", "e"]));
        move_mods(&mut active, &ids(&["a", "e"]), 99);
        assert_eq!(active, ids(&["b", "d", "c", "a", "e"]));
    }

    #[test]
    fn mods_from_the_available_list_are_inserted() {
        let mut active = ids(&["core", "x"]);
        move_mods(&mut active, &ids(&["Author.New", "author.new", "Other.Mod"]), 1);
        assert_eq!(active, ids(&["core", "author.new", "other.mod", "x"]));
    }

    #[test]
    fn removing_keeps_core() {
        let mut active = ids(&["ludeon.rimworld", "a", "b"]);
        assert_eq!(remove_mods(&mut active, &ids(&["Ludeon.RimWorld", "A"])), 1);
        assert_eq!(active, ids(&["ludeon.rimworld", "b"]));
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
