use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui::{self, CornerRadius};

use crate::analyzer::{analyze_rimworld_log, detect_rimworld_log_paths, LogReport};
use crate::categories::ModCategory;
use crate::cleaner::{delete_mods, delete_translation_mods, unsubscribe_and_delete_single};
use crate::community_rules::CommunityDatabase;
use crate::dependencies::{validate_dependencies, DependencyProblem, DependencyReport};
use crate::game_install::known_launch_flags;
use crate::i18n::{fmt, tr, I18n, Language, TranslationKey};
use crate::matcher::{classify_and_link_translations, filter_and_rank_workshop_translations};
use crate::models::{is_optimizer_blacklisted, is_protected_package_id, ModInfo, TranslationTargetLang};
use crate::mods_config::{get_default_mods_config_path, load_active_mods, save_active_mods};
use crate::optimizer::{
    default_cpu_threads, dirs_from_mods, rollback_dds_async, run_optimization, texture_summary_async,
    total_system_threads, ConvertOptions, OptimizationMsg, OptimizeScope, RollbackResult, TextureSummary,
};
use crate::scanner::scan_mods;
use crate::sorter::sort_load_order_with_db;
use crate::steam::{detect_steam_paths, launch_rimworld, SteamPaths};
use crate::ui::icons::*;
use crate::ui::theme::THEME;
use crate::workshop_search::{search_steam_workshop, WorkshopItemResult};

/// The icon a tab gets in a narrow window.
fn tab_icon(tab: AppTab) -> egui::ImageSource<'static> {
    match tab {
        AppTab::Mods => ICON_MODS,
        AppTab::Workshop => ICON_STEAM,
        AppTab::Sets => ICON_COPY,
        AppTab::Saves => ICON_SAVE,
        AppTab::Translations => ICON_TRANSLATIONS,
        AppTab::Optimizer => ICON_OPTIMIZER,
        AppTab::Analyzer => ICON_SEARCH,
        AppTab::Settings => ICON_SETTINGS,
    }
}

fn ignored_duplicates_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("ignored_duplicates.json")
}

fn load_ignored_duplicates(path: &Path) -> HashMap<String, Vec<PathBuf>> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_ignored_duplicates(path: &Path, ignored: &HashMap<String, Vec<PathBuf>>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(ignored).map_err(std::io::Error::other)?;
    crate::fs_util::write_atomic(path, text)
}

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
pub fn passes_category_filter(filter: &std::collections::HashSet<ModCategory>, m: Option<&ModInfo>) -> bool {
    filter.is_empty()
        || m.map(|m| m.categories.iter().any(|c| filter.contains(c)))
            .unwrap_or(false)
}

/// Order of the available mods list (F9 of the plan). `Default` keeps the scan order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InactiveSort {
    #[default]
    Default,
    Name,
    /// Workshop subscribers, most first (fetched from Steam when chosen).
    Subscribers,
    /// Latest Workshop update, newest first.
    Updated,
    /// Folder size on disk, largest first.
    Size,
    /// Folder created (mod downloaded or copied), newest first.
    Added,
}

impl InactiveSort {
    pub const ALL: [InactiveSort; 6] = [
        InactiveSort::Default,
        InactiveSort::Name,
        InactiveSort::Subscribers,
        InactiveSort::Updated,
        InactiveSort::Size,
        InactiveSort::Added,
    ];

    pub fn key(self) -> TranslationKey {
        match self {
            InactiveSort::Default => TranslationKey::SortDefault,
            InactiveSort::Name => TranslationKey::SortName,
            InactiveSort::Subscribers => TranslationKey::SortSubscribers,
            InactiveSort::Updated => TranslationKey::SortUpdated,
            InactiveSort::Size => TranslationKey::SortSize,
            InactiveSort::Added => TranslationKey::SortAdded,
        }
    }

    /// Needs Steam's data of the Workshop items.
    pub fn needs_workshop(self) -> bool {
        matches!(self, InactiveSort::Subscribers | InactiveSort::Updated)
    }
}

/// DLCs a mod requires in its About.xml (F9 of the plan), lowercase IDs in DLC order.
pub fn required_dlcs(m: &ModInfo) -> Vec<&'static str> {
    crate::sorter::CANON_OFFICIAL_ORDER
        .iter()
        .copied()
        .filter(|dlc| dlc.starts_with("ludeon.rimworld."))
        .filter(|dlc| m.dependencies.iter().any(|d| d.trim().eq_ignore_ascii_case(dlc)))
        .collect()
}

/// "ludeon.rimworld.biotech" -> "Biotech" (DLC names are the same in every language).
pub fn dlc_name(id: &str) -> String {
    let short = id.rsplit('.').next().unwrap_or(id);
    let mut chars = short.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Undo / redo of the active list (F9 of the plan, item 23).
#[derive(Debug, Default)]
pub struct ActiveHistory {
    /// Earlier orders, oldest first; undone ones.
    pub undo: Vec<Vec<String>>,
    pub redo: Vec<Vec<String>>,
    /// The list as of the last look (`None` before the first one: nothing to compare).
    seen: Option<Vec<String>>,
}

impl ActiveHistory {
    const MAX_STEPS: usize = 100;

    /// Looks at the list once a frame; a change becomes an undo step (and drops the redo
    /// steps). Returns the list as it was before the change.
    pub fn observe(&mut self, current: &[String]) -> Option<Vec<String>> {
        let Some(seen) = self.seen.as_ref() else {
            self.seen = Some(current.to_vec());
            return None;
        };
        if seen == current {
            return None;
        }
        let before = self.seen.replace(current.to_vec())?;
        self.undo.push(before.clone());
        if self.undo.len() > Self::MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        Some(before)
    }

    /// The list read from disk, not an edit: nothing to undo for it.
    pub fn settle(&mut self, current: &[String]) {
        self.seen = Some(current.to_vec());
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// The list before the last change; `current` becomes a redo step.
    pub fn undo(&mut self, current: &[String]) -> Option<Vec<String>> {
        let prev = self.undo.pop()?;
        self.redo.push(current.to_vec());
        self.seen = Some(prev.clone());
        Some(prev)
    }

    pub fn redo(&mut self, current: &[String]) -> Option<Vec<String>> {
        let next = self.redo.pop()?;
        self.undo.push(current.to_vec());
        self.seen = Some(next.clone());
        Some(next)
    }
}

/// Mods of `before` that are not in `now` and that a save's `save_ids` lists (Core is never
/// turned off, so it is left out). Letter case doesn't matter.
pub fn turned_off_from_save(before: &[String], now: &[String], save_ids: &[String]) -> Vec<String> {
    let now: std::collections::HashSet<String> = now.iter().map(|id| id.to_lowercase()).collect();
    before
        .iter()
        .filter(|id| !now.contains(&id.to_lowercase()) && !is_protected_package_id(id))
        .filter(|id| save_ids.iter().any(|s| s.eq_ignore_ascii_case(id)))
        .cloned()
        .collect()
}

/// Folder size in bytes (all files below it).
pub fn folder_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|md| md.is_file())
        .map(|md| md.len())
        .sum()
}

/// When the folder was created (Unix seconds), or modified where creation isn't known.
pub fn folder_added(dir: &Path) -> Option<u64> {
    let md = std::fs::metadata(dir).ok()?;
    let t = md.created().or_else(|_| md.modified()).ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteKind {
    /// "Delete all translations" for the selected language
    Translations,
    /// A single mod (trash icon)
    Single,
    /// Mods chosen in the mod lists' context menu
    Mods,
    /// Unsubscribe only: Steam removes the files itself
    Unsubscribe,
    /// Folders to the Recycle Bin, then the mods are downloaded again
    Reinstall,
    /// Not a deletion: keep the installed version (`freeze.rs`)
    Freeze,
    /// Not a deletion either: let the mod update again
    Unfreeze,
}

/// A mod a background deletion works on: package ID, folder, Workshop ID.
type DeleteTarget = (String, PathBuf, Option<String>);

/// What a finished background deletion reports back.
#[derive(Debug)]
pub struct DeleteDone {
    pub message: String,
    /// The Steam step worked (for `Unsubscribe`: the mods are unsubscribed).
    pub steam_ok: bool,
}

/// A Workshop collection's ID and its mods (or why they didn't load).
pub type CollectionMods = (String, Result<Vec<crate::workshop_browse::WorkshopItem>, String>);

/// Workshop ID of a mod: from the Workshop folder, or a local copy kept in a folder named
/// by its ID (SteamCMD downloads).
pub fn workshop_id_of(m: &ModInfo) -> Option<String> {
    m.steam_item_id.clone().or_else(|| {
        m.dir_path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| crate::modlists::is_workshop_id(n))
            .map(str::to_string)
    })
}

/// Workshop ID to read a mod's change notes by: `workshop_id_of`, and for a frozen copy
/// (its folder has the mod's name) the ID its freeze mark remembers.
pub fn changelog_id_of(m: &ModInfo) -> Option<String> {
    workshop_id_of(m).or_else(|| {
        m.frozen
            .then(|| crate::freeze::read(&m.dir_path))
            .flatten()
            .and_then(|f| f.workshop_id)
    })
}

/// A mod that came from the Workshop and isn't frozen yet can be frozen.
pub fn can_freeze(m: &ModInfo) -> bool {
    !m.frozen && workshop_id_of(m).is_some()
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
    /// "Restore previous version" of a mod (lowercase package ID), confirmed first.
    RestoreVersion(String),
    /// An edit turned off mods the newest save is made with (F9 of the plan, item 24).
    RemovedFromSave { save: String, names: Vec<String> },
    /// Settings were loaded from a file: restart now or later (item 22).
    RestartAfterImport,
    /// Active mod textures exceed ~80% of dedicated video memory (V2).
    VramWarning {
        estimated_bytes: u64,
        total_vram_bytes: u64,
        heavy_mods: Vec<(String, u64)>,
        /// Measured in the game (companion), not estimated from the files.
        measured: bool,
    },
}

/// Window for a mod set action that needs a name or a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModListDialog {
    /// Save the current load order as a new set.
    SaveAs {
        name: String,
        error: Option<String>,
    },
    Rename {
        from: String,
        name: String,
        error: Option<String>,
    },
    Delete {
        name: String,
    },
    /// F5: import a Steam Workshop collection by URL (or a list of Workshop links).
    Collection {
        url: String,
        error: Option<String>,
    },
}

/// What a modpack is compared with in the "Compare" window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareWith {
    /// The list on the Mods tab.
    Current,
    Modpack(String),
    Save(String),
}

/// The "Compare" window of the Modpacks tab: a modpack against another list.
#[derive(Debug, Clone)]
pub struct SetCompare {
    pub first: String,
    pub with: CompareWith,
    pub result: crate::modlists::Comparison,
    /// Mod names by lowercase package ID.
    pub names: HashMap<String, String>,
}

/// The "What changed" window: a Workshop mod's change notes.
#[derive(Debug, Clone)]
pub struct ChangelogWindow {
    pub workshop_id: String,
    pub title: String,
    /// When the installed copy was published (unix seconds), if the mod is installed and
    /// Steam's record (or the freeze mark) says so: later notes are what an update brings.
    pub installed: Option<u64>,
    /// `None` while the page is downloaded.
    pub notes: Option<Result<Vec<crate::changelog::ChangeNote>, String>>,
}

/// A downloaded change notes page: Workshop ID, the installed copy's time, the notes.
type ChangelogLoaded = (String, Option<u64>, Result<Vec<crate::changelog::ChangeNote>, String>);

/// F7: window for renaming or deleting a save file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveDialog {
    Rename {
        from: String,
        name: String,
        error: Option<String>,
    },
    Delete {
        name: String,
    },
}

/// Translations of one mod found by the modpack-wide search.
#[derive(Debug, Clone)]
pub struct ModTranslations {
    /// The mod's package ID (lowercase): its row in the mod list gets a hint (T2).
    pub package_id: String,
    pub mod_name: String,
    pub items: Vec<WorkshopItemResult>,
}

/// Messages from the modpack-wide translation search.
pub enum BatchSearchMsg {
    Progress {
        done: usize,
        total: usize,
    },
    Found(ModTranslations),
    Done {
        searched: usize,
        errors: usize,
        cancelled: bool,
    },
}

/// A Workshop preview image: being downloaded, shown, or not available.
pub enum PreviewSlot {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

/// Messages from the "find by name" search (D1).
pub enum NameSearchMsg {
    Found(usize, Vec<crate::name_search::NameMatch>),
    Failed(usize),
    Done,
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

/// Counts per mod and the new cache entries (`optimizer::non_power_of_two_by_mod`).
type OddTexturesResult = (HashMap<String, (u32, u32)>, Vec<crate::optimizer::TextureSidesEntry>);

/// Mods being selected by holding the button and sweeping over the rows.
#[derive(Debug, Clone)]
pub struct ModSweep {
    pub from_active: bool,
    /// The row the sweep started on; the selection runs from it to the row under the cursor.
    pub anchor: String,
    /// What stays selected besides the swept rows (the selection before a Ctrl-sweep).
    pub base: Vec<String>,
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
    Workshop,
    Sets,
    Saves,
    Translations,
    Optimizer,
    Analyzer,
    Settings,
}

impl AppTab {
    /// Every tab, in the side bar's order.
    pub const ALL: [AppTab; 8] = [
        AppTab::Mods,
        AppTab::Workshop,
        AppTab::Sets,
        AppTab::Saves,
        AppTab::Translations,
        AppTab::Optimizer,
        AppTab::Analyzer,
        AppTab::Settings,
    ];
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
    /// Problems the player hid with "Ignore" on the dependency alert. The alert stays hidden
    /// while it has no other problems, until the next sort or the end of the session.
    pub dep_alert_ignored: Vec<DependencyProblem>,
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
    /// RWMM_TAB=saves-findname: "find by name" for the selected save once the mods and the
    /// saves are read (screenshots).
    name_search_after_load: bool,
    /// RWMM_TAB=mods-updates: check the Workshop for updates once the mods are scanned (screenshots).
    updates_after_scan: bool,
    /// RWMM_TAB=analyzer-companion: scroll the Log Analyzer to the companion's report once the
    /// log is analyzed (screenshots).
    pub scroll_to_companion_report: bool,

    // Optimizer State
    pub opt_scope: OptimizeScope,
    pub opt_max_size: crate::optimizer::MaxTextureSize,
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
    /// Background VRAM check report vs GPU capacity (V2).
    pub vram_report: Option<crate::optimizer::VramReport>,
    vram_rx: Option<Receiver<Result<crate::optimizer::VramCheckResult, ()>>>,
    vram_cache: HashMap<PathBuf, (u64, u64)>,
    /// Active mods changed while a VRAM scan was already running: scan again when it finishes.
    vram_recheck: bool,
    /// "Play" is waiting for the running VRAM scan to finish before deciding whether to warn.
    launch_after_vram_check: bool,

    // Analyzer State
    pub log_report: Option<LogReport>,
    /// What the companion mod wrote after the last loading with it (textures, conflicts).
    pub companion_report: Option<crate::companion_report::CompanionReport>,
    /// The mods' load per game tick, as the companion last measured it (`tick_profile.rs`).
    pub tick_profile: Option<crate::tick_profile::TickProfile>,
    /// The next launch from the manager runs with the game's verbose logging, so the log
    /// holds how long each mod took to load (`load_profile`).
    pub measure_next_launch: bool,
    /// Lowercase package ID -> (textures with a side not a power of two, all PNG/JPG) of the
    /// mods in the log's load profile (1.7 loads those twice).
    pub odd_textures: HashMap<String, (u32, u32)>,
    odd_textures_rx: Option<Receiver<OddTexturesResult>>,
    /// The log (path, size) `odd_textures` were counted for.
    odd_textures_for: Option<(PathBuf, u64)>,
    texture_sides_cache: Option<HashMap<PathBuf, (u64, u32, u32)>>,
    pub custom_log_path: Option<PathBuf>,
    /// F9: Snapshot of the last known working run and diff with current configuration.
    pub last_good: Option<crate::last_good::LastGoodSnapshot>,
    pub last_good_diff: Option<crate::last_good::LastGoodDiff>,
    last_good_rx: Option<Receiver<crate::last_good::LastGoodDiff>>,
    /// A new "last working run" snapshot being written in the background.
    last_good_save_rx: Option<Receiver<crate::last_good::LastGoodSnapshot>>,
    /// The active list changed while the diff was being computed: compute it again.
    last_good_rerun: bool,
    /// F13: Log viewer mode (Report / FullLog) and filters.
    pub log_filter_text: String,
    pub log_level_filter: crate::analyzer::LogLevelFilter,

    // Settings & Paths State
    pub steam_paths: SteamPaths,
    pub mods_config_path: Option<PathBuf>,
    /// Checked launch flags (from `known_launch_flags` of the detected engine).
    pub launch_flags: std::collections::HashSet<String>,
    /// Free-form extra launch arguments, split on whitespace.
    pub launch_extra_args: String,
    /// Backup latest save before launching the game (F12).
    pub backup_save_before_launch: bool,
    /// Number of save backups to retain per save stem (F12).
    pub save_backups_to_keep: usize,
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
    /// Progress of the running Steam client download (`GetItemDownloadInfo`).
    pub steam_progress: Option<crate::steam::SteamDownloadProgress>,
    steam_progress_rx: Option<Receiver<crate::steam::SteamDownloadProgress>>,
    /// Flag to cancel waiting for Steam client download early (S1).
    pub steam_stop_flag: Arc<AtomicBool>,
    /// History of game loading times and changes (F11).
    pub load_time_history: crate::load_time::LoadTimeHistory,
    /// Wall-clock timestamp when RimWorld was launched from the manager, used to verify
    /// that `Player.log` on disk was actually produced by this launch (F9 / F11).
    pub game_launch_time: Option<std::time::SystemTime>,
    /// Active mods at the time of the launch.
    pub game_launch_active_mods: Vec<String>,
    /// Background watcher waiting for RimWorld to exit after a normal launch, then analyzing Player.log.
    launch_watch_rx: Option<Receiver<Option<LogReport>>>,
    /// Save backup made before a launch (F12), running in the background.
    save_backup_rx: Option<Receiver<Result<(), String>>>,

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
    pub delete_rx: Option<Receiver<DeleteDone>>,
    /// What the running deletion does, and to which mods (ID, folder, Workshop ID).
    delete_job: Option<(DeleteKind, Vec<DeleteTarget>)>,
    /// Background reset of the 1.7 game cache.
    pub cache_rx: Option<Receiver<String>>,
    /// Background removal of DDS files that are not in the manager's log.
    pub legacy_rx: Option<Receiver<String>>,
    /// Mod sets from the game's ModLists folder (F4), by name.
    pub modlists: Vec<crate::modlists::ModList>,
    /// Card pictures of the sets by name: the texture and whether it is the user's own
    /// (else a generated placeholder). Filled by the Modpacks tab.
    pub modlist_pictures: HashMap<String, (egui::TextureHandle, bool)>,
    /// Tags of the sets by name (`modlist_tags.json`).
    pub modlist_tags: crate::modlist_tags::Tags,
    /// Modpacks tab: show only sets with any of these tags (lower case).
    pub sets_tag_filter: std::collections::HashSet<String>,
    /// Mods already warned about in this session: enabled without declaring the game version.
    warned_no_version: std::collections::HashSet<String>,
    /// The set last loaded or saved here.
    pub current_modlist: Option<String>,
    /// Set selected on the Sets tab, and its search query.
    pub selected_modlist: Option<String>,
    pub sets_search: String,
    pub modlist_dialog: Option<ModListDialog>,
    collection_rx: Option<Receiver<Result<crate::mod_updates::RemoteCollection, String>>>,
    /// Mod settings kept with the sets, by set name (`modlist_settings.rs`).
    pub modlist_settings: HashMap<String, crate::modlist_settings::Saved>,
    /// Modpacks tab: the "Compare" window.
    pub set_compare: Option<SetCompare>,
    /// The "What changed" window of a Workshop mod, and its notes being downloaded.
    pub changelog: Option<ChangelogWindow>,
    changelog_rx: Option<Receiver<ChangelogLoaded>>,

    // Saves (F7): headers of the game's saves, read in the background.
    pub saves: Vec<crate::saves::SaveInfo>,
    saves_rx: Option<Receiver<Vec<crate::saves::SaveInfo>>>,
    saves_loaded_at: Option<std::time::Instant>,
    pub selected_save: Option<String>,
    pub saves_search: String,
    pub save_dialog: Option<SaveDialog>,

    // Mod updates (F10): the last Workshop check, and the list filters for it.
    pub mod_updates: Option<crate::mod_updates::UpdateReport>,
    mod_updates_rx: Option<Receiver<Result<crate::mod_updates::UpdateReport, String>>>,

    /// D1: package ID -> Workshop ID (RimSort's database), for mods the game saved with ID 0.
    pub workshop_index: Option<std::sync::Arc<crate::workshop_db::WorkshopIndex>>,
    /// (downloaded now, result): loading the saved index at start, or a download.
    workshop_index_rx: Option<Receiver<(bool, Result<crate::workshop_db::WorkshopIndex, String>)>>,
    /// Workshop items asked about (missing mods of a modpack), and the answers: the title of
    /// each one that's there; one Steam doesn't return is removed or hidden.
    pub ws_item_checked: HashSet<String>,
    pub ws_item_titles: HashMap<String, String>,
    ws_item_rx: Option<Receiver<WorkshopItemsAnswer>>,
    /// D1: the "find by name" window for missing mods with no Workshop link.
    pub name_search: Option<crate::name_search::NameSearch>,
    name_search_rx: Option<Receiver<NameSearchMsg>>,
    name_search_stop: Arc<AtomicBool>,

    /// Show only mods with a newer Workshop version.
    pub filter_updates: bool,
    /// Show only mods that don't declare the installed game version.
    pub filter_no_version: bool,
    /// Show only mods from the game's Mods folder (not Workshop, not Core/DLC).
    pub filter_local: bool,
    /// Show only Workshop subscriptions (not Core/DLC). Never on together with `filter_local`.
    pub filter_workshop: bool,
    /// Colour marks of mods (`mod_colors.rs`) and the colours shown (empty: all).
    pub mod_colors: crate::mod_colors::Marks,
    pub color_filter: std::collections::HashSet<crate::mod_colors::ModColor>,
    /// DLCs shown (lowercase IDs; empty: all): mods that require one of them.
    pub dlc_filter: std::collections::HashSet<&'static str>,
    /// Copies in the duplicates card the player hid (`ignored_duplicates.json`): lowercase
    /// package ID -> the folders it had then. A new copy shows the mod again.
    pub ignored_duplicates: HashMap<String, Vec<PathBuf>>,
    /// Order of the available mods list and the data it needs, loaded when first chosen:
    /// Steam's details of the Workshop items (by Workshop ID), folder sizes and creation
    /// dates (by lowercase package ID).
    pub inactive_sort: InactiveSort,
    pub workshop_stats: Option<HashMap<String, crate::mod_updates::RemoteItem>>,
    workshop_stats_rx: Option<Receiver<Result<HashMap<String, crate::mod_updates::RemoteItem>, String>>>,
    pub mod_sizes: Option<HashMap<String, u64>>,
    mod_sizes_rx: Option<Receiver<HashMap<String, u64>>>,
    pub mod_added: Option<HashMap<String, u64>>,
    /// Previous versions saved before an update (`mod_versions.rs`), by lowercase package ID.
    pub mod_versions: HashMap<String, crate::mod_versions::SavedVersion>,
    mod_versions_rx: Option<Receiver<(Vec<crate::mod_versions::SavedVersion>, Vec<String>)>>,
    mod_restore_rx: Option<Receiver<(String, Result<PathBuf, String>)>>,
    /// Save the installed copy when the update check finds a newer one (on unless turned off).
    pub keep_mod_versions: bool,
    /// Undo / redo of the active list (F9 of the plan, item 23).
    active_history: ActiveHistory,
    /// The next change replaces the whole list on purpose (a modpack, a save, a backup):
    /// no "used by the last save" warning (item 24).
    skip_save_check: bool,
    /// Settings were imported from a file (item 22): they apply after a restart.
    pub settings_imported: bool,
    /// Narrow window: the Mods tab shows one list; this says which one.
    pub mods_show_active: bool,
    /// Narrow window: the category chips are folded behind a "Filters" button.
    pub show_filter_chips: bool,

    // Broken-mod search (F8). The session is saved to bisect.json after every change,
    // so it survives closing the program in the middle.
    pub bisect: Option<crate::bisect::Bisect>,
    pub bisect_open: bool,
    /// Setup screen: mods ticked as "never turn off" (lowercase IDs).
    pub bisect_keep: Option<std::collections::HashSet<String>>,
    pub bisect_filter: String,
    /// Why the last wizard action didn't run (game running, unsaved order...).
    pub bisect_note: Option<String>,
    /// Watches for RimWorld to exit after a bisect launch and analyzes Player.log.
    bisect_watch_rx: Option<Receiver<Option<LogReport>>>,
    /// "History" window: ModsConfig.xml backups, newest first (`Some` = window open).
    pub history: Option<Vec<crate::mods_config::ConfigBackup>>,
    /// Look for a new version on GitHub once a day (Settings).
    pub check_updates: bool,
    /// Workshop tab may ask the Steam client when the website refuses (Settings).
    pub workshop_via_steam_client: bool,
    /// SteamCMD: download mods with it instead of the Steam client (Settings).
    pub use_steamcmd: bool,
    /// The user set the checkbox; until then it follows "SteamCMD is installed".
    pub use_steamcmd_set: bool,
    pub custom_steamcmd_dir: Option<PathBuf>,
    /// The last SteamCMD job (install or download) for its window; `steamcmd_rx` while it runs.
    pub steamcmd_job: Option<crate::steamcmd::Job>,
    steamcmd_rx: Option<Receiver<crate::steamcmd::Msg>>,
    steamcmd_kill: crate::steamcmd::KillSwitch,
    pub steamcmd_window: bool,
    /// The running job installs SteamCMD: once it's in place, downloads go through it.
    steamcmd_fresh_install: bool,
    /// The SteamCMD window shows the console (own commands) instead of the last download.
    pub steamcmd_console: bool,
    pub steamcmd_console_input: String,

    // Workshop tab: the page shown, its items, previews by URL.
    pub ws_query: crate::workshop_browse::BrowseQuery,
    pub ws_search: String,
    pub ws_items: Vec<crate::workshop_browse::WorkshopItem>,
    pub ws_selected: Option<String>,
    pub ws_error: Option<String>,
    ws_rx: Option<Receiver<Result<crate::workshop_browse::Loaded, String>>>,
    /// Where the last list came from: once the website refuses, the Steam client goes first.
    pub ws_source: crate::workshop_browse::Source,
    ws_loaded: bool,
    /// The picked collection's mods: its ID and the result.
    pub ws_collection: Option<CollectionMods>,
    ws_collection_rx: Option<Receiver<CollectionMods>>,
    /// Items sent to Steam or SteamCMD from the tab this session.
    pub ws_requested: std::collections::HashSet<String>,
    ws_previews: std::collections::HashMap<String, PreviewSlot>,
    ws_preview_tx: Option<std::sync::mpsc::Sender<(String, String, u32)>>,
    ws_preview_rx: Option<Receiver<(String, Option<egui::ColorImage>)>>,
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
    log_silent: bool,
    initial_log_checked: bool,

    /// Lowercase packageId -> index in `all_mods` (M1: no linear search per row).
    pub mod_index: std::collections::HashMap<String, usize>,
    /// Measured row heights of the two mod lists (for drawing only the visible rows).
    pub active_row_height: f32,
    pub inactive_row_height: f32,

    /// Drag and drop of mods (F1): what is dragged and where it lands this frame.
    pub mod_drag: Option<ModDrag>,
    pub drop_target: Option<DropTarget>,
    pub mod_sweep: Option<ModSweep>,
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
        let workshop_via_steam_client = s.workshop_via_steam_client;
        let keep_mod_versions = s.keep_mod_versions;
        let custom_steamcmd_dir = s.steamcmd_dir.clone();
        let use_steamcmd_set = s.use_steamcmd.is_some();
        // Never set: SteamCMD installed means downloads go through it.
        let use_steamcmd = s.use_steamcmd.unwrap_or_else(|| {
            crate::steamcmd::is_installed(&custom_steamcmd_dir.clone().unwrap_or_else(crate::steamcmd::default_dir))
        });
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

        // RWMM_TAB=settings|optimizer|analyzer|analyzer-companion|translations|history opens that
        // tab (or the History window) first, for screenshots.
        let start_view = std::env::var("RWMM_TAB").unwrap_or_default().to_lowercase();
        let start_tab = match start_view.as_str() {
            "sets" => AppTab::Sets,
            "saves" | "saves-findname" => AppTab::Saves,
            "workshop" => AppTab::Workshop,
            "translations" | "translations-batch" => AppTab::Translations,
            "bisect" => AppTab::Analyzer,
            "optimizer" => AppTab::Optimizer,
            "analyzer" | "analyzer-companion" => AppTab::Analyzer,
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
            dep_alert_ignored: Vec::new(),
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
            scroll_to_companion_report: false,
            name_search_after_load: false,
            updates_after_scan: false,

            opt_scope: OptimizeScope::AllInstalled,
            opt_max_size: s.opt_max_size.unwrap_or_default(),
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
            vram_report: None,
            vram_rx: None,
            vram_cache: HashMap::new(),
            vram_recheck: false,
            launch_after_vram_check: false,

            log_report: None,
            custom_log_path: None,
            last_good: crate::last_good::LastGoodSnapshot::load(),
            last_good_diff: None,
            last_good_rx: None,
            last_good_save_rx: None,
            last_good_rerun: false,
            log_filter_text: String::new(),
            log_level_filter: crate::analyzer::LogLevelFilter::default(),

            steam_paths,
            mods_config_path,
            launch_flags,
            launch_extra_args: s.launch_extra_args.clone(),
            backup_save_before_launch: s.backup_save_before_launch,
            save_backups_to_keep: s.save_backups_to_keep,
            custom_game_path,
            custom_workshop_path,
            custom_config_path,
            last_saved_settings: s,
            community_db_rules_count: db_rules_count,
            community_db,
            rules_rx: None,
            steam_rx: None,
            steam_progress: None,
            steam_progress_rx: None,
            steam_stop_flag: Arc::new(AtomicBool::new(false)),
            load_time_history: crate::load_time::LoadTimeHistory::load(),
            companion_report: crate::companion_report::load(),
            tick_profile: crate::tick_profile::load(),
            measure_next_launch: false,
            odd_textures: HashMap::new(),
            odd_textures_rx: None,
            odd_textures_for: None,
            texture_sides_cache: None,
            game_launch_time: None,
            game_launch_active_mods: Vec::new(),
            launch_watch_rx: None,
            save_backup_rx: None,

            pending_delete: None,
            saved_active_mods: Vec::new(),
            reload_active_from_disk: true,
            pending_question: None,
            allow_close: false,
            delete_rx: None,
            delete_job: None,
            cache_rx: None,
            legacy_rx: None,
            history: None,
            modlists: Vec::new(),
            modlist_pictures: HashMap::new(),
            modlist_tags: crate::modlist_tags::load(&crate::modlist_tags::tags_path()),
            sets_tag_filter: std::collections::HashSet::new(),
            warned_no_version: std::collections::HashSet::new(),
            current_modlist: None,
            // RWMM_SET=<name> opens that modpack on the Modpacks tab (screenshots).
            selected_modlist: std::env::var("RWMM_SET").ok().filter(|s| !s.is_empty()),
            sets_search: String::new(),
            modlist_dialog: None,
            collection_rx: None,
            modlist_settings: HashMap::new(),
            set_compare: None,
            changelog: None,
            changelog_rx: None,
            saves: Vec::new(),
            saves_rx: None,
            saves_loaded_at: None,
            // RWMM_SAVE=<name> opens that save on the Saves tab (screenshots).
            selected_save: std::env::var("RWMM_SAVE").ok().filter(|s| !s.is_empty()),
            saves_search: String::new(),
            save_dialog: None,
            mod_updates: None,
            mod_updates_rx: None,
            workshop_index: None,
            workshop_index_rx: None,
            ws_item_checked: HashSet::new(),
            ws_item_titles: HashMap::new(),
            ws_item_rx: None,
            name_search: None,
            name_search_rx: None,
            name_search_stop: Arc::new(AtomicBool::new(false)),
            filter_updates: false,
            filter_no_version: false,
            filter_local: false,
            filter_workshop: false,
            mod_colors: crate::mod_colors::load(&crate::mod_colors::marks_path()),
            color_filter: std::collections::HashSet::new(),
            dlc_filter: std::collections::HashSet::new(),
            ignored_duplicates: load_ignored_duplicates(&ignored_duplicates_path()),
            inactive_sort: InactiveSort::Default,
            workshop_stats: None,
            workshop_stats_rx: None,
            mod_sizes: None,
            mod_sizes_rx: None,
            mod_added: None,
            mod_versions: crate::mod_versions::list(&crate::mod_versions::versions_dir()),
            mod_versions_rx: None,
            mod_restore_rx: None,
            keep_mod_versions,
            active_history: ActiveHistory::default(),
            skip_save_check: false,
            settings_imported: false,
            mods_show_active: true,
            show_filter_chips: false,
            // An unfinished search opens again where it stopped.
            bisect_open: bisect_session.is_some(),
            bisect: bisect_session,
            bisect_keep: None,
            bisect_filter: String::new(),
            bisect_note: None,
            bisect_watch_rx: None,
            check_updates: last_check_updates,
            workshop_via_steam_client,
            use_steamcmd,
            use_steamcmd_set,
            custom_steamcmd_dir,
            steamcmd_job: None,
            steamcmd_rx: None,
            steamcmd_kill: Arc::new(std::sync::Mutex::new(None)),
            steamcmd_window: false,
            steamcmd_fresh_install: false,
            steamcmd_console: false,
            steamcmd_console_input: String::new(),
            ws_query: crate::workshop_browse::BrowseQuery {
                page: 1,
                ..Default::default()
            },
            ws_search: String::new(),
            ws_items: Vec::new(),
            ws_selected: None,
            ws_error: None,
            ws_rx: None,
            ws_source: crate::workshop_browse::Source::Web,
            ws_loaded: false,
            ws_collection: None,
            ws_collection_rx: None,
            ws_requested: std::collections::HashSet::new(),
            ws_previews: std::collections::HashMap::new(),
            ws_preview_tx: None,
            ws_preview_rx: None,
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
            log_silent: false,
            initial_log_checked: false,
            mod_index: std::collections::HashMap::new(),
            active_row_height: 34.0,
            inactive_row_height: 34.0,
            mod_drag: None,
            mod_sweep: None,
            drop_target: None,
            active_scroll_offset: 0.0,
            active_scroll_to: None,

            status_message: tr!("Готов к работе", "Ready", "就绪").to_string(),
        };

        // Scans in the background: the window opens at once and fills in when the scan is done.
        app.rescan_mods();
        app.start_update_check();
        if start_view == "history" {
            app.open_history();
        }
        // Screenshots of the Log Analyzer: the last log read, its errors shown.
        if start_view == "analyzer" {
            app.run_log_analysis();
            if let Some(ctx) = app.repaint_ctx.clone() {
                crate::ui::view_analyzer::open_errors(&ctx);
            }
        }
        app.batch_search_after_scan = start_view == "translations-batch";
        app.bisect_after_scan = start_view == "bisect";
        app.name_search_after_load = start_view == "saves-findname";
        app.updates_after_scan = start_view == "mods-updates";
        app.scroll_to_companion_report = start_view == "analyzer-companion";
        // The Workshop ID index downloaded earlier (a few MB): read in the background.
        app.workshop_index_rx = Some(app.spawn_task(|| (false, crate::workshop_db::load().ok_or_else(String::new))));
        // A measured launch whose game closed after the manager did: verbose logging off now.
        if crate::load_profile::measured_launch_pending() && crate::game_install::game_pid().is_none() {
            crate::load_profile::finish_measured_launch();
        }
        // The start tab is hidden (Mods can be): open the first one shown instead.
        if start_view.is_empty() && !app.tab_visible(app.current_tab) {
            app.current_tab = app.visible_tabs().first().copied().unwrap_or(AppTab::Settings);
        }
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
    fn spawn_task<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
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
        // Data for the order of the available mods: what failed sorts as missing.
        if let Some(rx) = &self.workshop_stats_rx {
            if let Ok(result) = rx.try_recv() {
                self.workshop_stats_rx = None;
                match result {
                    Ok(stats) => self.workshop_stats = Some(stats),
                    Err(e) => {
                        self.status_message = fmt(self.i18n.get(TranslationKey::SortWorkshopError), &[&e]);
                        self.workshop_stats = Some(HashMap::new());
                    }
                }
            }
        }
        if let Some(rx) = &self.mod_sizes_rx {
            if let Ok(sizes) = rx.try_recv() {
                self.mod_sizes_rx = None;
                self.mod_sizes = Some(sizes);
            }
        }
        // Previous versions saved before an update: one line, errors after it.
        if let Some(rx) = &self.mod_versions_rx {
            if let Ok((saved, errors)) = rx.try_recv() {
                self.mod_versions_rx = None;
                self.mod_versions = crate::mod_versions::list(&crate::mod_versions::versions_dir());
                let t = self.i18n;
                if !saved.is_empty() {
                    let bytes: u64 = saved.iter().map(|v| v.size).sum();
                    self.status_message.push_str(" | ");
                    self.status_message.push_str(&fmt(
                        t.get(TranslationKey::VersionsSaved),
                        &[&saved.len().to_string(), &crate::workshop_browse::human_size(bytes)],
                    ));
                }
                if !errors.is_empty() {
                    self.status_message.push_str(" | ");
                    self.status_message
                        .push_str(&fmt(t.get(TranslationKey::VersionsSaveFailed), &[&errors.join("; ")]));
                }
            }
        }
        if let Some(rx) = &self.mod_restore_rx {
            if let Ok((name, result)) = rx.try_recv() {
                self.mod_restore_rx = None;
                let t = self.i18n;
                match result {
                    Ok(dir) => {
                        self.status_message = fmt(
                            t.get(TranslationKey::VersionsRestored),
                            &[&name, &dir.display().to_string()],
                        );
                        self.rescan_mods_keep_status();
                    }
                    Err(e) => self.status_message = fmt(t.get(TranslationKey::VersionsRestoreFailed), &[&name, &e]),
                }
            }
        }
        if let Some(rx) = &self.ws_item_rx {
            if let Ok((asked, result)) = rx.try_recv() {
                self.ws_item_rx = None;
                match result {
                    Ok(items) => {
                        for item in items {
                            self.ws_item_titles.insert(item.id, item.title);
                        }
                    }
                    // No answer at all (offline): ask again next time, mark nothing as removed.
                    Err(_) => {
                        for id in &asked {
                            self.ws_item_checked.remove(id);
                        }
                    }
                }
            }
        }
        if let Some(rx) = &self.workshop_index_rx {
            if let Ok((downloaded, result)) = rx.try_recv() {
                self.workshop_index_rx = None;
                let t = self.i18n;
                match result {
                    Ok(index) => {
                        if downloaded {
                            self.status_message =
                                fmt(t.get(TranslationKey::WorkshopDbReady), &[&index.len().to_string()]);
                        }
                        self.workshop_index = Some(std::sync::Arc::new(index));
                    }
                    // Nothing saved yet is not an error; a failed download is.
                    Err(e) if downloaded => {
                        self.status_message = fmt(t.get(TranslationKey::WorkshopDbError), &[&e]);
                    }
                    Err(_) => {}
                }
            }
        }
        if let Some(rx) = &self.mod_updates_rx {
            if let Ok(result) = rx.try_recv() {
                self.mod_updates_rx = None;
                let t = self.i18n;
                match result {
                    Ok(report) => {
                        let n = report.outdated_count();
                        self.status_message = if n == 0 {
                            fmt(t.get(TranslationKey::UpdatesNone), &[&report.checked.to_string()])
                        } else {
                            fmt(
                                t.get(TranslationKey::UpdatesFound),
                                &[&n.to_string(), &report.checked.to_string()],
                            )
                        };
                        if n == 0 {
                            self.filter_updates = false;
                        }
                        if !report.gone.is_empty() {
                            self.status_message.push_str(" | ");
                            self.status_message.push_str(&fmt(
                                t.get(TranslationKey::UpdatesGone),
                                &[&report.gone.len().to_string()],
                            ));
                        }
                        self.save_previous_versions(&report);
                        self.mod_updates = Some(report);
                    }
                    Err(e) => self.status_message = fmt(t.get(TranslationKey::UpdatesError), &[&e]),
                }
            }
        }
        if let Some(rx) = &self.saves_rx {
            if let Ok(saves) = rx.try_recv() {
                self.saves_rx = None;
                self.saves = saves;
                if let Some(sel) = &self.selected_save {
                    if !self.saves.iter().any(|s| &s.name == sel) {
                        self.selected_save = None;
                    }
                }
                self.name_search_when_ready();
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
                        self.status_message = tr!(
                            "Ошибка поиска в Workshop: {}",
                            "Workshop search failed: {}",
                            "创意工坊搜索失败：{}",
                            e
                        )
                    }
                }
            }
        }
        if let Some(rx) = &self.steam_progress_rx {
            let t = self.i18n;
            while let Ok(prog) = rx.try_recv() {
                self.status_message = if prog.bytes_total > 0 || prog.installed > 0 {
                    fmt(
                        t.get(TranslationKey::SteamDownloadProgress),
                        &[
                            &prog.installed.to_string(),
                            &prog.total.to_string(),
                            &prog.percent().to_string(),
                        ],
                    )
                } else {
                    fmt(
                        t.get(TranslationKey::SteamDownloadProgressNoPct),
                        &[&prog.installed.to_string(), &prog.total.to_string()],
                    )
                };
                self.steam_progress = Some(prog);
            }
        }
        if let Some(rx) = &self.steam_rx {
            if let Ok(msg) = rx.try_recv() {
                self.steam_rx = None;
                self.steam_progress_rx = None;
                let installed = self.steam_progress.take().map_or(0, |p| p.installed);
                self.status_message = msg;
                if installed > 0 {
                    self.rescan_mods_keep_status();
                }
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
                // F9 / F11 are recorded only from the launch watcher, once the game has
                // closed: a log analyzed by hand may be of a game still loading.
                if !std::mem::take(&mut self.log_silent) {
                    self.status_message = self.crash_summary_text(&report);
                }
                self.log_report = Some(report);
                self.companion_report = crate::companion_report::load();
                self.tick_profile = crate::tick_profile::load();
            }
        }
        if let Some(rx) = &self.launch_watch_rx {
            if let Ok(opt_report) = rx.try_recv() {
                self.launch_watch_rx = None;
                if let Some(report) = opt_report {
                    self.handle_verified_launch_report(&report);
                    self.status_message = self.crash_summary_text(&report);
                    self.log_report = Some(report);
                    self.companion_report = crate::companion_report::load();
                    self.tick_profile = crate::tick_profile::load();
                }
            }
        }
        if let Some(rx) = &self.bisect_watch_rx {
            if let Ok(opt_report) = rx.try_recv() {
                self.bisect_watch_rx = None;
                if let Some(report) = opt_report {
                    // Only a clear log answers by itself: a crash = "still there", not a single
                    // error = "gone". Errors without a crash are in almost every RimWorld log,
                    // working ones too, so then the player answers (the search would otherwise
                    // end on an innocent mod).
                    let answer = if report.has_hard_crash {
                        Some(true)
                    } else if report.total_errors == 0 && report.culprits.is_empty() {
                        Some(false)
                    } else {
                        None
                    };
                    let errors = report.total_errors;
                    self.log_report = Some(report);
                    self.companion_report = crate::companion_report::load();
                    self.tick_profile = crate::tick_profile::load();
                    if self.bisect.as_ref().is_some_and(|s| s.applied && s.outcome().is_none()) {
                        let note = match answer {
                            Some(problem) => {
                                self.bisect_answer(problem);
                                let key = if problem {
                                    TranslationKey::BisectAutoYes
                                } else {
                                    TranslationKey::BisectAutoNo
                                };
                                self.i18n.get(key).to_string()
                            }
                            None => fmt(self.i18n.get(TranslationKey::BisectAutoUnsure), &[&errors.to_string()]),
                        };
                        self.bisect_note = Some(note);
                    }
                }
            }
        }
        if let Some(rx) = &self.delete_rx {
            if let Ok(done) = rx.try_recv() {
                self.delete_rx = None;
                self.status_message = done.message;
                if let Some((kind, targets)) = self.delete_job.take() {
                    self.finish_delete_job(kind, targets, done.steam_ok);
                }
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
        if let Some(rx) = &self.collection_rx {
            if let Ok(res) = rx.try_recv() {
                self.collection_rx = None;
                self.finish_collection_import(res);
            }
        }
        self.poll_batch_search();
        self.poll_name_search();
        self.poll_steamcmd();
        self.poll_workshop();
        self.poll_odd_textures();
        if let Some(ids) = self
            .repaint_ctx
            .as_ref()
            .and_then(crate::ui::components::take_workshop_request)
        {
            self.open_workshop_items(ids);
        }
        if let Some(id) = self
            .repaint_ctx
            .as_ref()
            .and_then(crate::ui::components::take_changelog_request)
        {
            self.open_changelog(&id);
        }
        self.poll_changelog();
        self.poll_last_good_diff();
        self.poll_last_good_save();
        self.poll_save_backup();
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
        for (i, m) in self.all_mods.iter().enumerate() {
            if let Some(ws_id) = m.steam_item_id.clone().or_else(|| {
                m.dir_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|n| crate::modlists::is_workshop_id(n))
                    .map(str::to_string)
            }) {
                self.mod_index.entry(format!("workshop.{ws_id}")).or_insert(i);
            }
        }
        // The other copies of a duplicated mod: a modpack may name the one that was set
        // aside ("workshop.<its item>"), it's still the same mod on disk.
        for dup in &self.duplicate_mods {
            let Some(&i) = self.mod_index.get(&dup.package_id.to_lowercase()) else {
                continue;
            };
            for (_, dir) in &dup.origins {
                if let Some(ws_id) = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|n| crate::modlists::is_workshop_id(n))
                {
                    self.mod_index.entry(format!("workshop.{ws_id}")).or_insert(i);
                }
            }
        }
    }
}

/// Screenshots for the README and the mod pages: with `RWMM_SCREENSHOT=<file.png>` the app
/// sets its window to `RWMM_WINDOW` (`1282x834`, the inside of the window), waits for the
/// scan and the icons, saves what it drew to that file and closes. The picture is the app's
/// own frame, so screen scaling and other windows don't get into it.
fn promo_screenshot(ctx: &egui::Context) {
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0: not started, 1: size set, 2: screenshot asked for.
    static STAGE: AtomicU8 = AtomicU8::new(0);
    let Some(path) = std::env::var_os("RWMM_SCREENSHOT").filter(|p| !p.is_empty()) else {
        return;
    };
    let time = ctx.input(|i| i.time);
    match STAGE.load(Ordering::Relaxed) {
        0 => {
            let size = std::env::var("RWMM_WINDOW").ok().and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some(egui::vec2(w.trim().parse().ok()?, h.trim().parse().ok()?))
            });
            if let Some(size) = size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            STAGE.store(1, Ordering::Relaxed);
        }
        // RWMM_WAIT: seconds before the shot (pages that load from the Workshop need longer).
        1 if time
            > std::env::var("RWMM_WAIT")
                .ok()
                .and_then(|w| w.parse::<f64>().ok())
                .unwrap_or(8.0) =>
        {
            STAGE.store(2, Ordering::Relaxed);
            let path = std::path::PathBuf::from(path);
            let closer = ctx.clone();
            ctx.request_screenshot(move |image| {
                let _ = image::save_buffer(
                    &path,
                    image.as_raw(),
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ExtendedColorType::Rgba8,
                );
                closer.send_viewport_cmd(egui::ViewportCommand::Close);
                closer.request_repaint();
            });
        }
        _ => {}
    }
    ctx.request_repaint();
}

/// The Workshop items asked about and what Steam returned for them.
type WorkshopItemsAnswer = (Vec<String>, Result<Vec<crate::workshop_browse::WorkshopItem>, String>);

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

/// A Workshop collection as a modpack: Core and the DLCs installed with the game go in
/// (a collection can't list them, and the game doesn't start without Core), and the whole
/// list is put in load order. Mods that aren't downloaded yet are sorted by the community
/// rules alone, so the order is worth another sort once they are on disk.
pub fn modpack_from_collection(
    entries: Vec<crate::modlists::ModListEntry>,
    all_mods: &[ModInfo],
    db: Option<&crate::community_rules::CommunityDatabase>,
) -> Vec<crate::modlists::ModListEntry> {
    let listed = |id: &str| entries.iter().any(|e| e.id.eq_ignore_ascii_case(id));
    let mut all: Vec<crate::modlists::ModListEntry> = all_mods
        .iter()
        .filter(|m| m.package_id.to_lowercase().starts_with("ludeon.rimworld"))
        .filter(|m| !listed(&m.package_id))
        .map(|m| crate::modlists::ModListEntry {
            id: m.package_id.to_lowercase(),
            name: m.name.clone(),
            steam_id: None,
        })
        .collect();
    all.extend(entries);
    let ids: Vec<String> = all.iter().map(|e| e.id.clone()).collect();
    let place: std::collections::HashMap<String, usize> = sort_load_order_with_db(&ids, all_mods, db)
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id.to_lowercase(), i))
        .collect();
    all.sort_by_key(|e| place.get(&e.id.to_lowercase()).copied().unwrap_or(usize::MAX));
    all
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
                self.i18n
                    .get(TranslationKey::CrashSummaryHardCrash)
                    .replace("{}", &report.culprits.len().to_string())
            } else if !report.culprits.is_empty() {
                self.i18n
                    .get(TranslationKey::CrashSummaryCulprits)
                    .replace("{}", &report.culprits.len().to_string())
            } else if report.total_errors > 0 {
                self.i18n
                    .get(TranslationKey::CrashSummaryErrorsOnly)
                    .replace("{}", &report.total_errors.to_string())
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
        let current = self.find_mod(pkg_id).map(|m| m.categories.clone()).unwrap_or_default();
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
        if let Err(e) =
            crate::categories::save_overrides(&crate::categories::overrides_path(), &self.category_overrides)
        {
            self.status_message = tr!(
                "Не удалось сохранить категории: {}",
                "Could not save categories: {}",
                "无法保存分类：{}",
                e
            );
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
            opt_max_size: Some(self.opt_max_size),
            check_updates: self.check_updates,
            use_steamcmd: self.use_steamcmd_set.then_some(self.use_steamcmd),
            steamcmd_dir: self.custom_steamcmd_dir.clone(),
            backup_save_before_launch: self.backup_save_before_launch,
            save_backups_to_keep: self.save_backups_to_keep,
            workshop_via_steam_client: self.workshop_via_steam_client,
            keep_mod_versions: self.keep_mod_versions,
        }
    }

    /// Saves settings when anything changed (called every frame; writes only on change).
    fn save_settings_if_changed(&mut self) {
        // Imported settings wait for the restart: this run must not write its own over them.
        if self.settings_imported {
            return;
        }
        let snap = self.settings_snapshot();
        let mut last = self.last_saved_settings.clone();
        last.launch_flags.sort();
        if snap != last {
            match snap.save_to(&crate::settings::AppSettings::path()) {
                Ok(()) => self.last_saved_settings = snap,
                Err(e) => {
                    self.status_message = tr!(
                        "Не удалось сохранить настройки: {}",
                        "Could not save settings: {}",
                        "无法保存设置：{}",
                        e
                    );
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
        // The game reads Config and Saves from its AppData folder unless told otherwise. When
        // the ModsConfig.xml chosen in Settings is elsewhere (e.g. inside the game folder),
        // point the game there, so it loads the same mod list (otherwise it ignores it).
        if let Some(dir) = self.save_data_folder() {
            if !args.iter().any(|a| a.to_lowercase().starts_with("-savedatafolder")) {
                args.push(format!("-savedatafolder={}", dir.display()));
            }
        }
        args
    }

    /// The `-savedatafolder` the game needs for the chosen ModsConfig.xml, if not the default.
    pub fn save_data_folder(&self) -> Option<PathBuf> {
        let config = self.mods_config_path.as_deref()?;
        crate::mods_config::save_data_folder_for(config, crate::mods_config::default_save_data_dir().as_deref())
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
            let _ = crate::fs_util::write_atomic(&ignore_path, content);
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
            self.status_message = tr!(
                "Сканирование модов на диске...",
                "Scanning mods on disk...",
                "正在扫描磁盘上的模组..."
            )
            .to_string();
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
            let optimizer_dds =
                crate::optimizer::count_wrong_orientation(&crate::optimizer::manifest_path(), want_flipped);
            RescanResult {
                mods: unique,
                duplicates,
                optimizer_dds,
            }
        }));
    }

    fn apply_rescan(&mut self, res: RescanResult) {
        // Overrides may have changed while scanning: apply the current ones.
        let mut mods = res.mods;
        crate::categories::assign_categories(&mut mods, &self.category_overrides);
        self.all_mods = mods;
        // Sizes and dates of the old folders: loaded again for the chosen order.
        self.mod_sizes = None;
        self.mod_sizes_rx = None;
        self.mod_added = None;
        if matches!(self.inactive_sort, InactiveSort::Size | InactiveSort::Added) {
            self.set_inactive_sort(self.inactive_sort);
        }
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
            if let Some(list) = active_after_rescan(self.has_unsaved_changes(), self.reload_active_from_disk, &on_disk)
            {
                // Read from disk, not an edit: nothing to undo.
                self.active_history.settle(&list);
                self.active_mods = list;
            }
            self.reload_active_from_disk = false;
            self.remember_saved_order(&on_disk);
        }

        self.update_companion_if_outdated();
        self.validate_deps();
        if std::mem::take(&mut self.batch_search_after_scan) {
            self.start_batch_translation_search();
        }
        if std::mem::take(&mut self.bisect_after_scan) {
            self.open_bisect();
        }
        if std::mem::take(&mut self.updates_after_scan) {
            self.check_mod_updates();
        }
        if !self.initial_log_checked && self.log_report.is_none() && self.log_rx.is_none() {
            self.initial_log_checked = true;
            let log_target = self
                .custom_log_path
                .clone()
                .or_else(|| detect_rimworld_log_paths().into_iter().next().map(|(_, p)| p));
            if let Some(path) = log_target {
                let mods = self.all_mods.clone();
                self.log_silent = true;
                self.log_rx = Some(self.spawn_task(move || analyze_rimworld_log(&path, &mods)));
            }
        }
        self.name_search_when_ready();
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
        self.trigger_vram_check();
        self.refresh_last_good_diff();
    }

    pub fn sort_active_mods(&mut self) {
        self.active_mods = sort_load_order_with_db(&self.active_mods, &self.all_mods, Some(&self.community_db));
        self.dep_alert_ignored.clear();
        self.validate_deps();
        self.status_message = self.i18n.get(TranslationKey::AppSortedMsg).to_string();
    }

    /// "Ignore" on the dependency alert: hides it for its current problems.
    pub fn ignore_dependency_alert(&mut self) {
        self.dep_alert_ignored = self.dependency_report.problems.clone();
    }

    /// The dependency alert is shown: there are problems, and not only ignored ones.
    pub fn dependency_alert_shown(&self) -> bool {
        !self.dependency_report.is_empty()
            && !self
                .dependency_report
                .problems
                .iter()
                .all(|p| self.dep_alert_ignored.contains(p))
    }

    /// Name of an installed mod, or the ID as given.
    fn mod_name_or_id(&self, id: &str) -> String {
        lookup_mod(&self.mod_index, &self.all_mods, id).map_or_else(|| id.to_string(), |m| m.name.clone())
    }

    /// "Remember this place in the load order": own rules that make every sort leave the
    /// active mod `id` where it stands now (`user_order::pin_place`).
    pub fn pin_mod_place(&mut self, id: &str) {
        use crate::user_order::PinOutcome;
        let name = self.mod_name_or_id(id);
        let t = self.i18n;
        let outcome = crate::user_order::pin_place(&self.active_mods, &self.all_mods, &mut self.community_db, id);
        self.status_message = match outcome {
            PinOutcome::AlreadyThere => fmt(t.get(TranslationKey::OrderPinAlreadyThere), &[&name]),
            PinOutcome::Added(_) => {
                if !self.save_order_rules() {
                    return;
                }
                fmt(
                    t.get(TranslationKey::OrderPinned),
                    &[&name, &self.community_db.user_order.len().to_string()],
                )
            }
            PinOutcome::Blocked { blocker } => fmt(
                t.get(TranslationKey::OrderPinBlocked),
                &[&name, &self.mod_name_or_id(&blocker)],
            ),
            PinOutcome::TooFar => fmt(t.get(TranslationKey::OrderPinTooFar), &[&name]),
        };
    }

    /// Forgets every own order rule that names `id`.
    pub fn forget_order_rules_of(&mut self, id: &str) {
        let removed = crate::user_order::remove_for(&mut self.community_db.user_order, id);
        if removed > 0 && self.save_order_rules() {
            self.status_message = fmt(
                self.i18n.get(TranslationKey::OrderForgot),
                &[&self.mod_name_or_id(id), &removed.to_string()],
            );
        }
    }

    /// Removes one own order rule (the list in Settings).
    pub fn remove_order_rule(&mut self, index: usize) {
        if index < self.community_db.user_order.len() {
            self.community_db.user_order.remove(index);
            self.save_order_rules();
        }
    }

    /// Removes every own order rule (the button at the bottom of their card in Settings).
    pub fn reset_order_rules(&mut self) {
        let removed = self.community_db.user_order.len();
        self.community_db.user_order.clear();
        if self.save_order_rules() {
            self.status_message = fmt(self.i18n.get(TranslationKey::OrderResetDone), &[&removed.to_string()]);
        }
    }

    /// Writes the own order rules; on failure says so in the status line.
    fn save_order_rules(&mut self) -> bool {
        match crate::user_order::save(&crate::user_order::rules_path(), &self.community_db.user_order) {
            Ok(()) => true,
            Err(e) => {
                self.status_message = fmt(self.i18n.get(TranslationKey::OrderSaveFailed), &[&e.to_string()]);
                false
            }
        }
    }

    /// Writes the load order to ModsConfig.xml. Returns whether it was saved.
    pub fn save_config(&mut self) -> bool {
        let Some(config_p) = self.mods_config_path.clone() else {
            self.status_message = tr!(
                "ModsConfig.xml не найден — укажите его в Настройках",
                "ModsConfig.xml not found — set it in Settings",
                "找不到 ModsConfig.xml — 请在设置中指定"
            )
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
                        "ModsConfig.xml saved, but RimWorld is running: the order applies after a game restart, and changing mods inside the game will overwrite this file", "ModsConfig.xml 已保存，但 RimWorld 正在运行：排序会在重启游戏后生效，在游戏内修改模组会覆盖此文件"
                    )
                    .to_string()
                } else {
                    tr!(
                        "ModsConfig.xml сохранён",
                        "ModsConfig.xml saved",
                        "ModsConfig.xml 已保存"
                    )
                    .to_string()
                };
                true
            }
            Err(e) => {
                self.status_message = tr!(
                    "Ошибка сохранения ModsConfig.xml: {}",
                    "Could not save ModsConfig.xml: {}",
                    "无法保存 ModsConfig.xml：{}",
                    e
                );
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
            Some(DropTarget::Active(to)) => {
                move_mods(&mut self.active_mods, &drag.ids, to);
                if !drag.from_active {
                    self.warn_no_version(&drag.ids);
                }
            }
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
        let kept = crate::modlist_settings::root();
        self.modlist_settings = self
            .modlists
            .iter()
            .filter_map(|l| crate::modlist_settings::saved(&kept, &l.name).map(|s| (l.name.clone(), s)))
            .collect();
    }

    // ----- Mod settings kept with a modpack (`modlist_settings.rs`) -----

    /// The game's Config folder: ModsConfig.xml and the mods' `Mod_*.xml` settings.
    fn game_config_dir(&self) -> Option<PathBuf> {
        self.mods_config_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// Takes a copy of the mods' settings for the modpack (a new one replaces the old).
    pub fn save_modlist_settings(&mut self, name: &str) {
        let t = self.i18n;
        let Some(config) = self.game_config_dir() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        self.status_message = match crate::modlist_settings::save(&crate::modlist_settings::root(), name, &config) {
            Ok(n) => fmt(t.get(TranslationKey::SetsSettingsSavedMsg), &[name, &n.to_string()]),
            Err(e) => fmt(t.get(TranslationKey::SetsSettingsError), &[&e]),
        };
        self.refresh_modlists();
    }

    /// Puts the modpack's copy of the mods' settings back; says what happened.
    fn restore_modlist_settings_text(&self, name: &str) -> Option<String> {
        let t = self.i18n;
        let config = self.game_config_dir()?;
        let backup =
            crate::modlist_settings::backup_root().join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
        Some(
            match crate::modlist_settings::restore(&crate::modlist_settings::root(), name, &config, &backup) {
                Ok(done) if done.written == 0 => t.get(TranslationKey::SetsSettingsSameMsg).to_string(),
                Ok(done) => {
                    let mut msg = fmt(
                        t.get(TranslationKey::SetsSettingsRestoredMsg),
                        &[&done.written.to_string()],
                    );
                    if done.backed_up > 0 {
                        msg.push_str(" | ");
                        msg.push_str(&fmt(
                            t.get(TranslationKey::SetsZipBackup),
                            &[&done.backed_up.to_string(), &backup.display().to_string()],
                        ));
                    }
                    msg
                }
                Err(e) => fmt(t.get(TranslationKey::SetsSettingsError), &[&e]),
            },
        )
    }

    pub fn restore_modlist_settings(&mut self, name: &str) {
        if let Some(msg) = self.restore_modlist_settings_text(name) {
            self.status_message = msg;
        }
    }

    pub fn forget_modlist_settings(&mut self, name: &str) {
        crate::modlist_settings::remove(&crate::modlist_settings::root(), name);
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsSettingsForgotMsg), &[name]);
        self.refresh_modlists();
    }

    /// Enabling a modpack with kept settings: the modpacks the game ran until now (`live`)
    /// get a fresh copy of the settings in use, so nothing set while playing them is lost,
    /// and the new modpack's copy is put back. Returns what to tell the player.
    fn switch_modlist_settings(&mut self, live: &[String], name: &str) -> Option<String> {
        self.modlist_settings.get(name)?;
        if let Some(config) = self.game_config_dir() {
            let kept = crate::modlist_settings::root();
            for other in live.iter().filter(|l| self.modlist_settings.contains_key(*l)) {
                let _ = crate::modlist_settings::save(&kept, other, &config);
            }
        }
        let msg = self.restore_modlist_settings_text(name);
        self.refresh_modlists();
        msg
    }

    // ----- Comparing a modpack with another list -----

    /// IDs and names of the list a modpack is compared with.
    fn compare_side(&self, with: &CompareWith) -> Option<(Vec<String>, Vec<String>)> {
        match with {
            CompareWith::Current => Some((self.active_mods.clone(), Vec::new())),
            CompareWith::Modpack(name) => self
                .modlists
                .iter()
                .find(|l| &l.name == name)
                .map(|l| (l.ids.clone(), l.names.clone())),
            CompareWith::Save(name) => self
                .saves
                .iter()
                .find(|s| &s.name == name)
                .map(|s| (s.ids.clone(), s.names.clone())),
        }
    }

    /// Opens the "Compare" window (or changes what the modpack is compared with).
    pub fn open_set_compare(&mut self, name: &str, with: CompareWith) {
        let Some(list) = self.modlists.iter().find(|l| l.name == name) else {
            return;
        };
        let Some((ids, their_names)) = self.compare_side(&with) else {
            return;
        };
        let mut names = HashMap::new();
        for (ids, known) in [(&list.ids, &list.names), (&ids, &their_names)] {
            for (i, id) in ids.iter().enumerate() {
                let name = self
                    .find_mod(id)
                    .map(|m| m.name.clone())
                    .or_else(|| known.get(i).filter(|n| !n.trim().is_empty()).cloned())
                    .unwrap_or_else(|| id.clone());
                names.entry(id.to_lowercase()).or_insert(name);
            }
        }
        self.set_compare = Some(SetCompare {
            first: name.to_string(),
            result: crate::modlists::compare(&list.ids, &ids),
            with,
            names,
        });
    }

    // ----- Change notes of a Workshop mod (`changelog.rs`) -----

    /// Steam's records of installed Workshop items: the Steam client's and SteamCMD's.
    fn workshop_records(&self) -> Vec<PathBuf> {
        let mut acfs = Vec::new();
        if let Some(p) = self
            .steam_paths
            .workshop_path
            .as_deref()
            .and_then(crate::mod_updates::acf_path)
        {
            acfs.push(p);
        }
        let steamcmd_acf = self
            .steamcmd_dir()
            .join("steamapps")
            .join("workshop")
            .join("appworkshop_294100.acf");
        if steamcmd_acf.is_file() {
            acfs.push(steamcmd_acf);
        }
        acfs
    }

    /// Opens the "What changed" window for a Workshop mod and downloads its change notes.
    pub fn open_changelog(&mut self, workshop_id: &str) {
        let installed_mod = self
            .all_mods
            .iter()
            .find(|m| {
                workshop_id_of(m).as_deref() == Some(workshop_id)
                    || (m.frozen
                        && crate::freeze::read(&m.dir_path)
                            .is_some_and(|f| f.workshop_id.as_deref() == Some(workshop_id)))
            })
            .cloned();
        let title = installed_mod
            .as_ref()
            .map(|m| m.name.clone())
            .or_else(|| {
                self.ws_items
                    .iter()
                    .find(|i| i.id == workshop_id)
                    .map(|i| i.title.clone())
            })
            .unwrap_or_else(|| format!("#{workshop_id}"));
        self.changelog = Some(ChangelogWindow {
            workshop_id: workshop_id.to_string(),
            title,
            installed: None,
            notes: None,
        });
        let id = workshop_id.to_string();
        let records = self.workshop_records();
        self.changelog_rx = Some(self.spawn_task(move || {
            // A frozen copy is as old as its freeze; otherwise Steam's record knows.
            let installed = installed_mod.and_then(|m| {
                if m.frozen {
                    return crate::freeze::read(&m.dir_path).map(|f| f.frozen_at);
                }
                records
                    .iter()
                    .filter_map(|p| std::fs::read_to_string(p).ok())
                    .filter_map(|text| crate::mod_updates::parse_acf_installed(&text).get(&id).copied())
                    .max()
            });
            let notes = crate::changelog::fetch(&id);
            (id, installed, notes)
        }));
    }

    fn poll_changelog(&mut self) {
        let Some(rx) = &self.changelog_rx else { return };
        let Ok((id, installed, notes)) = rx.try_recv() else {
            return;
        };
        self.changelog_rx = None;
        if let Some(window) = self.changelog.as_mut().filter(|w| w.workshop_id == id) {
            window.installed = installed;
            window.notes = Some(notes);
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
            .enumerate()
            .filter(|(_, id)| self.find_mod(id).is_none())
            .filter_map(|(i, id)| self.workshop_id_for(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
            .filter(|ws| !self.workshop_item_gone(ws))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Asks Steam (in the background, once per item per run) about Workshop items not on disk:
    /// the answer tells which are removed or hidden, so they aren't offered for download.
    pub fn check_workshop_items(&mut self, ids: &[String]) {
        if self.ws_item_rx.is_some() {
            return;
        }
        let new: Vec<String> = ids
            .iter()
            .filter(|id| crate::modlists::is_workshop_id(id) && !self.ws_item_checked.contains(*id))
            .cloned()
            .collect();
        if new.is_empty() {
            return;
        }
        self.ws_item_checked.extend(new.iter().cloned());
        let asked = new.clone();
        self.ws_item_rx = Some(self.spawn_task(move || (asked, crate::workshop_browse::fetch_items(&new))));
    }

    /// The item was asked about and Steam returned nothing: removed by its author or hidden.
    pub fn workshop_item_gone(&self, id: &str) -> bool {
        self.ws_item_checked.contains(id) && self.ws_item_rx.is_none() && !self.ws_item_titles.contains_key(id)
    }

    /// D1: a mod's Workshop ID: the one its file remembers, else from the Workshop ID index
    /// (the mod's name tells re-uploads apart).
    pub fn workshop_id_for(&self, package_id: &str, known: Option<&str>, name_hint: Option<&str>) -> Option<String> {
        // "workshop.<ID>" (a collection's mod that wasn't on disk yet) carries its ID itself.
        let own = package_id
            .strip_prefix("workshop.")
            .filter(|id| crate::modlists::is_workshop_id(id));
        known.or(own).map(str::to_string).or_else(|| {
            self.workshop_index
                .as_ref()?
                .lookup(package_id, name_hint)
                .map(str::to_string)
        })
    }

    /// Downloads RimSort's Workshop database (~47 MB) in the background and keeps an index.
    pub fn download_workshop_index(&mut self) {
        if self.workshop_index_rx.is_some() {
            return;
        }
        self.status_message = self.i18n.get(TranslationKey::WorkshopDbDownloading).to_string();
        self.workshop_index_rx = Some(self.spawn_task(|| (true, crate::workshop_db::download())));
    }

    /// The index is being read or downloaded.
    pub fn workshop_index_busy(&self) -> bool {
        self.workshop_index_rx.is_some()
    }

    /// F6: subscribes to every missing mod of the set that has a Workshop ID, in one
    /// Steam session in the background. Steam downloads them; then the user rescans.
    pub fn subscribe_modlist_missing(&mut self, name: &str) {
        if self.steam_rx.is_some() {
            return;
        }
        let Some(list) = self.modlists.iter().find(|l| l.name == name) else {
            return;
        };
        let ids = self.modlist_missing_workshop_ids(list);
        self.subscribe_workshop_ids(ids);
    }

    /// Subscribes to Workshop items in one Steam session in the background.
    fn subscribe_workshop_ids(&mut self, ids: Vec<String>) {
        self.download_workshop_ids(ids, false);
    }

    /// Downloads Workshop items: SteamCMD when it is on, else the Steam client.
    /// `reinstall`: the old files were just removed, so Steam is asked to download again.
    fn download_workshop_ids(&mut self, ids: Vec<String>, reinstall: bool) {
        if self.steam_busy() || ids.is_empty() {
            return;
        }
        if self.steamcmd_active() {
            self.start_steamcmd_job(ids);
            return;
        }
        let t = self.i18n;
        let Some(dll) = self.steam_paths.steam_dll_path.clone() else {
            self.status_message = t.get(TranslationKey::SetsNoSteamDll).to_string();
            return;
        };
        self.status_message = fmt(t.get(TranslationKey::SetsSubscribing), &[&ids.len().to_string()]);
        self.start_steam_client_download(dll, ids, reinstall);
    }

    fn start_steam_client_download(&mut self, dll: PathBuf, ids: Vec<String>, force_download: bool) {
        let t = self.i18n;
        // A flag of its own: a wait stopped just before keeps its "stop" even if it has not
        // looked at it yet.
        self.steam_stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag = self.steam_stop_flag.clone();
        let (prog_tx, prog_rx) = channel();
        self.steam_progress = Some(crate::steam::SteamDownloadProgress {
            installed: 0,
            total: ids.len(),
            bytes_done: 0,
            bytes_total: 0,
            active: true,
        });
        self.steam_progress_rx = Some(prog_rx);
        let ctx = self.repaint_ctx.clone();
        self.steam_rx = Some(self.spawn_task(move || {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let mut on_prog = |prog: crate::steam::SteamDownloadProgress| {
                let _ = prog_tx.send(prog);
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            };
            match crate::steam::subscribe_workshop_items_with_progress(
                &dll,
                &refs,
                Some(stop_flag),
                force_download,
                &mut on_prog,
            ) {
                Ok((_accepted, installed)) if installed > 0 => fmt(
                    t.get(TranslationKey::SteamDownloadFinished),
                    &[&installed.to_string(), &ids.len().to_string()],
                ),
                Ok((accepted, _)) => fmt(t.get(TranslationKey::SetsSubscribed), &[&accepted.to_string()]),
                Err(e) => fmt(t.get(TranslationKey::SetsSubscribeError), &[&e]),
            }
        }));
    }

    /// S1: Stops waiting for Steam client downloads to finish, releasing the Steam API session.
    pub fn stop_steam_wait(&mut self) {
        self.steam_stop_flag.store(true, Ordering::Relaxed);
        self.steam_rx = None;
        self.steam_progress_rx = None;
        self.steam_progress = None;
        self.status_message = self.i18n.get(TranslationKey::SteamStoppedWaiting).to_string();
        self.rescan_mods_keep_status();
    }

    // ----- SteamCMD -----

    /// The SteamCMD folder: picked in Settings, else next to the settings.
    pub fn steamcmd_dir(&self) -> PathBuf {
        self.custom_steamcmd_dir
            .clone()
            .unwrap_or_else(crate::steamcmd::default_dir)
    }

    /// Mods are downloaded with SteamCMD: turned on in Settings, or there is no Steam client.
    pub fn steamcmd_active(&self) -> bool {
        self.use_steamcmd || self.steam_paths.steam_path.is_none()
    }

    pub fn steamcmd_busy(&self) -> bool {
        self.steamcmd_rx.is_some()
    }

    /// The game's Mods folder, where links to downloaded mods go.
    pub fn game_mods_dir(&self) -> Option<PathBuf> {
        self.steam_paths
            .local_mods_path
            .clone()
            .or_else(|| self.steam_paths.game_path.as_ref().map(|g| g.join("Mods")))
    }

    // ----- Companion mod (F11) -----

    /// The companion mod in the game's Mods folder (`None`: the Mods folder is unknown).
    pub fn companion_state(&self) -> Option<crate::companion::State> {
        self.game_mods_dir().map(|d| crate::companion::state(&d))
    }

    /// Asks the companion to measure the mods' load on the next launch of the game. The
    /// companion in the game must be the one this version carries: an older one doesn't
    /// know the request.
    pub fn request_tick_profile(&mut self) {
        let t = self.i18n;
        if self.companion_state() != Some(crate::companion::State::Current) {
            self.install_companion();
            if self.companion_state() != Some(crate::companion::State::Current) {
                // `install_companion` said why (no Mods folder, or the game is running).
                return;
            }
        }
        self.status_message =
            match crate::tick_profile::request_at(&crate::tick_profile::request_path(), crate::tick_profile::TICKS) {
                Ok(()) => t.get(TranslationKey::TickProfileRequested).to_string(),
                Err(e) => fmt(t.get(TranslationKey::TickProfileError), &[&e.to_string()]),
            };
    }

    /// Puts the companion mod into the game's Mods folder and turns it on (at the end of the
    /// list). Saved right away when the list had no unsaved changes of its own.
    pub fn install_companion(&mut self) {
        let t = self.i18n;
        let Some(mods_dir) = self.game_mods_dir() else {
            self.status_message = t.get(TranslationKey::SteamCmdNoMods).to_string();
            return;
        };
        if crate::game_install::is_game_running() {
            self.status_message = t.get(TranslationKey::CompanionCloseGame).to_string();
            return;
        }
        if let Err(e) = crate::companion::install(&mods_dir) {
            self.status_message = fmt(t.get(TranslationKey::CompanionError), &[&e.to_string()]);
            return;
        }
        let had_unsaved = self.has_unsaved_changes();
        add_active_id(&mut self.active_mods, crate::companion::PACKAGE_ID);
        if had_unsaved {
            self.status_message = t.get(TranslationKey::CompanionInstalledUnsaved).to_string();
        } else if self.save_config() {
            self.status_message = t.get(TranslationKey::CompanionInstalled).to_string();
        }
        self.rescan_mods_keep_status();
    }

    /// Turns the companion mod off and moves it to the Recycle Bin.
    pub fn remove_companion(&mut self) {
        let t = self.i18n;
        let Some(mods_dir) = self.game_mods_dir() else { return };
        if crate::game_install::is_game_running() {
            self.status_message = t.get(TranslationKey::CompanionCloseGame).to_string();
            return;
        }
        let had_unsaved = self.has_unsaved_changes();
        self.active_mods
            .retain(|id| !id.eq_ignore_ascii_case(crate::companion::PACKAGE_ID));
        if !had_unsaved {
            self.save_config();
        }
        self.status_message = match crate::companion::remove(&mods_dir) {
            Ok(()) => t.get(TranslationKey::CompanionRemoved).to_string(),
            Err(e) => fmt(t.get(TranslationKey::CompanionError), &[&e]),
        };
        self.rescan_mods_keep_status();
    }

    /// A newer companion mod comes with the manager: update the installed one (not while the
    /// game runs, it keeps the dll open).
    fn update_companion_if_outdated(&mut self) {
        let Some(mods_dir) = self.game_mods_dir() else { return };
        if crate::companion::state(&mods_dir) == crate::companion::State::Outdated
            && !crate::game_install::is_game_running()
        {
            let _ = crate::companion::install(&mods_dir);
        }
    }

    /// Installs or updates SteamCMD (nothing to download).
    pub fn install_steamcmd(&mut self) {
        self.start_steamcmd_job(Vec::new());
    }

    /// Downloads Workshop items with SteamCMD in the background (installing it first if
    /// needed) and opens its window; the mods get links in the game's Mods folder.
    pub fn start_steamcmd_job(&mut self, mut ids: Vec<String>) {
        if self.steamcmd_busy() {
            self.steamcmd_window = true;
            return;
        }
        ids.sort();
        ids.dedup();
        let t = self.i18n;
        let mods_dir = self.game_mods_dir();
        if !ids.is_empty() && mods_dir.is_none() {
            self.status_message = t.get(TranslationKey::SteamCmdNoMods).to_string();
            return;
        }
        let dir = self.steamcmd_dir();
        self.steamcmd_fresh_install = !crate::steamcmd::is_installed(&dir);
        self.steamcmd_job = Some(crate::steamcmd::Job::new(&ids));
        self.steamcmd_window = true;
        self.steamcmd_console = false;
        self.status_message = if ids.is_empty() {
            t.get(TranslationKey::SteamCmdInstalling).to_string()
        } else {
            fmt(t.get(TranslationKey::SteamCmdDownloading), &[&ids.len().to_string()])
        };
        let (tx, rx) = channel();
        self.steamcmd_rx = Some(rx);
        let kill = self.steamcmd_kill.clone();
        let ctx = self.repaint_ctx.clone();
        std::thread::spawn(move || {
            let repaint = || {
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            };
            crate::steamcmd::run_job(&dir, &ids, mods_dir.as_deref(), &kill, &tx, &repaint);
        });
    }

    pub fn stop_steamcmd(&mut self) {
        crate::steamcmd::stop(&self.steamcmd_kill);
    }

    /// Opens the SteamCMD window on its console.
    pub fn open_steamcmd_console(&mut self) {
        self.steamcmd_console = true;
        self.steamcmd_window = true;
    }

    /// Runs the console's commands (after an anonymous login) in the background.
    pub fn run_steamcmd_console(&mut self) {
        if self.steamcmd_busy() || self.steamcmd_console_input.trim().is_empty() {
            return;
        }
        let (args, dropped_login) = crate::steamcmd::console_args(&self.steamcmd_console_input);
        let mut job = crate::steamcmd::Job::new(&[]);
        if dropped_login {
            job.apply(crate::steamcmd::Msg::Line(
                self.i18n.get(TranslationKey::SteamCmdConsoleLogin).to_string(),
            ));
        }
        self.steamcmd_job = Some(job);
        self.steamcmd_console = true;
        self.steamcmd_window = true;
        let dir = self.steamcmd_dir();
        let (tx, rx) = channel();
        self.steamcmd_rx = Some(rx);
        let kill = self.steamcmd_kill.clone();
        let ctx = self.repaint_ctx.clone();
        std::thread::spawn(move || {
            let repaint = || {
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            };
            crate::steamcmd::run_console(&dir, &args, &kill, &tx, &repaint);
        });
    }

    // ----- Workshop tab -----

    /// Loads the first page the first time the tab is shown.
    pub fn ensure_workshop_loaded(&mut self) {
        if !self.ws_loaded {
            self.ws_loaded = true;
            self.browse_workshop();
        }
    }

    /// Loads `ws_query`'s page in the background.
    pub fn browse_workshop(&mut self) {
        if self.ws_rx.is_some() {
            return;
        }
        self.ws_error = None;
        let query = self.ws_query.clone();
        let route = self.workshop_route();
        self.ws_rx = Some(self.spawn_task(move || crate::workshop_browse::load_page(&query, &route)));
    }

    /// With the setting on, the Steam client is the other way to the Workshop (the game's
    /// `steam_api64.dll` is needed for it), and goes first once the website has refused.
    fn workshop_route(&self) -> crate::workshop_browse::Route {
        crate::workshop_browse::Route {
            steam_dll: self
                .workshop_via_steam_client
                .then(|| self.steam_paths.steam_dll_path.clone())
                .flatten(),
            prefer_steam: self.ws_source == crate::workshop_browse::Source::SteamClient,
        }
    }

    pub fn workshop_loading(&self) -> bool {
        self.ws_rx.is_some()
    }

    /// Opens the Workshop tab searching for a mod's name (version tags dropped). The tab
    /// opens even when it's hidden in Settings.
    pub fn open_workshop_search(&mut self, name: &str) {
        self.ws_search = crate::name_search::clean_name(name);
        self.ws_query.section = crate::workshop_browse::Section::Mods;
        self.ws_selected = None;
        self.ws_loaded = true;
        self.current_tab = AppTab::Workshop;
        // A page still loading would be replaced by the search right after.
        self.ws_rx = None;
        self.search_workshop_tab();
    }

    /// For a log with a load profile from 1.7: counts each profiled mod's textures with a side
    /// that isn't a power of two, in the background (cached per mod, so only the first time
    /// reads every texture header).
    fn poll_odd_textures(&mut self) {
        if let Some(rx) = &self.odd_textures_rx {
            if let Ok((counts, fresh)) = rx.try_recv() {
                self.odd_textures_rx = None;
                let cache = self.texture_sides_cache.get_or_insert_with(HashMap::new);
                if !fresh.is_empty() {
                    for (dir, fp, odd, all) in fresh {
                        cache.insert(dir, (fp, odd, all));
                    }
                    let _ = crate::optimizer::write_texture_sides_cache(
                        &crate::optimizer::texture_sides_cache_path(),
                        cache,
                    );
                }
                self.odd_textures = counts.into_iter().map(|(id, c)| (id.to_lowercase(), c)).collect();
            }
            return;
        }
        let Some(report) = self.log_report.as_ref().filter(|r| r.is_waylith) else {
            return;
        };
        let Some(profile) = &report.load_profile else {
            return;
        };
        let key = (report.log_path.clone(), report.file_size_bytes);
        if self.odd_textures_for.as_ref() == Some(&key) {
            return;
        }
        let mods: Vec<(String, PathBuf)> = profile
            .mods
            .iter()
            .filter_map(|p| self.find_mod(&p.id))
            .filter(|m| !self.is_official_mod(m))
            .map(|m| (m.package_id.clone(), m.dir_path.clone()))
            .collect();
        self.odd_textures_for = Some(key);
        let cache = self
            .texture_sides_cache
            .get_or_insert_with(|| {
                crate::optimizer::read_texture_sides_cache(&crate::optimizer::texture_sides_cache_path())
            })
            .clone();
        let version = self.steam_paths.game_version.clone();
        self.odd_textures_rx =
            Some(self.spawn_task(move || crate::optimizer::non_power_of_two_by_mod(&mods, version.as_deref(), &cache)));
    }

    pub fn odd_textures_counting(&self) -> bool {
        self.odd_textures_rx.is_some()
    }

    /// "Measure and play": the next launch runs with verbose logging (see `measure_next_launch`).
    pub fn launch_with_load_measure(&mut self) {
        self.measure_next_launch = true;
        self.launch_game();
    }

    /// Opens mods in the Workshop tab instead of the browser: the grid shows just them, the
    /// first one's card on the right ("Open the Steam page" is still there). The tab opens
    /// even when hidden.
    pub fn open_workshop_items(&mut self, item_ids: Vec<String>) {
        let Some(first) = item_ids.first() else {
            return;
        };
        self.ws_selected = Some(first.clone());
        self.ws_loaded = true;
        self.ws_error = None;
        self.ws_query.section = crate::workshop_browse::Section::Mods;
        self.current_tab = AppTab::Workshop;
        let route = self.workshop_route();
        self.ws_rx = Some(self.spawn_task(move || crate::workshop_browse::load_items(&item_ids, &route)));
    }

    /// "Mods" / "Collections": page 1 of the other section (a page still loading is dropped).
    pub fn set_workshop_section(&mut self, section: crate::workshop_browse::Section) {
        if self.ws_query.section == section {
            return;
        }
        self.ws_query.section = section;
        self.ws_selected = None;
        self.ws_items.clear();
        self.ws_rx = None;
        self.search_workshop_tab();
    }

    /// Loads the picked collection's mods once (in the background).
    pub fn ensure_collection_mods(&mut self) {
        let Some(item) = self
            .ws_selected
            .as_ref()
            .and_then(|id| self.ws_items.iter().find(|i| &i.id == id))
            .filter(|i| i.is_collection)
        else {
            return;
        };
        let loaded = self.ws_collection.as_ref().is_some_and(|(id, _)| id == &item.id);
        if loaded || self.ws_collection_rx.is_some() {
            return;
        }
        let (id, children) = (item.id.clone(), item.children.clone());
        let route = self.workshop_route();
        self.ws_collection_rx =
            Some(self.spawn_task(move || (id, crate::workshop_browse::load_collection_mods(&children, &route))));
    }

    /// "Save as a modpack" on the Workshop tab: the collection becomes a new modpack.
    pub fn import_workshop_collection(&mut self, id: &str) {
        if let Err(e) = self.import_steam_collection(id) {
            self.status_message = e;
        }
    }

    /// A new search or sort: back to page 1.
    pub fn search_workshop_tab(&mut self) {
        self.ws_query.text = self.ws_search.trim().to_string();
        self.ws_query.page = 1;
        self.browse_workshop();
    }

    pub fn workshop_page(&mut self, page: u32) {
        if self.ws_rx.is_some() {
            return;
        }
        self.ws_query.page = page.max(1);
        self.browse_workshop();
    }

    fn poll_workshop(&mut self) {
        if let Some(rx) = &self.ws_rx {
            if let Ok(result) = rx.try_recv() {
                self.ws_rx = None;
                match result {
                    Ok(loaded) => {
                        let items = loaded.items;
                        if !self
                            .ws_selected
                            .as_ref()
                            .is_some_and(|s| items.iter().any(|i| &i.id == s))
                        {
                            self.ws_selected = None;
                        }
                        self.ws_items = items;
                        self.ws_source = loaded.source;
                    }
                    Err(e) => {
                        // The site refused and the other way is turned off: say where it is.
                        let hint = (!self.workshop_via_steam_client && self.steam_paths.steam_dll_path.is_some())
                            .then(|| format!("\n{}", crate::i18n::text(crate::i18n::TranslationKey::WsSteamClientOff)));
                        self.ws_error = Some(e + hint.as_deref().unwrap_or(""));
                    }
                }
            }
        }
        if let Some(rx) = &self.ws_collection_rx {
            if let Ok(result) = rx.try_recv() {
                self.ws_collection_rx = None;
                self.ws_collection = Some(result);
            }
        }
        if let Some(rx) = &self.ws_preview_rx {
            let ctx = self.repaint_ctx.clone();
            while let Ok((key, image)) = rx.try_recv() {
                let slot = match (image, &ctx) {
                    (Some(image), Some(ctx)) => {
                        PreviewSlot::Ready(ctx.load_texture(&key, image, egui::TextureOptions::LINEAR))
                    }
                    _ => PreviewSlot::Failed,
                };
                self.ws_previews.insert(key, slot);
            }
        }
    }

    /// A Workshop preview at `width` px: the texture once it's downloaded (four at a time,
    /// in the background); None while loading or when there's none.
    pub fn workshop_preview(&mut self, url: &str, width: u32) -> Option<egui::TextureHandle> {
        let key = format!("{width}:{url}");
        if let Some(slot) = self.ws_previews.get(&key) {
            return match slot {
                PreviewSlot::Ready(t) => Some(t.clone()),
                _ => None,
            };
        }
        // Many pages browsed: the oldest textures go (they load again if needed).
        if self.ws_previews.len() > 400 {
            self.ws_previews.retain(|_, s| matches!(s, PreviewSlot::Loading));
        }
        if self.ws_preview_tx.is_none() {
            self.start_preview_workers();
        }
        if let Some(tx) = &self.ws_preview_tx {
            let _ = tx.send((key.clone(), url.to_string(), width));
            self.ws_previews.insert(key, PreviewSlot::Loading);
        }
        None
    }

    fn start_preview_workers(&mut self) {
        let (req_tx, req_rx) = channel::<(String, String, u32)>();
        let (res_tx, res_rx) = channel();
        let req_rx = Arc::new(std::sync::Mutex::new(req_rx));
        for _ in 0..4 {
            let (req_rx, res_tx, ctx) = (req_rx.clone(), res_tx.clone(), self.repaint_ctx.clone());
            std::thread::spawn(move || loop {
                let next = req_rx.lock().ok().and_then(|rx| rx.recv().ok());
                let Some((key, url, width)) = next else { break };
                let image =
                    crate::workshop_browse::load_preview(&crate::workshop_browse::sized_preview(&url, width), width)
                        .map(|rgba| {
                            egui::ColorImage::from_rgba_unmultiplied(
                                [rgba.width() as usize, rgba.height() as usize],
                                rgba.as_raw(),
                            )
                        });
                if res_tx.send((key, image)).is_err() {
                    break;
                }
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            });
        }
        self.ws_preview_tx = Some(req_tx);
        self.ws_preview_rx = Some(res_rx);
    }

    /// Workshop IDs on disk: Workshop mods, and folders named by a Workshop ID in the game's
    /// Mods folder (SteamCMD links, copies on a game without Steam).
    pub fn installed_workshop_ids(&self) -> std::collections::HashSet<String> {
        self.all_mods
            .iter()
            .filter_map(|m| {
                m.steam_item_id.clone().or_else(|| {
                    m.dir_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .filter(|n| crate::modlists::is_workshop_id(n))
                        .map(str::to_string)
                })
            })
            .collect()
    }

    /// "Download" on the Workshop tab: Steam subscription or SteamCMD, as set up.
    pub fn download_workshop_item(&mut self, id: &str) {
        if self.steam_busy() {
            return;
        }
        self.ws_requested.insert(id.to_string());
        self.subscribe_steam_mod(id);
    }

    /// W1: downloads the Workshop item together with its missing dependencies.
    pub fn download_workshop_items(&mut self, ids: Vec<String>) {
        if self.steam_busy() || ids.is_empty() {
            return;
        }
        for id in &ids {
            self.ws_requested.insert(id.clone());
        }
        if ids.len() == 1 {
            self.subscribe_steam_mod(&ids[0]);
        } else {
            self.subscribe_workshop_ids(ids);
        }
    }

    fn poll_steamcmd(&mut self) {
        let Some(rx) = &self.steamcmd_rx else { return };
        let Some(job) = self.steamcmd_job.as_mut() else { return };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    job.apply(msg);
                    finished |= job.phase.is_finished();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            return;
        }
        let t = self.i18n;
        let (done, failed) = (job.finished_items(), job.failed_items());
        let status = match &job.phase {
            crate::steamcmd::Phase::Failed(e) => fmt(t.get(TranslationKey::SteamCmdFailed), &[e]),
            crate::steamcmd::Phase::Stopped => t.get(TranslationKey::SteamCmdStopped).to_string(),
            _ if job.items.is_empty() => t.get(TranslationKey::SteamCmdReady).to_string(),
            _ => fmt(
                t.get(TranslationKey::SteamCmdDone),
                &[&done.to_string(), &failed.to_string()],
            ),
        };
        let done_ids: std::collections::HashSet<String> = job
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.state,
                    crate::steamcmd::ItemState::Linked | crate::steamcmd::ItemState::OwnCopy
                )
            })
            .map(|i| i.id.clone())
            .collect();
        self.steamcmd_rx = None;
        self.status_message = status;
        // Just installed: from now on every "Download" goes through SteamCMD (Settings can
        // turn it off again).
        if std::mem::take(&mut self.steamcmd_fresh_install)
            && !self.use_steamcmd
            && crate::steamcmd::is_installed(&self.steamcmd_dir())
        {
            self.use_steamcmd = true;
            self.status_message = format!("{} | {}", self.status_message, t.get(TranslationKey::SteamCmdNowOn));
        }
        if done > 0 {
            if let Some(report) = self.mod_updates.as_mut() {
                report.mods.retain(|_, u| !done_ids.contains(&u.workshop_id));
                if report.outdated_count() == 0 {
                    self.filter_updates = false;
                }
            }
            self.rescan_mods_keep_status();
        }
    }

    /// F5: adds a shared `.rml`, `ModsConfig.xml` or `.zip` pack as a new set (a free name is picked)
    /// and selects it. Names and Workshop IDs come from the file, else from installed mods.
    pub fn import_modlist(&mut self, file: &Path) {
        let t = self.i18n;
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let file_name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_zip = file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let (list, zip_configs) = if is_zip {
            let Some(pack) = crate::modlists::read_zip_pack(file) else {
                self.status_message = fmt(t.get(TranslationKey::SetsImportBad), &[&file_name]);
                return;
            };
            (pack.list, Some(pack.configs))
        } else {
            let Some(list) = crate::modlists::read_shared(file) else {
                self.status_message = fmt(t.get(TranslationKey::SetsImportBad), &[&file_name]);
                return;
            };
            (list, None)
        };
        let base = crate::modlists::check_name(&list.name)
            .unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
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
        let version = list
            .game_version
            .clone()
            .unwrap_or_else(|| self.game_version_for_sets());
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &name), &version, &entries) {
            Ok(()) => {
                let mut written_configs = 0usize;
                // The player's own settings of these mods are copied away before the pack's
                // replace them (a different file only), so nothing is lost.
                let backup_dir = crate::game_install::manager_data_dir()
                    .join("mod_settings_backup")
                    .join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
                let mut backed_up = 0usize;
                if let (Some(cfgs), Some(cfg_dir)) = (
                    zip_configs.as_ref(),
                    self.mods_config_path.as_deref().and_then(Path::parent),
                ) {
                    let _ = std::fs::create_dir_all(cfg_dir);
                    for (cfg_name, bytes) in cfgs {
                        if !crate::modlists::is_mod_config_name(cfg_name) {
                            continue;
                        }
                        let target = cfg_dir.join(cfg_name);
                        if let Ok(old) = std::fs::read(&target) {
                            if &old == bytes {
                                continue;
                            }
                            let saved = std::fs::create_dir_all(&backup_dir)
                                .and_then(|_| std::fs::write(backup_dir.join(cfg_name), &old))
                                .is_ok();
                            if !saved {
                                continue;
                            }
                            backed_up += 1;
                        }
                        if std::fs::write(&target, bytes).is_ok() {
                            written_configs += 1;
                        }
                    }
                }
                self.refresh_modlists();
                self.selected_modlist = Some(name.clone());
                self.status_message = if zip_configs.is_some() {
                    let mut msg = fmt(
                        t.get(TranslationKey::SetsImportedZip),
                        &[&name, &entries.len().to_string(), &written_configs.to_string()],
                    );
                    if backed_up > 0 {
                        msg.push_str(" | ");
                        msg.push_str(&fmt(
                            t.get(TranslationKey::SetsZipBackup),
                            &[&backed_up.to_string(), &backup_dir.display().to_string()],
                        ));
                    }
                    msg
                } else {
                    fmt(
                        t.get(TranslationKey::SetsImported),
                        &[&name, &entries.len().to_string()],
                    )
                };
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// F5: copies the set's `.rml` file (or exports a `.zip` pack with `Config/Mod_*.xml`) to where the user chose.
    pub fn export_modlist(&mut self, name: &str, target: &Path) {
        let t = self.i18n;
        let Some(list) = self.modlists.iter().find(|l| l.name == name) else {
            return;
        };
        let is_zip = target
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if is_zip {
            let rml = match std::fs::read_to_string(&list.path) {
                Ok(s) => s,
                Err(e) => {
                    self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]);
                    return;
                }
            };
            let cfg_dir = self.mods_config_path.as_deref().and_then(Path::parent);
            self.status_message = match crate::modlists::write_zip_pack(target, name, &rml, cfg_dir) {
                Ok(n) => fmt(
                    t.get(TranslationKey::SetsExportedZip),
                    &[name, &n.to_string(), &target.display().to_string()],
                ),
                Err(e) => fmt(t.get(TranslationKey::SetsWriteError), &[&e]),
            };
            return;
        }
        self.status_message = match std::fs::copy(&list.path, target) {
            Ok(_) => fmt(
                t.get(TranslationKey::SetsExported),
                &[name, &target.display().to_string()],
            ),
            Err(e) => fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        };
    }

    /// F5: starts importing a Steam Workshop collection (or a list of Workshop links) into a new set.
    pub fn import_steam_collection(&mut self, raw: &str) -> Result<(), String> {
        let t = self.i18n;
        if self.modlists_folder().is_none() {
            return Err(t.get(TranslationKey::SetsNoFolder).to_string());
        }
        if crate::mod_updates::extract_workshop_ids(raw).is_empty() {
            return Err(t.get(TranslationKey::SetsCollectionBadUrl).to_string());
        }
        if self.collection_rx.is_some() {
            return Ok(());
        }
        let input = raw.to_string();
        self.status_message = t.get(TranslationKey::SetsCollectionLoading).to_string();
        self.collection_rx = Some(self.spawn_task(move || crate::mod_updates::fetch_collection_or_items(&input)));
        Ok(())
    }

    fn finish_collection_import(&mut self, res: Result<crate::mod_updates::RemoteCollection, String>) {
        let t = self.i18n;
        let col = match res {
            Ok(c) => c,
            Err(e) => {
                self.status_message = fmt(t.get(TranslationKey::SetsCollectionError), &[&e]);
                return;
            }
        };
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let base = crate::modlists::check_name(&col.title)
            .unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let name = crate::modlists::unique_name(&dir, &base);
        let entries: Vec<crate::modlists::ModListEntry> = col
            .items
            .iter()
            .map(|(ws_id, ws_title)| {
                let installed = self.all_mods.iter().find(|m| {
                    m.steam_item_id.as_deref() == Some(ws_id.as_str())
                        || m.dir_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n == ws_id)
                });
                let idx = self.workshop_index.as_ref();
                let id = installed
                    .map(|m| m.package_id.to_lowercase())
                    .or_else(|| idx.and_then(|i| i.package_for_id(ws_id)).map(str::to_string))
                    .unwrap_or_else(|| format!("workshop.{ws_id}"));
                let item_name = installed
                    .map(|m| m.name.clone())
                    .or_else(|| (!ws_title.trim().is_empty()).then(|| ws_title.clone()))
                    .or_else(|| idx.and_then(|i| i.name_for_id(ws_id)).map(str::to_string))
                    .unwrap_or_else(|| ws_id.clone());
                crate::modlists::ModListEntry {
                    id,
                    name: item_name,
                    steam_id: Some(ws_id.clone()),
                }
            })
            .collect();
        let entries = modpack_from_collection(entries, &self.all_mods, Some(&self.community_db));
        let version = self.game_version_for_sets();
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &name), &version, &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.selected_modlist = Some(name.clone());
                self.status_message = fmt(
                    t.get(TranslationKey::SetsImported),
                    &[&name, &entries.len().to_string()],
                );
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    pub fn collection_busy(&self) -> bool {
        self.collection_rx.is_some()
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
        let installed = self
            .steam_paths
            .game_version
            .as_deref()
            .and_then(crate::game_install::major_minor);
        let saved = save.game_version.as_deref().and_then(crate::game_install::major_minor);
        matches!((installed, saved), (Some(a), Some(b)) if a != b)
    }

    /// "Enable as in the save": the save's mods in its order, as unsaved changes
    /// (ModsConfig.xml changes only on "Save Order"; mods not on disk are kept).
    pub fn load_save_mods(&mut self, name: &str) {
        let Some(save) = self.saves.iter().find(|s| s.name == name) else {
            return;
        };
        let mut ids = Vec::with_capacity(save.ids.len());
        for id in &save.ids {
            add_active_id(&mut ids, id);
        }
        self.skip_save_check = true;
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
        let Some(save) = self.saves.iter().find(|s| s.name == name) else {
            return;
        };
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
        let base =
            crate::modlists::check_name(name).unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let set_name = crate::modlists::unique_name(&dir, &base);
        let version = save
            .game_version
            .clone()
            .unwrap_or_else(|| self.game_version_for_sets());
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &set_name), &version, &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.selected_modlist = Some(set_name.clone());
                self.status_message = fmt(t.get(TranslationKey::SavesToSetDone), &[&set_name]);
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// F7: renames a save file on disk.
    pub fn rename_save(&mut self, from: &str, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self
            .saves_folder()
            .ok_or_else(|| self.i18n.get(TranslationKey::SavesNoFolder).to_string())?;
        let source = self
            .saves
            .iter()
            .find(|s| s.name == from)
            .map(|s| s.path.clone())
            .unwrap_or_else(|| crate::saves::path_for(&dir, from));
        let target = crate::saves::path_for(&dir, &name);
        if target.exists() && !name.eq_ignore_ascii_case(from) {
            return Err(self.i18n.get(TranslationKey::SavesNameExists).to_string());
        }
        std::fs::rename(&source, &target)
            .map_err(|e| fmt(self.i18n.get(TranslationKey::SavesWriteError), &[&e.to_string()]))?;
        if let Some(save) = self.saves.iter_mut().find(|s| s.name == from) {
            save.name = name.clone();
            save.path = target;
        }
        if self.selected_save.as_deref() == Some(from) {
            self.selected_save = Some(name.clone());
        }
        self.status_message = fmt(self.i18n.get(TranslationKey::SavesRenamed), &[from, &name]);
        Ok(())
    }

    /// F7: moves a save file to the Recycle Bin.
    pub fn delete_save(&mut self, name: &str) {
        let Some(path) = self
            .saves
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.path.clone())
            .or_else(|| self.saves_folder().map(|d| crate::saves::path_for(&d, name)))
        else {
            return;
        };
        match crate::cleaner::move_mod_dir_to_trash(&path) {
            Ok(()) => {
                self.saves.retain(|s| s.name != name);
                if self.selected_save.as_deref() == Some(name) {
                    self.selected_save = self.saves.first().map(|s| s.name.clone());
                }
                self.status_message = fmt(self.i18n.get(TranslationKey::SavesDeleted), &[name]);
            }
            Err(e) => {
                self.status_message = fmt(self.i18n.get(TranslationKey::SavesWriteError), &[&e]);
            }
        }
    }

    /// Subscribes to the save's mods that are not installed and have a Workshop ID.
    pub fn subscribe_save_missing(&mut self, name: &str) {
        let Some(save) = self.saves.iter().find(|s| s.name == name) else {
            return;
        };
        let ids = self.save_missing_workshop_ids(save);
        self.subscribe_workshop_ids(ids);
    }

    pub fn save_missing_workshop_ids(&self, save: &crate::saves::SaveInfo) -> Vec<String> {
        let mut ids: Vec<String> = save
            .ids
            .iter()
            .filter(|id| self.find_mod(id).is_none())
            .filter_map(|id| self.workshop_id_for(id, save.steam_id(id), save.name_of(id)))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    // ----- Find missing mods by name (D1) -----

    /// A missing mod with no Workshop link, to look for by name. Core and DLC are left out.
    fn wanted(&self, package_id: &str, known: Option<&str>, name: Option<&str>) -> Option<crate::name_search::Wanted> {
        let lower = package_id.to_lowercase();
        if self.find_mod(package_id).is_some()
            || lower == "ludeon.rimworld"
            || lower.starts_with("ludeon.rimworld.")
            || self.workshop_id_for(package_id, known, name).is_some()
        {
            return None;
        }
        let db = self
            .workshop_index
            .as_ref()
            .and_then(|i| i.by_package.get(&lower))
            .map(|items| items.iter().map(|(id, _)| id.clone()).collect())
            .unwrap_or_default();
        Some(crate::name_search::Wanted {
            package_id: package_id.to_string(),
            name: name.filter(|n| !n.trim().is_empty()).unwrap_or(package_id).to_string(),
            known: db,
        })
    }

    /// The set's missing mods that have no Workshop link (`only`: just this one).
    pub fn modlist_wanted(
        &self,
        list: &crate::modlists::ModList,
        only: Option<&str>,
    ) -> Vec<crate::name_search::Wanted> {
        list.ids
            .iter()
            .enumerate()
            .filter(|(_, id)| only.is_none_or(|o| o.eq_ignore_ascii_case(id)))
            .filter_map(|(i, id)| self.wanted(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
            .collect()
    }

    /// The save's missing mods that have no Workshop link (`only`: just this one).
    pub fn save_wanted(&self, save: &crate::saves::SaveInfo, only: Option<&str>) -> Vec<crate::name_search::Wanted> {
        save.ids
            .iter()
            .filter(|id| only.is_none_or(|o| o.eq_ignore_ascii_case(id)))
            .filter_map(|id| self.wanted(id, save.steam_id(id), save.name_of(id)))
            .collect()
    }

    /// Opens the "find by name" window and searches the Workshop for each mod in the
    /// background (3 requests at a time, like the modpack translation search).
    pub fn start_name_search(&mut self, source: String, wanted: Vec<crate::name_search::Wanted>) {
        if wanted.is_empty() {
            return;
        }
        self.name_search_stop.store(true, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        self.name_search_stop = stop.clone();
        let installed: std::collections::HashSet<String> =
            self.all_mods.iter().filter_map(|m| m.steam_item_id.clone()).collect();
        self.name_search = Some(crate::name_search::NameSearch::new(source, wanted.clone()));
        let (tx, rx) = channel();
        self.name_search_rx = Some(rx);
        let ctx = self.repaint_ctx.clone();
        let route = self.workshop_route();
        std::thread::spawn(move || {
            use std::sync::atomic::AtomicUsize;
            let next = AtomicUsize::new(0);
            std::thread::scope(|s| {
                for _ in 0..3 {
                    s.spawn(|| loop {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(w) = wanted.get(i) else { break };
                        let query = crate::name_search::clean_name(&w.name);
                        let msg = match search_steam_workshop(&query, &route) {
                            Ok(raw) => NameSearchMsg::Found(i, crate::name_search::matches_for(raw, w, &installed)),
                            Err(_) => NameSearchMsg::Failed(i),
                        };
                        let _ = tx.send(msg);
                        if let Some(ctx) = &ctx {
                            ctx.request_repaint();
                        }
                    });
                }
            });
            let _ = tx.send(NameSearchMsg::Done);
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        });
    }

    /// RWMM_TAB=saves-findname: starts the search once both the mods and the saves are read.
    fn name_search_when_ready(&mut self) {
        if !self.name_search_after_load || self.all_mods.is_empty() || self.saves.is_empty() {
            return;
        }
        self.name_search_after_load = false;
        let name = self
            .selected_save
            .clone()
            .or_else(|| self.saves.first().map(|s| s.name.clone()));
        let Some(save) = name.and_then(|n| self.saves.iter().find(|s| s.name == n)) else {
            return;
        };
        let (source, wanted) = (save.name.clone(), self.save_wanted(save, None));
        self.start_name_search(source, wanted);
    }

    pub fn stop_name_search(&mut self) {
        self.name_search_stop.store(true, Ordering::Relaxed);
    }

    pub fn close_name_search(&mut self) {
        self.stop_name_search();
        self.name_search = None;
        self.name_search_rx = None;
    }

    fn poll_name_search(&mut self) {
        let Some(rx) = &self.name_search_rx else { return };
        let mut done = false;
        while let Ok(msg) = rx.try_recv() {
            let Some(search) = self.name_search.as_mut() else { break };
            match msg {
                NameSearchMsg::Found(i, matches) => search.set_matches(i, matches),
                NameSearchMsg::Failed(i) => search.set_failed(i),
                NameSearchMsg::Done => done = true,
            }
        }
        if done {
            self.name_search_rx = None;
            if let Some(search) = self.name_search.as_mut() {
                search.running = false;
            }
        }
    }

    /// Subscribes to the ticked items of the "find by name" window in one Steam session.
    pub fn subscribe_name_search_checked(&mut self) {
        if self.steam_rx.is_some() {
            return;
        }
        let Some(search) = self.name_search.as_mut() else {
            return;
        };
        let mut ids: Vec<String> = search.checked.drain().collect();
        ids.sort();
        search.subscribed.extend(ids.iter().cloned());
        self.subscribe_workshop_ids(ids);
    }

    // ----- Top bar tabs (U1) -----

    /// Whether the tab is in the top bar. A hidden tab can still be opened by a button
    /// that leads to it (e.g. leftover DDS files -> Optimizer).
    pub fn tab_visible(&self, tab: AppTab) -> bool {
        let p = &self.ui_prefs.tabs;
        match tab {
            AppTab::Workshop => !p.hide_workshop,
            AppTab::Sets => !p.hide_sets,
            AppTab::Saves => !p.hide_saves,
            AppTab::Translations => !p.hide_translations,
            AppTab::Optimizer => !p.hide_optimizer,
            AppTab::Mods => !p.hide_mods,
            AppTab::Analyzer => !p.hide_analyzer,
            AppTab::Settings => true,
        }
    }

    /// The tabs the side bar shows, as the user set them up in Settings (Settings always).
    pub fn visible_tabs(&self) -> Vec<AppTab> {
        AppTab::ALL.into_iter().filter(|tab| self.tab_visible(*tab)).collect()
    }

    /// A tab's name.
    pub fn tab_name(&self, tab: AppTab) -> &'static str {
        Self::tab_title(self.i18n, tab)
    }

    /// A tab's name in the given language set (no borrow of the app).
    pub fn tab_title(t: I18n, tab: AppTab) -> &'static str {
        let key = match tab {
            AppTab::Mods => TranslationKey::TabMods,
            AppTab::Workshop => TranslationKey::TabWorkshop,
            AppTab::Sets => TranslationKey::TabSets,
            AppTab::Saves => TranslationKey::TabSaves,
            AppTab::Translations => TranslationKey::TabTranslations,
            AppTab::Optimizer => TranslationKey::TabOptimizer,
            AppTab::Analyzer => TranslationKey::TabAnalyzer,
            AppTab::Settings => TranslationKey::TabSettings,
        };
        t.get(key)
    }

    // ----- Mod updates (F10) -----

    /// Asks the Workshop in the background which installed mods have a newer version.
    /// Covers Workshop mods and, on a game without Steam, local copies kept in folders
    /// named by their Workshop ID.
    pub fn check_mod_updates(&mut self) {
        if self.mod_updates_rx.is_some() {
            return;
        }
        let items: Vec<crate::mod_updates::CheckItem> = self
            .all_mods
            .iter()
            .filter(|m| !self.is_official_mod(m))
            .filter_map(|m| {
                let workshop_id = workshop_id_of(m)?;
                Some(crate::mod_updates::CheckItem {
                    package_id: m.package_id.clone(),
                    workshop_id,
                    dir: m.dir_path.clone(),
                    local_versions: m.supported_versions.clone(),
                })
            })
            .collect();
        let t = self.i18n;
        if items.is_empty() {
            self.status_message = t.get(TranslationKey::UpdatesNothing).to_string();
            return;
        }
        let acfs = self.workshop_records();
        let game = self.steam_paths.game_major_minor();
        self.status_message = fmt(t.get(TranslationKey::UpdatesChecking), &[&items.len().to_string()]);
        self.mod_updates_rx = Some(self.spawn_task(move || crate::mod_updates::check(items, acfs, game)));
    }

    pub fn checking_mod_updates(&self) -> bool {
        self.mod_updates_rx.is_some()
    }

    // ----- Previous versions (F9 of the plan, item 20) -----

    /// After an update check: saves the installed copy of every outdated mod in the
    /// background, before Steam or SteamCMD replaces it (frozen and Core/DLC mods are left).
    fn save_previous_versions(&mut self, report: &crate::mod_updates::UpdateReport) {
        if !self.keep_mod_versions || self.mod_versions_rx.is_some() {
            return;
        }
        let items: Vec<crate::mod_versions::SaveItem> = report
            .mods
            .iter()
            .filter(|(_, u)| u.outdated)
            .filter_map(|(id, _)| self.find_mod(id))
            .filter(|m| !m.frozen && !self.is_official_mod(m))
            .map(|m| crate::mod_versions::SaveItem {
                package_id: m.package_id.clone(),
                name: m.name.clone(),
                workshop_id: workshop_id_of(m),
                dir: m.dir_path.clone(),
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let root = crate::mod_versions::versions_dir();
        self.mod_versions_rx = Some(self.spawn_task(move || crate::mod_versions::save(&root, &items)));
    }

    /// The saved previous version of a mod, if any.
    pub fn saved_version(&self, id: &str) -> Option<&crate::mod_versions::SavedVersion> {
        self.mod_versions.get(&id.to_lowercase())
    }

    /// "Restore previous version" in a mod's menu: asks first.
    pub fn ask_restore_version(&mut self, id: &str) {
        if self.saved_version(id).is_some() && self.mod_restore_rx.is_none() {
            self.pending_question = Some(PendingQuestion::RestoreVersion(id.to_lowercase()));
        }
    }

    /// Puts the saved copy back in the background, then scans the mods again.
    fn restore_version_now(&mut self, id: &str) {
        let (Some(version), Some(m)) = (self.saved_version(id).cloned(), self.find_mod(id)) else {
            return;
        };
        let current = m.dir_path.clone();
        let from_workshop = m.origin == crate::models::ModOrigin::Workshop;
        let mods_dir = self.game_mods_dir();
        let name = m.name.clone();
        self.mod_restore_rx = Some(self.spawn_task(move || {
            let result = crate::mod_versions::restore(&version, &current, from_workshop, mods_dir.as_deref());
            (name, result)
        }));
    }

    /// Deletes every saved previous version (Settings).
    pub fn delete_saved_versions(&mut self) {
        let root = crate::mod_versions::versions_dir();
        self.status_message = match crate::mod_versions::delete(&root, None) {
            Ok(()) => fmt(
                self.i18n.get(TranslationKey::VersionsDeleted),
                &[&self.mod_versions.len().to_string()],
            ),
            Err(e) => fmt(self.i18n.get(TranslationKey::VersionsDeleteFailed), &[&e.to_string()]),
        };
        self.mod_versions = crate::mod_versions::list(&root);
    }

    /// SC1: outdated mods SteamCMD can update — the ones in the game's Mods folder. Mods the
    /// Steam client keeps in its Workshop folder are left to Steam (a SteamCMD copy would
    /// give the mod twice).
    pub fn steamcmd_updatable_ids(&self) -> Vec<String> {
        let Some(report) = &self.mod_updates else {
            return Vec::new();
        };
        let mut ids: Vec<String> = report
            .mods
            .iter()
            .filter(|(pkg, u)| u.outdated && self.can_update_with_steamcmd(pkg))
            .map(|(_, u)| u.workshop_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// The mod is on disk outside the Steam client's Workshop folder.
    pub fn can_update_with_steamcmd(&self, package_id: &str) -> bool {
        self.find_mod(package_id)
            .is_some_and(|m| m.origin != crate::models::ModOrigin::Workshop && !m.frozen)
    }

    /// SC1: downloads the outdated mods of the game's Mods folder again (SteamCMD).
    pub fn update_outdated_mods(&mut self) {
        let ids = self.steamcmd_updatable_ids();
        self.subscribe_workshop_ids(ids);
    }

    /// What the last check found for a mod (only mods with a newer version).
    pub fn mod_update(&self, package_id: &str) -> Option<&crate::mod_updates::ModUpdate> {
        self.mod_updates
            .as_ref()
            .and_then(|r| r.mods.get(&package_id.to_lowercase()))
    }

    /// A mod of the game's Mods folder: not from Workshop and not Core/DLC.
    pub fn is_local_mod(&self, m: &ModInfo) -> bool {
        m.origin == crate::models::ModOrigin::Local && !self.is_official_mod(m)
    }

    /// The "updates", "no support for this version" and "local" filters of the mod lists.
    pub fn passes_extra_filters(&self, m: Option<&ModInfo>, game_mm: Option<&str>) -> bool {
        let has_update = m.is_some_and(|m| self.mod_update(&m.package_id).is_some());
        let lacks_version = m.is_some_and(|m| crate::version_check::supports_version(m, game_mm) == Some(false));
        let local = m.is_some_and(|m| self.is_local_mod(m));
        let workshop = m.is_some_and(|m| self.is_workshop_mod(m));
        let colored = self.color_filter.is_empty()
            || m.and_then(|m| self.mod_colors.get(&m.package_id.to_lowercase()))
                .is_some_and(|c| self.color_filter.contains(c));
        (has_update || !self.filter_updates)
            && (lacks_version || !self.filter_no_version)
            && (local || !self.filter_local)
            && (workshop || !self.filter_workshop)
            && colored
            && (self.dlc_filter.is_empty()
                || m.is_some_and(|m| required_dlcs(m).iter().any(|d| self.dlc_filter.contains(d))))
    }

    /// A Steam Workshop subscription (not Core/DLC).
    pub fn is_workshop_mod(&self, m: &ModInfo) -> bool {
        m.origin == crate::models::ModOrigin::Workshop && !self.is_official_mod(m)
    }

    /// Some chip filter is on: "Enable All" / "Disable All" act on the shown mods only.
    pub fn filters_on(&self) -> bool {
        !self.category_filter.is_empty()
            || self.filter_updates
            || self.filter_no_version
            || self.filter_local
            || self.filter_workshop
            || !self.color_filter.is_empty()
            || !self.dlc_filter.is_empty()
    }

    // ----- Duplicates the player hid (F9 of the plan) -----

    /// Duplicates to show: not hidden, or hidden with other folders than now.
    pub fn shown_duplicates(&self) -> Vec<&crate::scanner::DuplicateModEntry> {
        self.duplicate_mods
            .iter()
            .filter(|d| !self.is_duplicate_ignored(d))
            .collect()
    }

    fn is_duplicate_ignored(&self, d: &crate::scanner::DuplicateModEntry) -> bool {
        self.ignored_duplicates
            .get(&d.package_id.to_lowercase())
            .is_some_and(|dirs| d.origins.iter().all(|(_, dir)| dirs.contains(dir)))
    }

    /// How many duplicates are hidden now (for "Show again" in Settings).
    pub fn ignored_duplicate_count(&self) -> usize {
        self.duplicate_mods
            .iter()
            .filter(|d| self.is_duplicate_ignored(d))
            .count()
    }

    /// "Ignore" on a duplicate: hidden while it has these copies.
    pub fn ignore_duplicate(&mut self, package_id: &str) {
        let Some(d) = self
            .duplicate_mods
            .iter()
            .find(|d| d.package_id.eq_ignore_ascii_case(package_id))
        else {
            return;
        };
        let dirs = d.origins.iter().map(|(_, dir)| dir.clone()).collect();
        self.ignored_duplicates.insert(package_id.to_lowercase(), dirs);
        self.save_ignored_duplicates();
    }

    // ----- Settings in one file (F9 of the plan, item 22) -----

    /// "Save settings to a file": the current settings go to disk first, then into the zip.
    pub fn export_settings(&mut self, target: &Path) {
        self.save_settings_if_changed();
        let t = self.i18n;
        self.status_message = match crate::settings_backup::export(&crate::game_install::manager_data_dir(), target) {
            Ok(n) => fmt(
                t.get(TranslationKey::SettingsBackupExported),
                &[&n.to_string(), &target.display().to_string()],
            ),
            Err(e) => fmt(t.get(TranslationKey::SettingsBackupError), &[&e]),
        };
    }

    /// "Load settings from a file": replaces the settings files, then offers a restart.
    pub fn import_settings(&mut self, source: &Path) {
        let t = self.i18n;
        match crate::settings_backup::import(source, &crate::game_install::manager_data_dir()) {
            Ok(n) => {
                self.settings_imported = true;
                self.status_message = fmt(t.get(TranslationKey::SettingsBackupImported), &[&n.to_string()]);
                self.pending_question = Some(PendingQuestion::RestartAfterImport);
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SettingsBackupError), &[&e]),
        }
    }

    /// Starts the program again and closes this window (imported settings load at start).
    fn restart(&mut self, ctx: &egui::Context) {
        let t = self.i18n;
        // An unsaved load order would be lost without the usual question.
        if self.has_unsaved_changes() {
            self.status_message = t.get(TranslationKey::SettingsBackupRestartUnsaved).to_string();
            return;
        }
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).spawn()) {
            Ok(_) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SettingsBackupError), &[&e.to_string()]),
        }
    }

    // ----- Undo / redo of the active list (F9 of the plan, item 23) -----

    /// Called once a frame: remembers each change of the active list for undo, and warns
    /// when an edit turns off mods the newest save is made with (item 24).
    fn track_active_changes(&mut self) {
        let whole_list = std::mem::take(&mut self.skip_save_check);
        if let Some(before) = self.active_history.observe(&self.active_mods) {
            if !whole_list {
                self.warn_removed_from_save(&before);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.active_history.can_undo()
    }

    /// Ctrl+Z: the active list as it was before the last change.
    pub fn undo_active(&mut self) {
        if let Some(prev) = self.active_history.undo(&self.active_mods) {
            self.active_mods = prev;
            self.after_history_jump(TranslationKey::UndoDone, self.active_history.undo.len());
        }
    }

    /// Ctrl+Y / Ctrl+Shift+Z: the change undone last, again.
    pub fn redo_active(&mut self) {
        if let Some(next) = self.active_history.redo(&self.active_mods) {
            self.active_mods = next;
            self.after_history_jump(TranslationKey::RedoDone, self.active_history.redo.len());
        }
    }

    fn after_history_jump(&mut self, key: TranslationKey, left: usize) {
        self.validate_deps();
        self.status_message = fmt(self.i18n.get(key), &[&left.to_string()]);
    }

    // ----- Mods the newest save needs (F9 of the plan, item 24) -----

    /// The save written last, once the saves are read (read in the background at start).
    fn newest_save(&self) -> Option<&crate::saves::SaveInfo> {
        self.saves.iter().max_by_key(|s| s.modified.unwrap_or(0))
    }

    /// Mods of `before` that are off now and that the newest save is made with
    /// (Core stays on anyway): asks whether to turn them back on.
    fn warn_removed_from_save(&mut self, before: &[String]) {
        if self.pending_question.is_some() {
            return;
        }
        let Some(save) = self.newest_save() else { return };
        let names: Vec<String> = turned_off_from_save(before, &self.active_mods, &save.ids)
            .into_iter()
            .map(|id| {
                self.find_mod(&id)
                    .map(|m| m.name.clone())
                    .or_else(|| save.name_of(&id).map(str::to_string))
                    .unwrap_or(id)
            })
            .collect();
        if !names.is_empty() {
            self.pending_question = Some(PendingQuestion::RemovedFromSave {
                save: save.name.clone(),
                names,
            });
        }
    }

    /// Shows every hidden duplicate again.
    pub fn show_ignored_duplicates(&mut self) {
        self.ignored_duplicates.clear();
        self.save_ignored_duplicates();
    }

    fn save_ignored_duplicates(&mut self) {
        if let Err(e) = save_ignored_duplicates(&ignored_duplicates_path(), &self.ignored_duplicates) {
            self.status_message = fmt(self.i18n.get(TranslationKey::DuplicatesSaveFailed), &[&e.to_string()]);
        }
    }

    /// The mod passes the chips and the search text `query` (lower case) of a list, as the
    /// list draws it: the available list also searches authors (`by_author`).
    fn mod_shown(&self, m: Option<&ModInfo>, id: &str, query: &str, game_mm: Option<&str>, by_author: bool) -> bool {
        let text_ok = query.is_empty()
            || id.to_lowercase().contains(query)
            || m.is_some_and(|m| {
                m.name.to_lowercase().contains(query) || (by_author && m.author.to_lowercase().contains(query))
            });
        text_ok && passes_category_filter(&self.category_filter, m) && self.passes_extra_filters(m, game_mm)
    }

    // ----- Colour marks (F9 of the plan) -----

    /// Gives the mods `ids` the colour `color` (`None` takes it away) and saves the marks.
    pub fn set_mod_color(&mut self, ids: &[String], color: Option<crate::mod_colors::ModColor>) {
        crate::mod_colors::set(&mut self.mod_colors, ids, color);
        if let Err(e) = crate::mod_colors::save(&crate::mod_colors::marks_path(), &self.mod_colors) {
            self.status_message = fmt(self.i18n.get(TranslationKey::ColorSaveFailed), &[&e.to_string()]);
        }
        // A colour nobody has any more can't stay a filter.
        let used: std::collections::HashSet<_> = self.mod_colors.values().copied().collect();
        self.color_filter.retain(|c| used.contains(c));
    }

    pub fn mod_color(&self, id: &str) -> Option<crate::mod_colors::ModColor> {
        self.mod_colors.get(&id.to_lowercase()).copied()
    }

    /// Installed mods (not Core/DLC) with the colour `color`, for deleting a whole group.
    pub fn mods_with_color(&self, color: crate::mod_colors::ModColor) -> Vec<String> {
        self.all_mods
            .iter()
            .filter(|m| !self.is_official_mod(m) && self.mod_color(&m.package_id) == Some(color))
            .map(|m| m.package_id.clone())
            .collect()
    }

    // ----- Order of the available mods (F9 of the plan) -----

    /// Chooses the order; loads what it needs the first time (Steam's Workshop details,
    /// folder sizes in the background, folder dates at once).
    pub fn set_inactive_sort(&mut self, sort: InactiveSort) {
        self.inactive_sort = sort;
        if sort.needs_workshop() && self.workshop_stats.is_none() && self.workshop_stats_rx.is_none() {
            let ids: Vec<String> = self
                .all_mods
                .iter()
                .filter(|m| !self.is_official_mod(m))
                .filter_map(workshop_id_of)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if ids.is_empty() {
                self.workshop_stats = Some(HashMap::new());
            } else {
                self.workshop_stats_rx = Some(self.spawn_task(move || crate::mod_updates::fetch_details(&ids)));
            }
        }
        if sort == InactiveSort::Size && self.mod_sizes.is_none() && self.mod_sizes_rx.is_none() {
            let dirs: Vec<(String, PathBuf)> = self
                .all_mods
                .iter()
                .map(|m| (m.package_id.to_lowercase(), m.dir_path.clone()))
                .collect();
            self.mod_sizes_rx =
                Some(self.spawn_task(move || dirs.into_iter().map(|(id, dir)| (id, folder_size(&dir))).collect()));
        }
        if sort == InactiveSort::Added && self.mod_added.is_none() {
            self.mod_added = Some(
                self.all_mods
                    .iter()
                    .filter_map(|m| Some((m.package_id.to_lowercase(), folder_added(&m.dir_path)?)))
                    .collect(),
            );
        }
    }

    /// The data for the chosen order is still loading.
    pub fn inactive_sort_loading(&self) -> bool {
        match self.inactive_sort {
            s if s.needs_workshop() => self.workshop_stats_rx.is_some(),
            InactiveSort::Size => self.mod_sizes_rx.is_some(),
            _ => false,
        }
    }

    /// The number the chosen order sorts `m` by (bigger first); `None` goes to the end.
    pub fn inactive_sort_value(&self, m: &ModInfo) -> Option<u64> {
        let remote = || {
            let id = workshop_id_of(m)?;
            self.workshop_stats.as_ref()?.get(&id)
        };
        let key = m.package_id.to_lowercase();
        match self.inactive_sort {
            InactiveSort::Default | InactiveSort::Name => None,
            InactiveSort::Subscribers => remote().map(|r| r.subscriptions),
            InactiveSort::Updated => remote().map(|r| r.time_updated),
            InactiveSort::Size => self.mod_sizes.as_ref()?.get(&key).copied(),
            InactiveSort::Added => self.mod_added.as_ref()?.get(&key).copied(),
        }
    }

    /// Puts the available mods in the chosen order (stable: equal ones keep their place).
    pub fn sort_inactive(&self, mods: &mut [&ModInfo]) {
        match self.inactive_sort {
            InactiveSort::Default => {}
            InactiveSort::Name => mods.sort_by(|a, b| a.name_lower.cmp(&b.name_lower)),
            _ => mods.sort_by_key(|m| std::cmp::Reverse(self.inactive_sort_value(m))),
        }
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
                    || lower == crate::companion::PACKAGE_ID
                    || self
                        .find_mod(id)
                        .is_some_and(|m| self.is_official_mod(m) || m.categories.contains(&ModCategory::Framework))
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
                    .chain(
                        crate::dependencies::dependency_substitutes(&dep)
                            .iter()
                            .map(|s| s.to_string()),
                    )
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
                    let _ = crate::fs_util::write_atomic(&path, text);
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
        let (Some(search), Some(config)) = (self.bisect.as_ref(), self.mods_config_path.clone()) else {
            return;
        };
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
        self.skip_save_check = true;
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
            self.start_bisect_game_watch();
        }
    }

    /// Spawns a background task that waits for RimWorld to exit and then analyzes a fresh Player.log.
    fn spawn_game_log_watch(&self) -> Option<Receiver<Option<LogReport>>> {
        let log_target = self
            .custom_log_path
            .clone()
            .or_else(|| detect_rimworld_log_paths().into_iter().next().map(|(_, p)| p));
        let path = log_target?;
        let mods = self.all_mods.clone();
        let launched_at = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(2))
            .unwrap_or(std::time::UNIX_EPOCH);
        let launch_clock = std::time::Instant::now();
        Some(self.spawn_task(move || {
            // Wait up to 20 s for RimWorld to appear in the process list.
            let mut pid = None;
            for _ in 0..40 {
                pid = crate::game_install::game_pid();
                if pid.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            let mut pid = pid?;
            // F11: measure the loading while the game runs. When Steam restarts the game the
            // PID changes: the CPU time and disk reads of the first process carry over.
            let mut probe = crate::load_time::LoadProbe::default();
            // The samples of the last launch, to see why a time was or was not measured.
            let mut probe_log = std::fs::File::create(crate::load_time::probe_log_path()).ok();
            if let Some(f) = probe_log.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "{}", crate::load_time::ProbeSample::CSV_HEADER);
            }
            let mut base = crate::game_install::ProcessStats::default();
            let mut last = base;
            let mut measurable = true;
            loop {
                if measurable && probe.result().is_none() {
                    match crate::game_install::process_stats(pid) {
                        Some(stats) => {
                            last = stats;
                            let sample = crate::load_time::ProbeSample {
                                cpu_secs: base.cpu_secs + stats.cpu_secs,
                                read_bytes: base.read_bytes + stats.read_bytes,
                                private_bytes: stats.private_bytes,
                                log_len: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                                responsive: stats.responsive,
                            };
                            let secs = launch_clock.elapsed().as_secs_f32();
                            let done = probe.push(secs, sample);
                            if let Some(f) = probe_log.as_mut() {
                                use std::io::Write;
                                let _ = writeln!(f, "{}", sample.csv_row(secs, done));
                            }
                        }
                        None => measurable = false,
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                match crate::game_install::game_pid() {
                    Some(p) if p == pid => {}
                    Some(p) => {
                        base.cpu_secs += last.cpu_secs;
                        base.read_bytes += last.read_bytes;
                        last = crate::game_install::ProcessStats::default();
                        pid = p;
                    }
                    None => break,
                }
            }
            // Give the game a moment to flush Player.log (and write Prefs.xml) on exit.
            std::thread::sleep(std::time::Duration::from_millis(600));
            // A measured launch: verbose logging off again, now that the game won't overwrite it.
            crate::load_profile::finish_measured_launch();
            let fresh = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t >= launched_at);
            if !fresh {
                return None;
            }
            let mut report = analyze_rimworld_log(&path, &mods);
            // The companion mod's own time (from the log) beats the guess from outside.
            report.load_time_secs = report.load_time_secs.or(probe.result());
            Some(report)
        }))
    }

    /// F8: watches for the game process to exit after a bisect launch, then analyzes Player.log.
    fn start_bisect_game_watch(&mut self) {
        self.bisect_watch_rx = self.spawn_game_log_watch();
    }

    /// Watches for RimWorld to exit after a normal launch so F9 / F11 only record verified runs.
    fn start_launch_game_watch(&mut self) {
        self.launch_watch_rx = self.spawn_game_log_watch();
    }

    pub fn bisect_watching_game(&self) -> bool {
        self.bisect_watch_rx.is_some()
    }

    pub fn bisect_answer(&mut self, problem: bool) {
        self.bisect_watch_rx = None;
        if let Some(search) = self.bisect.as_mut() {
            search.answer(problem);
            self.bisect_note = None;
            self.persist_bisect();
        }
    }

    pub fn bisect_step_back(&mut self) {
        self.bisect_watch_rx = None;
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
        let Some(config) = self.mods_config_path.clone() else {
            return;
        };
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
        self.skip_save_check = true;
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

    /// Refreshes, in the background, the diff between the active mods and the last known
    /// working snapshot (F9). It reads Steam's install records and walks the files of local
    /// mods, too slow for every click in the mod list; a change during a run reruns it.
    pub fn refresh_last_good_diff(&mut self) {
        let Some(snapshot) = self.last_good.clone() else {
            self.last_good_diff = None;
            return;
        };
        if self.last_good_rx.is_some() {
            self.last_good_rerun = true;
            return;
        }
        let active = self.active_mods.clone();
        let mods: Vec<ModInfo> = active.iter().filter_map(|id| self.find_mod(id)).cloned().collect();
        let workshop = self.steam_paths.workshop_path.clone();
        let steamcmd = self.steamcmd_dir();
        self.last_good_rx = Some(self.spawn_task(move || {
            let timestamps =
                crate::last_good::collect_mod_timestamps(&active, &mods, workshop.as_deref(), Some(&steamcmd));
            snapshot.diff(&active, &timestamps)
        }));
    }

    fn poll_last_good_diff(&mut self) {
        let Some(rx) = &self.last_good_rx else { return };
        if let Ok(diff) = rx.try_recv() {
            self.last_good_rx = None;
            self.last_good_diff = Some(diff);
            if std::mem::take(&mut self.last_good_rerun) {
                self.refresh_last_good_diff();
            }
        }
    }

    /// Records the current active mod configuration and file timestamps as the known good run (F9).
    pub fn remember_last_good(&mut self) {
        let mods = self.active_mods.clone();
        self.remember_last_good_for(&mods);
        self.status_message = self.i18n.get(TranslationKey::LastGoodSavedMsg).to_string();
    }

    /// Saves a `LastGoodSnapshot` for the specified verified mod order without overwriting
    /// `status_message`. In the background: it reads Steam's records and walks local mods.
    pub fn remember_last_good_for(&mut self, active_mods: &[String]) {
        let steamcmd = self.steamcmd_dir();
        let active = active_mods.to_vec();
        let mods: Vec<ModInfo> = active.iter().filter_map(|id| self.find_mod(id)).cloned().collect();
        let steam_paths = self.steam_paths.clone();
        self.last_good_save_rx = Some(self.spawn_task(move || {
            let snap = crate::last_good::LastGoodSnapshot::create(&active, &mods, &steam_paths, Some(&steamcmd));
            let _ = snap.save();
            snap
        }));
    }

    fn poll_last_good_save(&mut self) {
        let Some(rx) = &self.last_good_save_rx else { return };
        if let Ok(snap) = rx.try_recv() {
            self.last_good_save_rx = None;
            self.last_good = Some(snap);
            self.refresh_last_good_diff();
        }
    }

    /// Records load time (F11) and last-good snapshot (F9) only when `report` comes from a
    /// `Player.log` written after RimWorld was actually launched in this session (the launch
    /// watcher, once the game has closed).
    /// A run counts as working when the game neither crashed nor hit a graphics card or memory
    /// failure nor a broken Prepatcher / Harmony patch: big mod lists nearly always log some errors.
    pub fn handle_verified_launch_report(&mut self, report: &LogReport) {
        if !is_verified_launch_log(
            self.game_launch_time,
            self.bisect.is_some(),
            self.custom_log_path.is_some(),
            &report.log_path,
        ) {
            return;
        }
        self.game_launch_time = None;
        let mods = if !self.game_launch_active_mods.is_empty() {
            std::mem::take(&mut self.game_launch_active_mods)
        } else if !self.saved_active_mods.is_empty() {
            self.saved_active_mods.clone()
        } else {
            self.active_mods.clone()
        };
        if let Some(secs) = report.load_time_secs {
            let mut history = std::mem::take(&mut self.load_time_history);
            history.record_launch(secs, &mods, |id| self.mod_display_name(id));
            let _ = history.save();
            self.load_time_history = history;
        }
        if is_working_run(report) {
            self.remember_last_good_for(&mods);
        }
    }

    /// Launches a bisect search over the mods updated or added since the last good run (F9);
    /// the rest stay on as the base.
    pub fn start_bisect_on_updated(&mut self) {
        let Some(diff) = &self.last_good_diff else { return };
        if diff.updated.is_empty() && diff.added.is_empty() {
            return;
        };
        let suspects: std::collections::HashSet<String> = diff
            .updated
            .iter()
            .chain(&diff.added)
            .map(|id| id.to_lowercase())
            .collect();

        let mut keep = std::collections::HashSet::new();
        for id in &self.active_mods {
            let key = id.to_lowercase();
            if !suspects.contains(&key) {
                keep.insert(key);
            }
        }
        self.bisect_keep = Some(keep);
        self.start_bisect();
        self.bisect_open = true;
    }

    /// Mods related to the selected one (highlighted in both lists).
    pub fn related_to_selected(&self) -> std::collections::HashMap<String, crate::relations::Relation> {
        self.selected_mod_id
            .as_deref()
            .and_then(|id| self.find_mod(id))
            .map(|sel| crate::relations::related_mods(sel, &self.all_mods))
            .unwrap_or_default()
    }

    /// A mod's display name: the installed mod's name, else its ID.
    pub fn mod_display_name(&self, id: &str) -> String {
        self.find_mod(id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// Whether a Steam request (subscribe, check) is running now.
    /// A Steam subscription or a SteamCMD download is running.
    pub fn steam_busy(&self) -> bool {
        self.steam_rx.is_some() || self.steamcmd_rx.is_some()
    }

    /// "Enable": writes the set to ModsConfig.xml (a backup of the old order is made),
    /// and starts the game too if asked.
    pub fn apply_modlist(&mut self, name: &str, launch: bool) {
        // The modpacks the game loads until now: the mods' settings in use are theirs.
        let live: Vec<String> = self
            .modlists
            .iter()
            .filter(|l| self.modlist_diff(l).is_same())
            .map(|l| l.name.clone())
            .collect();
        self.load_modlist(name);
        if self.save_config() {
            let mut message = fmt(self.i18n.get(TranslationKey::SetsApplied), &[name]);
            // Enabling the modpack that is on already changes nothing, its settings neither.
            if !live.iter().any(|l| l == name) {
                if let Some(settings) = self.switch_modlist_settings(&live, name) {
                    message.push_str(" | ");
                    message.push_str(&settings);
                }
            }
            self.status_message = message;
            if launch {
                self.check_vram_and_launch();
            }
        }
    }

    /// Makes the textures of set pictures that are not loaded yet: the user's own picture,
    /// else a placeholder generated from the name. Small files, read once per set.
    pub fn load_modlist_pictures(&mut self, ctx: &egui::Context) {
        let dir = crate::modlist_images::images_dir();
        for list in &self.modlists {
            if self.modlist_pictures.contains_key(&list.name) {
                continue;
            }
            let own = crate::modlist_images::load_image(&dir, &list.name);
            let custom = own.is_some();
            let img = own.unwrap_or_else(|| crate::modlist_images::placeholder(&list.name));
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            let tex = ctx.load_texture(
                format!("modlist_picture/{}", list.name),
                color,
                egui::TextureOptions::LINEAR,
            );
            self.modlist_pictures.insert(list.name.clone(), (tex, custom));
        }
    }

    /// Makes a picture file the card picture of a set.
    pub fn set_modlist_picture(&mut self, name: &str, source: &Path) {
        match crate::modlist_images::set_image(&crate::modlist_images::images_dir(), name, source) {
            Ok(()) => {
                self.modlist_pictures.remove(name);
            }
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsPictureError), &[&e]),
        }
    }

    /// Back to the generated placeholder.
    pub fn remove_modlist_picture(&mut self, name: &str) {
        crate::modlist_images::remove_image(&crate::modlist_images::images_dir(), name);
        self.modlist_pictures.remove(name);
    }

    /// Turns a tag on or off for a set and saves the tags.
    pub fn toggle_modlist_tag(&mut self, set: &str, tag: &str) {
        crate::modlist_tags::toggle(&mut self.modlist_tags, set, tag);
        self.save_modlist_tags();
    }

    fn save_modlist_tags(&mut self) {
        if let Err(e) = crate::modlist_tags::save(&crate::modlist_tags::tags_path(), &self.modlist_tags) {
            self.status_message = fmt(self.i18n.get(TranslationKey::SetsTagsSaveError), &[&e.to_string()]);
        }
    }

    /// Copies a set to a free "Name (copy)" and selects the copy.
    pub fn duplicate_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        let copy = crate::modlists::copy_name(&dir, name, self.i18n.get(TranslationKey::SetsCopyWord));
        match std::fs::copy(
            crate::modlists::path_for(&dir, name),
            crate::modlists::path_for(&dir, &copy),
        ) {
            Ok(_) => {
                crate::modlist_images::copy_image(&crate::modlist_images::images_dir(), name, &copy);
                crate::modlist_tags::copy(&mut self.modlist_tags, name, &copy);
                crate::modlist_settings::copy(&crate::modlist_settings::root(), name, &copy);
                self.save_modlist_tags();
                self.refresh_modlists();
                self.selected_modlist = Some(copy.clone());
                self.status_message = fmt(self.i18n.get(TranslationKey::SetsDuplicated), &[name, &copy]);
            }
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// "Merge with…" (F9 of the plan): a new set "A + B" with the mods of both in load
    /// order; the two sets stay as they are.
    pub fn merge_modlists(&mut self, first: &str, second: &str) {
        let t = self.i18n;
        let find = |name: &str| self.modlists.iter().find(|l| l.name == name).cloned();
        let (Some(dir), Some(a), Some(b)) = (self.modlists_folder(), find(first), find(second)) else {
            return;
        };
        let mut entries = crate::modlists::merge(&a, &b);
        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let place: HashMap<String, usize> = sort_load_order_with_db(&ids, &self.all_mods, Some(&self.community_db))
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id.to_lowercase(), i))
            .collect();
        entries.sort_by_key(|e| place.get(&e.id).copied().unwrap_or(usize::MAX));
        let name = crate::modlists::unique_name(&dir, &format!("{first} + {second}"));
        let path = crate::modlists::path_for(&dir, &name);
        match crate::modlists::write_rml(&path, &self.game_version_for_sets(), &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.selected_modlist = Some(name.clone());
                self.status_message = fmt(
                    t.get(TranslationKey::SetsMerged),
                    &[first, second, &name, &entries.len().to_string()],
                );
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
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
        let Some(list) = self.modlists.iter().find(|l| l.name == name).cloned() else {
            return;
        };
        let mut ids = Vec::with_capacity(list.ids.len());
        for id in &list.ids {
            // A collection imported before the mod was on disk lists it as "workshop.<ID>":
            // once it's downloaded, the set enables it under its real ID (the game knows no other).
            let real = id
                .starts_with("workshop.")
                .then(|| self.find_mod(id).map(|m| m.package_id.clone()))
                .flatten();
            add_active_id(&mut ids, real.as_deref().unwrap_or(id));
        }
        self.skip_save_check = true;
        self.active_mods = ids;
        self.validate_deps();
        self.current_modlist = Some(list.name.clone());
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsLoaded), &[&list.name]);
    }

    /// Saves the current load order as a new set. The error text is shown in the dialog.
    pub fn save_modlist_as(&mut self, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self
            .modlists_folder()
            .ok_or_else(|| self.i18n.get(TranslationKey::SetsNoFolder).to_string())?;
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
        self.status_message = fmt(
            self.i18n.get(TranslationKey::SetsSaved),
            &[name, &entries.len().to_string()],
        );
        Ok(())
    }

    pub fn rename_modlist(&mut self, from: &str, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self
            .modlists_folder()
            .ok_or_else(|| self.i18n.get(TranslationKey::SetsNoFolder).to_string())?;
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
        crate::modlist_images::rename_image(&crate::modlist_images::images_dir(), from, &name);
        self.modlist_pictures.remove(from);
        self.modlist_pictures.remove(&name);
        crate::modlist_tags::rename(&mut self.modlist_tags, from, &name);
        crate::modlist_settings::rename(&crate::modlist_settings::root(), from, &name);
        self.save_modlist_tags();
        self.refresh_modlists();
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsRenamed), &[from, &name]);
        Ok(())
    }

    /// Moves a set file to the Recycle Bin. Mods and ModsConfig.xml are not touched.
    pub fn delete_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        match trash::delete(crate::modlists::path_for(&dir, name)) {
            Ok(()) => {
                crate::modlist_images::remove_image(&crate::modlist_images::images_dir(), name);
                crate::modlist_settings::remove(&crate::modlist_settings::root(), name);
                self.modlist_pictures.remove(name);
                if self.modlist_tags.remove(name).is_some() {
                    self.save_modlist_tags();
                }
                self.status_message = fmt(self.i18n.get(TranslationKey::SetsDeleted), &[name]);
            }
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
        self.skip_save_check = true;
        self.active_mods = backup.mods.clone();
        self.validate_deps();
        self.history = None;
        self.status_message = tr!(
            "Загружен порядок от {} — нажмите «Сохранить порядок», чтобы его увидела игра",
            "Loaded the order from {} — press \"Save Order\" so the game sees it",
            "已从 {} 加载排序 — 请点击「保存排序」让游戏使用它",
            crate::ui::components::local_date_time(backup.saved_at)
        );
    }

    /// Adds a mod to the end of the load order unless it is already there (IDs are
    /// compared case-insensitively and stored in lower case, like ModsConfig.xml does).
    /// Returns whether it was added.
    pub fn activate_mod(&mut self, id: &str) -> bool {
        add_active_id(&mut self.active_mods, id)
    }

    /// "Enable All": every mod, or only the available mods the chips and the search show.
    pub fn activate_all_mods(&mut self) {
        let ids: Vec<String> = if self.filters_on() {
            let game_mm = self.steam_paths.game_major_minor();
            let query = self.mod_search_query.trim().to_lowercase();
            self.all_mods
                .iter()
                .filter(|m| self.mod_shown(Some(m), &m.package_id, &query, game_mm.as_deref(), true))
                .map(|m| m.package_id.clone())
                .collect()
        } else {
            self.all_mods.iter().map(|m| m.package_id.clone()).collect()
        };
        for id in ids {
            self.activate_mod(&id);
        }
        self.validate_deps();
    }

    /// "Disable All": every mod but Core, or only the active mods the chips and the search show.
    pub fn deactivate_all_mods(&mut self) {
        if self.filters_on() {
            let game_mm = self.steam_paths.game_major_minor();
            let query = self.active_search_query.trim().to_lowercase();
            let shown: std::collections::HashSet<String> = self
                .active_mods
                .iter()
                .filter(|id| {
                    let m = lookup_mod(&self.mod_index, &self.all_mods, id);
                    self.mod_shown(m, id, &query, game_mm.as_deref(), false)
                })
                .map(|id| id.to_lowercase())
                .collect();
            self.active_mods
                .retain(|id| is_protected_package_id(id) || !shown.contains(&id.to_lowercase()));
        } else {
            self.active_mods.retain(|id| is_protected_package_id(id));
        }
        self.validate_deps();
    }

    /// Turns one mod off (incompatible pair in the dependency bar).
    pub fn deactivate_mod(&mut self, id: &str) {
        let before = self.active_mods.len();
        self.active_mods.retain(|a| !a.eq_ignore_ascii_case(id));
        if self.active_mods.len() != before {
            self.validate_deps();
            let name = self.mod_display_name(id);
            self.status_message = fmt(self.i18n.get(TranslationKey::DepTurnedOff), &[&name]);
        }
    }

    /// Puts an active mod right after another one (a dependency it has to load after), then
    /// checks the dependencies again. The `_steam` copy of either counts as the same mod.
    pub fn move_mod_after(&mut self, id: &str, after: &str) {
        let same = |a: &str, b: &str| {
            let (a, b) = (a.to_lowercase(), b.to_lowercase());
            a == b || a.strip_suffix("_steam") == Some(&b) || b.strip_suffix("_steam") == Some(&a)
        };
        let Some(from) = self.active_mods.iter().position(|m| same(m, id)) else {
            return;
        };
        let moved = self.active_mods.remove(from);
        let Some(target) = self.active_mods.iter().position(|m| same(m, after)) else {
            self.active_mods.insert(from, moved);
            return;
        };
        self.active_mods.insert(target + 1, moved);
        self.validate_deps();
    }

    /// Workshop IDs of the missing dependencies: from the About.xml of the mod that needs
    /// one (`steamWorkshopUrl`), else from the Workshop database when it's downloaded.
    pub fn missing_dependency_workshop_ids(&self) -> Vec<String> {
        let index = self.workshop_index.as_ref();
        let mut ids: Vec<String> = Vec::new();
        for problem in &self.dependency_report.problems {
            if let DependencyProblem::Missing {
                required_id,
                required_info,
                ..
            } = problem
            {
                let ws = required_info
                    .workshop_id
                    .clone()
                    .or_else(|| index.and_then(|idx| idx.lookup(required_id, None)).map(str::to_string));
                if let Some(ws) = ws {
                    if !ids.contains(&ws) {
                        ids.push(ws);
                    }
                }
            }
        }
        ids
    }

    /// Downloads every missing dependency the Workshop database knows, in one go.
    pub fn download_missing_dependencies(&mut self) {
        let ids = self.missing_dependency_workshop_ids();
        self.subscribe_workshop_ids(ids);
    }

    /// Mods the user just enabled by hand (double click, menu, drag): says once per session
    /// which of them do not declare the installed game version.
    pub fn warn_no_version(&mut self, ids: &[String]) {
        let game_mm = self.steam_paths.game_major_minor();
        let found: Vec<(String, String)> = ids
            .iter()
            .filter_map(|id| self.find_mod(id))
            .filter(|m| crate::version_check::supports_version(m, game_mm.as_deref()) == Some(false))
            .map(|m| (m.package_id.to_lowercase(), m.name.clone()))
            .filter(|(id, _)| !self.warned_no_version.contains(id))
            .collect();
        if found.is_empty() {
            return;
        }
        let version = game_mm.unwrap_or_else(|| "?".to_string());
        let t = self.i18n;
        self.status_message = if let [(_, name)] = found.as_slice() {
            fmt(t.get(TranslationKey::ModsNoVersionWarnOne), &[name, &version])
        } else {
            let mut names: Vec<&str> = found.iter().take(3).map(|(_, n)| n.as_str()).collect();
            if found.len() > 3 {
                names.push("…");
            }
            fmt(
                t.get(TranslationKey::ModsNoVersionWarnMany),
                &[&found.len().to_string(), &version, &names.join(", ")],
            )
        };
        self.warned_no_version.extend(found.into_iter().map(|(id, _)| id));
    }

    /// Enables every missing dependency, including dependencies of the mods it just
    /// enabled, then sorts the load order once.
    pub fn auto_fix_dependencies(&mut self) {
        let added = enable_dependencies_recursive(&mut self.active_mods, &self.all_mods);
        self.active_mods = sort_load_order_with_db(&self.active_mods, &self.all_mods, Some(&self.community_db));
        self.dep_alert_ignored.clear();
        self.validate_deps();
        self.status_message = if added > 0 {
            tr!(
                "Включено зависимостей: {}, порядок загрузки отсортирован",
                "Enabled dependencies: {}, load order sorted",
                "已启用前置：{}，加载顺序已排序",
                added
            )
        } else {
            self.i18n.get(TranslationKey::AppSortedMsg).to_string()
        };
        let missing = self.dependency_report.missing_list().len();
        if missing > 0 {
            self.status_message.push_str(" | ");
            self.status_message.push_str(&fmt(
                self.i18n.get(TranslationKey::DepMissingManual),
                &[&missing.to_string()],
            ));
        }
        if self.dependency_report.incompatible_count() > 0 {
            self.status_message.push_str(" | ");
            self.status_message
                .push_str(self.i18n.get(TranslationKey::DepIncompatibleManual));
        }
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
                "受保护的模组「{}」不能删除",
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

    /// Asks for confirmation of a context menu action on mods (delete, unsubscribe,
    /// reinstall); only the mods it applies to are listed. Core and DLC are left out.
    pub fn request_mod_action(&mut self, kind: DeleteKind, ids: &[String]) {
        let mods: Vec<&ModInfo> = ids
            .iter()
            .filter_map(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .filter(|m| match kind {
                DeleteKind::Unsubscribe => m.steam_item_id.is_some(),
                DeleteKind::Reinstall => self.can_reinstall(m),
                DeleteKind::Freeze => can_freeze(m),
                DeleteKind::Unfreeze => m.frozen,
                _ => true,
            })
            .collect();
        if mods.is_empty() {
            return;
        }
        let pending = PendingDelete {
            kind,
            ids: mods.iter().map(|m| m.package_id.clone()).collect(),
            names: mods.iter().map(|m| m.name.clone()).collect(),
        };
        self.pending_delete = Some(pending);
    }

    /// A mod can be downloaded again: a Steam subscription, or a copy in the Mods folder
    /// named by its Workshop ID while SteamCMD downloads mods.
    pub fn can_reinstall(&self, m: &ModInfo) -> bool {
        if self.is_official_mod(m) {
            return false;
        }
        // A frozen mod keeps its version: unfreeze it first.
        !m.frozen && (m.steam_item_id.is_some() || (self.steamcmd_active() && workshop_id_of(m).is_some()))
    }

    /// After a background deletion: deleted mods leave the active list, unsubscribed ones
    /// too; reinstalled ones stay and are downloaded again.
    fn finish_delete_job(&mut self, kind: DeleteKind, targets: Vec<DeleteTarget>, steam_ok: bool) {
        match kind {
            DeleteKind::Unsubscribe => {
                if steam_ok {
                    let ids: Vec<String> = targets.into_iter().map(|(id, _, _)| id).collect();
                    remove_mods(&mut self.active_mods, &ids);
                }
            }
            DeleteKind::Reinstall => {
                let ws_ids: Vec<String> = targets
                    .into_iter()
                    .filter(|(_, dir, _)| !dir.exists())
                    .filter_map(|(_, _, ws)| ws)
                    .collect();
                if !ws_ids.is_empty() {
                    if self.steam_busy() {
                        self.status_message = self.i18n.get(TranslationKey::ModsReinstallBusy).to_string();
                    } else {
                        self.download_workshop_ids(ws_ids, true);
                    }
                }
            }
            // The mods stay installed and enabled; the rescan picks up the new folders.
            DeleteKind::Freeze | DeleteKind::Unfreeze => {}
            _ => {
                let gone: Vec<String> = targets
                    .into_iter()
                    .filter(|(_, dir, _)| !dir.exists())
                    .map(|(id, _, _)| id)
                    .collect();
                remove_mods(&mut self.active_mods, &gone);
            }
        }
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
        let freezing = matches!(pending.kind, DeleteKind::Freeze | DeleteKind::Unfreeze);
        let mods_dir = self.game_mods_dir();
        self.status_message = if freezing {
            self.i18n.get(TranslationKey::FreezeWorking).to_string()
        } else {
            tr!("Удаление...", "Deleting...", "正在删除...").to_string()
        };
        self.delete_job = Some((
            pending.kind,
            targets
                .iter()
                .map(|m| (m.package_id.clone(), m.dir_path.clone(), workshop_id_of(m)))
                .collect(),
        ));
        let t = self.i18n;
        let done = |message: String| DeleteDone {
            message,
            steam_ok: true,
        };

        self.delete_rx = Some(self.spawn_task(move || match pending.kind {
            DeleteKind::Freeze | DeleteKind::Unfreeze => {
                let freeze = pending.kind == DeleteKind::Freeze;
                let results: Vec<Result<(), String>> = targets
                    .iter()
                    .map(|m| {
                        if freeze {
                            crate::freeze::freeze(m, workshop_id_of(m), mods_dir.as_deref()).map(|_| ())
                        } else {
                            crate::freeze::unfreeze(&m.dir_path, steam_paths.workshop_path.as_deref()).map(|_| ())
                        }
                    })
                    .collect();
                let ok = results.iter().filter(|r| r.is_ok()).count();
                let key = if freeze {
                    TranslationKey::FreezeDone
                } else {
                    TranslationKey::UnfreezeDone
                };
                let mut msg = fmt(t.get(key), &[&ok.to_string()]);
                if let Some(err) = results.iter().find_map(|r| r.as_ref().err()) {
                    msg.push_str(&fmt(
                        t.get(TranslationKey::FreezeErrors),
                        &[&(results.len() - ok).to_string(), err],
                    ));
                }
                done(msg)
            }
            DeleteKind::Unsubscribe => {
                let refs: Vec<&ModInfo> = targets.iter().collect();
                match crate::cleaner::unsubscribe_mods(&refs, &steam_paths) {
                    Ok(n) => done(fmt(t.get(TranslationKey::ModsUnsubscribed), &[&n.to_string()])),
                    Err(e) => DeleteDone {
                        message: fmt(t.get(TranslationKey::ModsUnsubscribeFailed), &[&e]),
                        steam_ok: false,
                    },
                }
            }
            DeleteKind::Translations | DeleteKind::Mods | DeleteKind::Reinstall => {
                let refs: Vec<&ModInfo> = targets.iter().collect();
                let res = match pending.kind {
                    DeleteKind::Translations => delete_translation_mods(&refs, &steam_paths),
                    DeleteKind::Reinstall => crate::cleaner::trash_mods(&refs),
                    _ => delete_mods(&refs, &steam_paths),
                };
                let mut msg = tr!(
                    "Перемещено в Корзину: {} | Отписано в Steam: {}",
                    "Moved to Recycle Bin: {} | Unsubscribed in Steam: {}",
                    "已移至回收站：{} | 已在 Steam 取消订阅：{}",
                    res.deleted_count,
                    res.unsubscribed_count
                );
                if let Some(err) = res.errors.first() {
                    msg.push_str(&tr!(
                        " | Ошибок: {} ({})",
                        " | Errors: {} ({})",
                        " | 错误：{}（{}）",
                        res.errors.len(),
                        err
                    ));
                }
                done(msg)
            }
            DeleteKind::Single => {
                let m = &targets[0];
                done(match unsubscribe_and_delete_single(m, &steam_paths) {
                    Ok(()) => tr!(
                        "Мод {} перемещён в Корзину",
                        "Mod {} moved to the Recycle Bin",
                        "模组 {} 已移至回收站",
                        m.name
                    ),
                    Err(e) => tr!(
                        "Мод {} не удалён: {}",
                        "Mod {} not deleted: {}",
                        "模组 {} 未删除：{}",
                        m.name,
                        e
                    ),
                })
            }
        }));
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
                "Select a mod in the list to find a translation for it",
                "请在列表中选择一个模组以查找它的翻译"
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
        self.status_message = tr!(
            "Поиск в Workshop: {}...",
            "Searching the Workshop: {}...",
            "正在搜索创意工坊：{}...",
            query
        );

        let route = self.workshop_route();
        self.workshop_rx = Some(self.spawn_task(move || {
            let raw = search_steam_workshop(&query, &route)?;
            let found = raw.len();
            // Keep only translations of this mod in the selected language,
            // drop the mod itself and unrelated items, best matches first.
            let filtered = filter_and_rank_workshop_translations(raw, &name, steam_id.as_deref(), lang);
            let status = tr!(
                "Workshop: найдено {}, подходящих переводов «{}»: {}",
                "Workshop: found {}, matching translations of \"{}\": {}",
                "创意工坊：找到 {} 个，「{}」的匹配翻译：{}",
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
        let mut candidates: Vec<(String, String, Option<String>)> = Vec::new();
        for id in &self.active_mods {
            let Some(m) = self.find_mod(id) else { continue };
            if self.is_official_mod(m) || m.is_translation_mod || !seen.insert(m.package_id.to_lowercase()) {
                continue;
            }
            if lang.is_built_into(m) || translated.contains(&m.package_id.to_lowercase()) {
                skipped += 1;
                continue;
            }
            candidates.push((m.package_id.to_lowercase(), m.name.clone(), m.steam_item_id.clone()));
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
                "所有已启用的模组都已有 {} — 无需搜索",
                lang.label()
            );
            return;
        }
        self.status_message = tr!(
            "Поиск переводов на {} для {} модов...",
            "Searching {} translations for {} mods...",
            "正在为 {1} 个模组搜索 {0} 翻译...",
            lang.label(),
            candidates.len()
        );

        let stop = Arc::new(AtomicBool::new(false));
        self.batch_stop = stop.clone();
        let (tx, rx) = channel();
        self.batch_rx = Some(rx);
        let ctx = self.repaint_ctx.clone();
        let route = self.workshop_route();
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
                        let Some((package_id, name, steam_id)) = candidates.get(i) else {
                            break;
                        };
                        let query = format!("{} {}", name, lang.search_suffix());
                        match search_steam_workshop(&query, &route) {
                            Ok(raw) => {
                                let items = crate::matcher::translations_for_mod(
                                    raw,
                                    name,
                                    steam_id.as_deref(),
                                    lang,
                                    &installed_steam,
                                );
                                if !items.is_empty() {
                                    let _ = tx.send(BatchSearchMsg::Found(ModTranslations {
                                        package_id: package_id.clone(),
                                        mod_name: name.clone(),
                                        items,
                                    }));
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

    /// T2: mods for which the last modpack search found a translation that isn't subscribed
    /// yet: lowercase package ID -> the best match's title.
    pub fn translation_hints(&self) -> std::collections::HashMap<String, String> {
        self.batch_results
            .iter()
            .filter(|r| !r.items.iter().any(|i| self.batch_subscribed.contains(&i.id)))
            .filter_map(|r| r.items.first().map(|best| (r.package_id.clone(), best.title.clone())))
            .collect()
    }

    /// Opens the Translations tab on the modpack search results.
    pub fn open_batch_translations(&mut self) {
        self.show_batch_results = true;
        self.current_tab = AppTab::Translations;
    }

    pub fn stop_batch_translation_search(&mut self) {
        self.batch_stop.store(true, Ordering::Relaxed);
        self.status_message = tr!("Останавливаю поиск...", "Stopping the search...", "正在停止搜索...").to_string();
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
                BatchSearchMsg::Done {
                    searched,
                    errors,
                    cancelled,
                } => finished = Some((searched, errors, cancelled)),
            }
        }
        if let Some((searched, errors, cancelled)) = finished {
            self.batch_rx = None;
            let mut msg = tr!(
                "Проверено модов: {} | с переводом на {}: {}",
                "Mods checked: {} | with a {} translation: {}",
                "已检查模组：{} | 有 {} 翻译的：{}",
                searched,
                self.batch_lang.label(),
                self.batch_results.len()
            );
            if cancelled {
                msg.push_str(tr!(" | остановлено", " | stopped", " | 已停止"));
            }
            if errors > 0 {
                msg.push_str(&tr!(
                    " | без ответа Steam: {}",
                    " | no answer from Steam: {}",
                    " | Steam 无响应：{}",
                    errors
                ));
            }
            self.status_message = msg;
        }
    }

    /// Subscribes to the ticked translations in one Steam session (background).
    pub fn subscribe_checked_translations(&mut self) {
        if self.steam_busy() || self.batch_checked.is_empty() {
            return;
        }
        if self.steamcmd_active() {
            let ids: Vec<String> = self.batch_checked.drain().collect();
            self.batch_subscribed.extend(ids.iter().cloned());
            self.start_steamcmd_job(ids);
            return;
        }
        let Some(dll) = self.steam_paths.steam_dll_path.clone() else {
            self.status_message = tr!(
                "steam_api64.dll не найден — проверьте папку игры в Настройках",
                "steam_api64.dll not found — check the game folder in Settings",
                "找不到 steam_api64.dll — 请在设置中检查游戏文件夹"
            )
            .to_string();
            return;
        };
        let ids: Vec<String> = self.batch_checked.drain().collect();
        self.batch_subscribed.extend(ids.iter().cloned());
        self.status_message = tr!(
            "Подписка на переводы: {}...",
            "Subscribing to translations: {}...",
            "正在订阅翻译：{}...",
            ids.len()
        );
        self.start_steam_client_download(dll, ids, false);
    }

    /// Free-text Workshop search — in the background (S5).
    pub fn search_workshop_custom(&mut self) {
        if self.workshop_rx.is_some() || self.workshop_search_query.trim().is_empty() {
            return;
        }
        self.show_batch_results = false;
        let query = self.workshop_search_query.clone();
        self.status_message = tr!(
            "Поиск в Workshop: {}...",
            "Searching the Workshop: {}...",
            "正在搜索创意工坊：{}...",
            query
        );
        let route = self.workshop_route();
        self.workshop_rx = Some(self.spawn_task(move || {
            let res = search_steam_workshop(&query, &route)?;
            let status = tr!(
                "Поиск в Workshop «{}»: результатов {}",
                "Workshop search \"{}\": {} results",
                "创意工坊搜索「{}」：{} 个结果",
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
                "steam_api64.dll not found — check the game folder",
                "找不到 steam_api64.dll — 请检查游戏文件夹"
            )
            .to_string();
            return;
        };
        self.status_message = tr!(
            "Проверка Steam API...",
            "Checking the Steam API...",
            "正在检查 Steam API..."
        )
        .to_string();
        self.steam_rx = Some(self.spawn_task(move || match crate::steam::steam_api_check(&dll) {
            Ok(n) => tr!(
                "Steam API работает: подписок на моды RimWorld — {}",
                "Steam API works: RimWorld mod subscriptions — {}",
                "Steam API 正常：RimWorld 模组订阅 — {}",
                n
            ),
            Err(e) => tr!(
                "Steam API не работает: {}",
                "Steam API doesn't work: {}",
                "Steam API 不可用：{}",
                e
            ),
        }));
    }

    /// Subscribes via the Steam API in the background (the request needs ~1.5 s to reach Steam).
    pub fn subscribe_steam_mod(&mut self, item_id: &str) {
        if self.steam_busy() {
            return;
        }
        if self.steamcmd_active() {
            self.start_steamcmd_job(vec![item_id.to_string()]);
            return;
        }
        if let Some(dll) = self.steam_paths.steam_dll_path.clone() {
            let id = item_id.to_string();
            self.status_message = tr!(
                "Отправка запроса на подписку #{}...",
                "Sending subscribe request #{}...",
                "正在发送订阅请求 #{}...",
                id
            );
            self.start_steam_client_download(dll, vec![id], false);
        } else {
            crate::ui::components::open_external(crate::steam::workshop_url(item_id));
            self.status_message = tr!(
                "Открыта страница Steam Workshop #{}",
                "Opened Steam Workshop page #{}",
                "已打开 Steam 创意工坊页面 #{}",
                item_id
            );
        }
    }

    /// Mod folders for the chosen scope (All / Active / Selected), without Core/DLC,
    /// the blacklist and the user's ignore list. Used by both "Start" and "Scan" (R7).
    fn optimizer_target_dirs(&self) -> Vec<PathBuf> {
        let active: std::collections::HashSet<String> = self.active_mods.iter().map(|s| s.to_lowercase()).collect();
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
            self.status_message = tr!(
                "Нет выбранных модов для оптимизации текстур",
                "No mods selected for texture optimization",
                "没有选择要优化纹理的模组"
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
            // The side the installed game needs: flipped on 1.7, as the PNG on 1.6.
            ConvertOptions {
                flip_vertical: crate::optimizer::engine_wants_flipped(self.steam_paths.engine),
                max_size: self.opt_max_size.pixels(),
            },
            self.opt_threads,
            tx,
            self.opt_stop_flag.clone(),
        );

        self.status_message = tr!(
            "Запущена конвертация текстур (BC7 / BC1 с мипмапами)...",
            "Converting textures (BC7 / BC1 with mipmaps)...",
            "正在转换纹理（BC7 / BC1，含 mipmap）..."
        )
        .to_string();
    }

    /// RimWorld 1.7: moves TextureCache and ShaderCache to the Recycle Bin (game rebuilds them).
    /// In the background: the cache can be gigabytes, moving it to the Recycle Bin takes a while (R4).
    pub fn clear_game_caches(&mut self) {
        if self.cache_rx.is_some() {
            return;
        }
        self.status_message = tr!(
            "Очистка кэша игры...",
            "Clearing the game cache...",
            "正在清除游戏缓存..."
        )
        .to_string();
        self.cache_rx = Some(self.spawn_task(|| {
            let size_mb = crate::game_install::game_cache_size() as f64 / (1024.0 * 1024.0);
            match crate::game_install::clear_game_caches() {
                Ok(cleared) if cleared.is_empty() => tr!(
                    "Кэш игры уже пуст",
                    "The game cache is already empty",
                    "游戏缓存已经是空的"
                )
                .to_string(),
                Ok(cleared) => tr!(
                    "Кэш игры перемещён в Корзину ({}, {:.0} МБ). Игра пересоберёт его при следующем запуске.",
                    "Game cache moved to the Recycle Bin ({}, {:.0} MB). The game rebuilds it on the next start.",
                    "游戏缓存已移至回收站（{}，{:.0} MB）。游戏下次启动时会重新生成。",
                    cleared.join(", "),
                    size_mb
                ),
                Err(e) => tr!(
                    "Не удалось очистить кэш: {}",
                    "Could not clear the cache: {}",
                    "无法清除缓存：{}",
                    e
                ),
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
                crate::optimizer::OrientationMsg::Progress {
                    done,
                    total,
                    current_file,
                } => {
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
            let key = if s.cancelled {
                TranslationKey::OptFlipStopped
            } else {
                TranslationKey::OptFlipDone
            };
            self.status_message = fmt(
                self.i18n.get(key),
                &[&s.fixed.to_string(), &s.already_ok.to_string(), &s.errors.to_string()],
            );
            self.rescan_mods_keep_status();
        }
    }

    pub fn cancel_texture_optimization(&mut self) {
        self.opt_stop_flag.store(true, Ordering::Relaxed);
        self.status_message = tr!("Отмена оптимизации...", "Cancelling optimization...", "正在取消优化...").to_string();
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
        texture_summary_async(
            dirs,
            self.opt_force,
            crate::optimizer::engine_wants_flipped(self.steam_paths.engine),
            self.opt_max_size.pixels(),
            tx,
        );
        self.opt_scan_rx = Some(rx);
        self.status_message = tr!("Подсчёт текстур...", "Counting textures...", "正在统计纹理...").to_string();
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
        rollback_dds_async(dirs_from_mods(&mods), false, tx);
        self.opt_rollback_rx = Some(rx);
        self.status_message = tr!("Откат текстур...", "Rolling back textures...", "正在回滚纹理...").to_string();
    }

    /// O1: rolls back only the mod selected on the Mods tab; other mods' DDS stay.
    pub fn rollback_selected_mod_textures(&mut self) {
        let t = self.i18n;
        let Some(m) = self
            .selected_mod_id
            .as_deref()
            .and_then(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .cloned()
        else {
            self.status_message = t.get(TranslationKey::OptRollbackPickMod).to_string();
            return;
        };
        let (tx, rx) = channel();
        rollback_dds_async(dirs_from_mods(std::slice::from_ref(&m)), true, tx);
        self.opt_rollback_rx = Some(rx);
        self.status_message = fmt(t.get(TranslationKey::OptRollbackOneRunning), &[&m.name]);
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
                    "待转换：{} | 已完成：{} | 作者自带 DDS（不处理）：{}",
                    s.to_convert,
                    s.up_to_date,
                    s.foreign_dds
                );
                if s.same_name > 0 {
                    self.status_message.push_str(&fmt(
                        self.i18n.get(TranslationKey::OptSameNameSkipped),
                        &[&s.same_name.to_string()],
                    ));
                }
                // O2: video memory estimate. Not on 1.7: the game compresses textures to BC7
                // itself there, so DDS only speed up loading.
                if s.pixels > 0 {
                    let mb = |b: u64| (b as f64 / 1_048_576.0).round().to_string();
                    if !crate::optimizer::engine_wants_flipped(self.steam_paths.engine) {
                        let (plain, dds) = s.vram_estimate();
                        self.status_message.push_str(&fmt(
                            self.i18n.get(TranslationKey::OptVramEstimate),
                            &[&mb(plain), &mb(dds)],
                        ));
                    }
                    let saved = s.vram_saved_by_limit();
                    if saved > 0 {
                        self.status_message
                            .push_str(&fmt(self.i18n.get(TranslationKey::OptVramSavedByLimit), &[&mb(saved)]));
                    }
                }
                self.opt_scan_rx = None;
            }
        }
        if let Some(rx) = &self.opt_rollback_rx {
            if let Ok(res) = rx.try_recv() {
                self.status_message = tr!(
                    "Откат: удалено .dds: {} | восстановлено исходников из бэкапа: {} | ошибок: {}",
                    "Rollback: .dds removed: {} | sources restored from backup: {} | errors: {}",
                    "回滚：已删除 .dds：{} | 已从备份恢复源文件：{} | 错误：{}",
                    res.removed,
                    res.restored,
                    res.errors
                );
                self.opt_rollback_rx = None;
                // Count again what is left on disk (normally nothing).
                self.rescan_mods_keep_status();
                self.trigger_vram_check();
                if !res.unlogged.is_empty() {
                    self.pending_question = Some(PendingQuestion::LegacyDds(res.unlogged));
                }
            }
        }
        self.poll_vram_check();
    }

    /// Deletes the DDS files the user agreed to remove after a rollback (background).
    fn delete_unlogged_dds(&mut self, paths: Vec<PathBuf>) {
        self.status_message = tr!("Удаление DDS...", "Deleting DDS...", "正在删除 DDS...").to_string();
        self.legacy_rx = Some(self.spawn_task(move || {
            let (removed, errors) = crate::optimizer::delete_files(&paths);
            tr!(
                "Удалено DDS не из журнала: {} | ошибок: {}",
                "Unlogged DDS deleted: {} | errors: {}",
                "已删除未记录的 DDS：{} | 错误：{}",
                removed,
                errors
            )
        }));
    }

    pub fn run_log_analysis(&mut self) {
        let log_target = if let Some(ref custom_p) = self.custom_log_path {
            Some(custom_p.clone())
        } else {
            detect_rimworld_log_paths().into_iter().next().map(|(_, p)| p)
        };

        if self.log_rx.is_some() {
            return;
        }
        if let Some(path) = log_target {
            // Matching stack frames to mods walks every mod folder: done in the background (S5).
            let mods = self.all_mods.clone();
            self.log_silent = false;
            self.status_message = tr!(
                "Анализ лога {}...",
                "Analyzing log {}...",
                "正在分析日志 {}...",
                path.display()
            );
            self.log_rx = Some(self.spawn_task(move || analyze_rimworld_log(&path, &mods)));
        } else {
            self.status_message = self.i18n.get(TranslationKey::CrashFileNotFound).to_string();
        }
    }

    /// Exports the current analysis report as text or standalone HTML (F13).
    pub fn save_log_report(&mut self) {
        let Some(report) = &self.log_report else { return };
        let t = self.i18n;
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(t.get(TranslationKey::ReportFilterText), &["txt"])
            .add_filter(t.get(TranslationKey::ReportFilterHtml), &["html"])
            .set_file_name("rimworld_mod_report.txt")
            .save_file()
        {
            let is_html = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("html"));
            let content = if is_html {
                crate::analyzer::generate_html_report(report)
            } else {
                crate::analyzer::generate_text_report(report)
            };
            self.status_message = match std::fs::write(&path, content) {
                Err(e) => fmt(t.get(TranslationKey::ReportSaveFailed), &[&e.to_string()]),
                Ok(()) => fmt(t.get(TranslationKey::ReportSaved), &[&path.display().to_string()]),
            };
        }
    }

    /// F10: mods that produced errors in the last analyzed Player.log (lowercase package_id -> culprit).
    pub fn log_culprit_map(&self) -> std::collections::HashMap<String, crate::analyzer::CrashCulprit> {
        let mut out = std::collections::HashMap::new();
        let Some(report) = &self.log_report else {
            return out;
        };
        for c in &report.culprits {
            let Some(pkg) = &c.package_id else { continue };
            let key = pkg.to_lowercase();
            let replace = out
                .get(&key)
                .is_none_or(|prev: &crate::analyzer::CrashCulprit| c.severity < prev.severity);
            if replace {
                out.insert(key, c.clone());
            }
        }
        out
    }

    fn crash_summary_text(&self, report: &LogReport) -> String {
        if let Some(issue) = report.system_issue {
            return match issue {
                crate::analyzer::SystemCrashIssue::GpuReset => {
                    self.i18n.get(TranslationKey::CrashGpuResetTitle).to_string()
                }
                crate::analyzer::SystemCrashIssue::OutOfMemory => {
                    self.i18n.get(TranslationKey::CrashOomTitle).to_string()
                }
            };
        }
        if report.has_hard_crash {
            fmt(
                self.i18n.get(TranslationKey::CrashSummaryHardCrash),
                &[&report.culprits.len().to_string()],
            )
        } else if !report.culprits.is_empty() {
            fmt(
                self.i18n.get(TranslationKey::CrashSummaryCulprits),
                &[&report.culprits.len().to_string()],
            )
        } else if report.total_errors > 0 {
            fmt(
                self.i18n.get(TranslationKey::CrashSummaryErrorsOnly),
                &[&report.total_errors.to_string()],
            )
        } else {
            self.i18n.get(TranslationKey::CrashSummaryClean).to_string()
        }
    }

    /// "Play": with an unsaved load order, asks first; then checks VRAM before launch (V2).
    pub fn launch_game(&mut self) {
        if self.has_unsaved_changes() {
            self.pending_question = Some(PendingQuestion::LaunchUnsaved);
        } else {
            self.check_vram_and_launch();
        }
    }

    pub fn check_vram_and_launch(&mut self) {
        if self.vram_report.is_none() && self.vram_rx.is_none() {
            self.trigger_vram_check();
        }
        while let Some(rx) = &self.vram_rx {
            match rx.recv_timeout(std::time::Duration::from_millis(150)) {
                Ok(Ok((report, updated_cache))) => {
                    for (dir, mtime, bytes) in updated_cache {
                        self.vram_cache.insert(dir, (mtime, bytes));
                    }
                    self.vram_rx = None;
                    if self.vram_recheck {
                        self.vram_recheck = false;
                        self.trigger_vram_check();
                        continue;
                    }
                    self.vram_report = Some(report);
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    self.launch_after_vram_check = true;
                    self.status_message = self.i18n.get(TranslationKey::VramCheckingBeforeLaunch).to_string();
                    return;
                }
                Ok(Err(())) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.launch_after_vram_check = true;
                    self.fail_vram_check();
                    return;
                }
            }
        }
        self.finish_vram_check_and_launch();
    }

    /// The VRAM check broke down: it is only a warning, so a waiting "Play" goes ahead.
    fn fail_vram_check(&mut self) {
        self.vram_rx = None;
        self.vram_recheck = false;
        self.vram_report = None;
        if std::mem::take(&mut self.launch_after_vram_check) {
            self.launch_game_now();
        }
    }

    fn finish_vram_check_and_launch(&mut self) {
        self.launch_after_vram_check = false;
        // What the game really took last time with these mods beats the estimate (1.6).
        let report = self.vram_report.as_ref().map(|estimate| {
            self.companion_report
                .as_ref()
                .and_then(|r| r.measured_vram(estimate, &self.active_mods))
                .unwrap_or_else(|| estimate.clone())
        });
        if let Some(report) = report.filter(crate::optimizer::VramReport::is_critical) {
            self.pending_question = Some(PendingQuestion::VramWarning {
                estimated_bytes: report.estimated_used_bytes,
                total_vram_bytes: report.total_vram_bytes,
                heavy_mods: report.heavy_mods,
                measured: report.measured,
            });
            return;
        }
        self.launch_game_now();
    }

    /// Triggers a background evaluation of active mod texture VRAM vs GPU capacity (V2).
    pub fn trigger_vram_check(&mut self) {
        self.vram_report = None;
        if self.vram_rx.is_some() {
            self.vram_recheck = true;
            return;
        }
        let active_mods: Vec<(String, PathBuf)> = self
            .active_mods
            .iter()
            .filter_map(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .map(|m| (m.name.clone(), m.dir_path.clone()))
            .collect();

        let engine = self.steam_paths.engine;
        let version = self.steam_paths.game_version.clone();
        let cache = self.vram_cache.clone();
        // The game's Prefs.xml lives next to the ModsConfig.xml it reads.
        let config_dir = self
            .mods_config_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        self.vram_rx = Some(self.spawn_task(move || {
            let game_compresses = crate::optimizer::game_compresses_textures(config_dir.as_deref());
            let (tx, rx) = channel();
            crate::optimizer::check_vram_async(active_mods, engine, version, game_compresses, cache, tx);
            rx.recv().map_err(|_| ())
        }));
    }

    fn poll_vram_check(&mut self) {
        let Some(rx) = &self.vram_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((report, updated_cache))) => {
                for (dir, mtime, bytes) in updated_cache {
                    self.vram_cache.insert(dir, (mtime, bytes));
                }
                self.vram_rx = None;
                if self.vram_recheck {
                    self.vram_recheck = false;
                    self.trigger_vram_check();
                    return;
                }
                self.vram_report = Some(report);
                if std::mem::take(&mut self.launch_after_vram_check) {
                    self.finish_vram_check_and_launch();
                }
            }
            Ok(Err(())) => self.fail_vram_check(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.fail_vram_check(),
        }
    }

    /// Backs up the latest save before launching RimWorld, if enabled in settings (F12).
    pub fn backup_latest_save_if_enabled(&mut self) {
        if !self.backup_save_before_launch {
            return;
        }
        let Some(saves_dir) = self.saves_folder() else {
            return;
        };
        // In the background: a save can be 100+ MB. The game does not write saves while it loads.
        let keep = self.save_backups_to_keep;
        self.save_backup_rx = Some(self.spawn_task(move || {
            crate::saves::backup_latest_save(&saves_dir, keep)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }));
    }

    fn poll_save_backup(&mut self) {
        let Some(rx) = &self.save_backup_rx else { return };
        if let Ok(res) = rx.try_recv() {
            self.save_backup_rx = None;
            if let Err(e) = res {
                self.status_message = fmt(self.i18n.get(TranslationKey::SaveBackupFailed), &[&e]);
            }
        }
    }

    fn launch_game_now(&mut self) {
        self.backup_latest_save_if_enabled();
        if std::mem::take(&mut self.measure_next_launch) {
            // The game's Prefs.xml lives next to the ModsConfig.xml it reads.
            let config_dir = self.mods_config_path.as_deref().and_then(Path::parent);
            if let Some(Err(e)) = config_dir.map(crate::load_profile::start_measured_launch) {
                self.status_message = fmt(self.i18n.get(TranslationKey::LoadProfileStartFailed), &[&e]);
            }
        }
        let args = self.launch_args();
        match launch_rimworld(&self.steam_paths, &args) {
            Ok(_) => {
                self.game_launch_time = Some(
                    std::time::SystemTime::now()
                        .checked_sub(std::time::Duration::from_secs(2))
                        .unwrap_or_else(std::time::SystemTime::now),
                );
                self.game_launch_active_mods = if !self.saved_active_mods.is_empty() {
                    self.saved_active_mods.clone()
                } else {
                    self.active_mods.clone()
                };
                if self.bisect.is_none() {
                    self.start_launch_game_watch();
                }
                self.status_message = tr!("RimWorld запущен", "RimWorld started", "RimWorld 已启动").to_string();
            }
            Err(e) => {
                self.status_message = tr!(
                    "Ошибка запуска игры: {}",
                    "Could not start the game: {}",
                    "无法启动游戏：{}",
                    e
                );
            }
        }
    }

    /// Downloads the RimSort community rules in the background (S14).
    pub fn update_community_rules(&mut self) {
        if self.rules_rx.is_some() {
            return;
        }
        self.status_message = tr!(
            "Загрузка базы правил RimSort с GitHub...",
            "Downloading RimSort rules from GitHub...",
            "正在从 GitHub 下载 RimSort 规则..."
        )
        .to_string();
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
                    "规则已更新：社区规则 {} 条，含内置规则共 {} 条",
                    imported,
                    self.community_db_rules_count
                );
            }
            Err(e) => {
                self.status_message = tr!(
                    "Не удалось обновить базу правил: {}",
                    "Could not update the rules: {}",
                    "无法更新规则：{}",
                    e
                )
            }
        }
    }
}

impl eframe::App for App {
    /// One frame: the panels of the main window go into `root` (the whole window); windows
    /// and popups still open on the context.
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::ui::theme::paint_title_bar(frame);
        crate::ui::theme::set_window_icons(frame);
        promo_screenshot(&root.ctx().clone());
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        // Process loaded mod icons
        while let Ok((pkg_id, img)) = self.icon_rx.try_recv() {
            let handle = ctx.load_texture(&pkg_id, img, Default::default());
            self.mod_textures.insert(pkg_id, Some(handle));
        }

        self.poll_background_tasks();
        self.poll_optimizer_tasks();
        if self.opt_scan_rx.is_some() || self.opt_rollback_rx.is_some() || self.vram_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Receive background optimizer messages
        let mut opt_finished = false;
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
                            "完成：BC7 {}，BC1 {} | 保留为 PNG（尺寸原因）：{} | 已是最新：{} | 错误：{}",
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
                                " | 已备份源文件：{}",
                                s.sources_backed_up
                            ));
                        }
                        if s.foreign_dds > 0 {
                            msg.push_str(&tr!(
                                " | чужих DDS не тронуто: {}",
                                " | author DDS left alone: {}",
                                " | 未处理作者自带 DDS：{}",
                                s.foreign_dds
                            ));
                        }
                        self.status_message = msg;
                        opt_finished = true;
                    }
                    OptimizationMsg::Cancelled => {
                        self.opt_is_running = false;
                        self.status_message =
                            tr!("Оптимизация остановлена", "Optimization stopped", "优化已停止").to_string();
                    }
                }
            }
        }
        if opt_finished {
            self.trigger_vram_check();
        }

        // Navigation sidebar: logo, "Play", the tabs as icon + name, Settings at the bottom.
        // Icons only (names on hover) unless names are turned on in Settings → Tabs; a window
        // narrower than 1200 px gets icons only anyway.
        let compact = !self.ui_prefs.tabs.sidebar_labels || root.available_width() < 1200.0;
        let nav_frame = egui::Frame {
            inner_margin: egui::Margin::symmetric(10, 12),
            fill: THEME.bg_surface,
            ..Default::default()
        };
        let nav_w = if compact { 64.0 } else { 212.0 };
        egui::Panel::left("nav_sidebar")
            .resizable(false)
            .default_size(nav_w)
            .size_range(nav_w..=nav_w)
            .frame(nav_frame)
            .show(root, |ui| {
                let t = self.i18n;
                let title = "RimWorld Mod Manager";
                let version = concat!("v", env!("CARGO_PKG_VERSION"));

                // Logo (and the name with the version when there is room).
                ui.horizontal(|ui| {
                    if compact {
                        ui.add_space((ui.available_width() - 36.0).max(0.0) / 2.0);
                    }
                    // The logo follows the theme: the planet in the text color, the gear in the accent.
                    let (logo_rect, logo) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    egui::Image::new(ICON_LOGO_BODY)
                        .tint(THEME.text_primary)
                        .paint_at(ui, logo_rect);
                    egui::Image::new(ICON_LOGO_ACCENT)
                        .tint(THEME.accent_cyan)
                        .paint_at(ui, logo_rect);
                    if compact {
                        logo.on_hover_text(format!("{title} {version}"));
                    } else {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(
                                egui::RichText::new("RimWorld")
                                    .color(THEME.text_primary)
                                    .size(15.0)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!("Mod Manager {version}"))
                                    .color(THEME.text_muted)
                                    .size(11.0),
                            );
                        });
                    }
                });
                ui.add_space(14.0);

                // "Play": a full-width filled button.
                let play_label = t.get(TranslationKey::AppPlayBtn);
                let (rect, play) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
                let fill = if play.hovered() {
                    THEME.accent_emerald.gamma_multiply(1.15)
                } else {
                    THEME.accent_emerald
                };
                ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
                if compact {
                    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0));
                    egui::Image::new(ICON_PLAY).tint(THEME.bg_dark).paint_at(ui, icon_rect);
                } else {
                    let galley = ui.painter().layout_no_wrap(
                        play_label.to_string(),
                        egui::FontId::proportional(14.0),
                        THEME.bg_dark,
                    );
                    let total = 18.0 + 8.0 + galley.size().x;
                    let left = rect.center().x - total / 2.0;
                    let icon_rect =
                        egui::Rect::from_min_size(egui::pos2(left, rect.center().y - 9.0), egui::vec2(18.0, 18.0));
                    egui::Image::new(ICON_PLAY).tint(THEME.bg_dark).paint_at(ui, icon_rect);
                    ui.painter().galley(
                        egui::pos2(left + 26.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        THEME.bg_dark,
                    );
                }
                let play = play.on_hover_cursor(egui::CursorIcon::PointingHand);
                let play = if compact { play.on_hover_text(play_label) } else { play };
                if play.clicked() {
                    self.launch_game();
                }

                // A new release and a running broken-mod search, as tags under "Play".
                let update_text = self
                    .update_available
                    .as_ref()
                    .map(|l| tr!("Вышла {} →", "{} is out →", "{} 已发布 →", l.tag));
                let bisect_text = (self.bisect.is_some() && !self.bisect_open).then(|| {
                    let step = self.bisect.as_ref().map_or(1, |s| s.stage.answers + 1);
                    fmt(t.get(TranslationKey::BisectHeaderTag), &[&step.to_string()])
                });
                if let (Some(latest), Some(text)) = (self.update_available.clone(), update_text.as_deref()) {
                    ui.add_space(8.0);
                    let shown = if compact { "↑" } else { text };
                    if crate::ui::components::tag(ui, shown, THEME.accent_emerald)
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(tr!(
                            "Открыть страницу новой версии на GitHub",
                            "Open the new version's page on GitHub",
                            "在 GitHub 上打开新版本页面"
                        ))
                        .clicked()
                    {
                        crate::ui::components::open_external(&latest.url);
                    }
                }
                if let Some(text) = bisect_text.as_deref() {
                    ui.add_space(8.0);
                    let shown = if compact { "?" } else { text };
                    if crate::ui::components::tag(ui, shown, THEME.accent_amber)
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(text)
                        .clicked()
                    {
                        self.open_bisect();
                    }
                }
                ui.add_space(14.0);

                // The tabs; Settings goes to the bottom of the bar.
                let tabs = self.visible_tabs();
                let name_of = move |tab: AppTab| App::tab_title(t, tab);
                ui.spacing_mut().item_spacing.y = 2.0;
                // Counts next to the names: active mods, modpacks, saves; mods with a newer
                // Workshop version light up the Workshop item.
                let outdated = self.mod_updates.as_ref().map_or(0, |r| r.outdated_count());
                let count_of = |tab: AppTab| match tab {
                    AppTab::Mods => Some((self.active_mods.len(), false)),
                    AppTab::Sets => Some((self.modlists.len(), false)).filter(|(n, _)| *n > 0),
                    AppTab::Saves => Some((self.saves.len(), false)).filter(|(n, _)| *n > 0),
                    AppTab::Workshop => (outdated > 0).then_some((outdated, true)),
                    _ => None,
                };
                let groups: [(String, &[AppTab]); 2] = [
                    (
                        tr!("БИБЛИОТЕКА", "LIBRARY", "库").to_string(),
                        &[AppTab::Mods, AppTab::Sets, AppTab::Saves, AppTab::Workshop],
                    ),
                    (
                        tr!("ИНСТРУМЕНТЫ", "TOOLS", "工具").to_string(),
                        &[AppTab::Translations, AppTab::Optimizer, AppTab::Analyzer],
                    ),
                ];
                for (title, members) in groups {
                    let shown: Vec<AppTab> = members.iter().copied().filter(|m| tabs.contains(m)).collect();
                    if shown.is_empty() {
                        continue;
                    }
                    crate::ui::components::side_nav_group(ui, &title, compact);
                    for tab in shown {
                        let icon = if tab == AppTab::Analyzer {
                            ICON_ANALYZER
                        } else {
                            tab_icon(tab)
                        };
                        if crate::ui::components::side_nav_item(
                            ui,
                            icon,
                            name_of(tab),
                            self.current_tab == tab,
                            compact,
                            count_of(tab),
                        )
                        .clicked()
                        {
                            self.current_tab = tab;
                        }
                    }
                }

                // Bottom: Settings, and above it a small card with the found game.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if crate::ui::components::side_nav_item(
                        ui,
                        ICON_SETTINGS,
                        name_of(AppTab::Settings),
                        self.current_tab == AppTab::Settings,
                        compact,
                        None,
                    )
                    .clicked()
                    {
                        self.current_tab = AppTab::Settings;
                    }
                    // Search over the whole app (also Ctrl+K), right above Settings.
                    let search_label = tr!("Поиск  (Ctrl+K)", "Search  (Ctrl+K)", "搜索 (Ctrl+K)");
                    if crate::ui::components::side_nav_item(ui, ICON_SEARCH, search_label, false, compact, None)
                        .clicked()
                    {
                        crate::ui::view_search::open(ui.ctx());
                    }
                    ui.add_space(12.0);
                    if !compact {
                        // "RimWorld 1.7" on top, the full build and the engine under it.
                        let (version, ok) = match self.steam_paths.game_major_minor() {
                            Some(v) => (format!("RimWorld {v}"), true),
                            None => (
                                tr!("Игра не найдена", "Game not found", "未找到游戏").to_string(),
                                false,
                            ),
                        };
                        // Its own top-down box: a frame laid out bottom-up gets its margins wrong.
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 48.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::NONE
                                    .fill(THEME.bg_dark)
                                    .corner_radius(CornerRadius::same(8))
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let (dot, _) =
                                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                            ui.painter().circle_filled(
                                                dot.center(),
                                                4.0,
                                                if ok { THEME.accent_emerald } else { THEME.accent_amber },
                                            );
                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(version)
                                                            .color(THEME.text_primary)
                                                            .size(12.0),
                                                    )
                                                    .truncate(),
                                                );
                                                if ok {
                                                    let build =
                                                        self.steam_paths.game_version.clone().unwrap_or_default();
                                                    let line = format!("{build} · {}", self.steam_paths.engine.label());
                                                    ui.add(
                                                        egui::Label::new(
                                                            egui::RichText::new(line)
                                                                .color(THEME.text_muted)
                                                                .size(11.0),
                                                        )
                                                        .truncate(),
                                                    );
                                                }
                                            });
                                        });
                                    });
                            },
                        );
                    }
                });
            });

        // Status bar (28 px): last status on the left, game version on the right.
        let status_frame = egui::Frame {
            inner_margin: egui::Margin::symmetric(16, 6),
            fill: THEME.bg_surface,
            ..Default::default()
        };
        egui::Panel::bottom("bottom_status_bar")
            .frame(status_frame)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&self.status_message)
                            .color(THEME.text_secondary)
                            .size(12.0),
                    );
                    if self.steam_progress.is_some() || self.steam_rx.is_some() {
                        ui.add_space(8.0);
                        if crate::ui::components::small_button(
                            ui,
                            self.i18n.get(TranslationKey::SteamBtnStopWaiting),
                            crate::ui::components::ButtonStyle::Secondary,
                        )
                        .clicked()
                        {
                            self.stop_steam_wait();
                        }
                    }
                });
            });

        // Main area: window background with 16 px side padding.
        let central_frame = egui::Frame {
            inner_margin: egui::Margin {
                left: 16,
                right: 16,
                top: 8,
                bottom: 12,
            },
            fill: THEME.bg_dark,
            ..Default::default()
        };
        egui::CentralPanel::default()
            .frame(central_frame)
            .show(root, |ui| match self.current_tab {
                AppTab::Mods => crate::ui::view_mods::render_mods_view(self, ui),
                AppTab::Sets => crate::ui::view_sets::render_sets_view(self, ui),
                AppTab::Saves => crate::ui::view_saves::render_saves_view(self, ui),
                AppTab::Workshop => crate::ui::view_workshop::render_workshop_view(self, ui),
                AppTab::Translations => crate::ui::view_translations::render_translations_view(self, ui),
                // Long pages scroll as a whole, so nothing below the window edge is cut off.
                AppTab::Optimizer => {
                    egui::ScrollArea::vertical()
                        .id_salt("optimizer_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| crate::ui::view_optimizer::render_optimizer_view(self, ui));
                }
                AppTab::Analyzer => crate::ui::view_analyzer::render_analyzer_view(self, ui),
                AppTab::Settings => crate::ui::view_settings::render_settings_view(self, ui),
            });

        // The app-wide search (Ctrl+K or the side bar's magnifier) over everything else.
        crate::ui::view_search::render(self, &root.ctx().clone());

        // Ctrl+Z / Ctrl+Y on the Mods tab, unless a text field has the keyboard.
        if self.current_tab == AppTab::Mods && !ctx.egui_wants_keyboard_input() {
            let (redo, undo) = ctx.input_mut(|i| {
                let redo = i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
                (redo, i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z))
            });
            if redo {
                self.redo_active();
            } else if undo {
                self.undo_active();
            }
        }
        self.track_active_changes();
        // The newest save's mods are needed for the "used by the last save" warning.
        if self.saves_loaded_at.is_none() {
            self.refresh_saves();
        }

        // Closing the window with an unsaved load order: ask first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.has_unsaved_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_question = Some(PendingQuestion::CloseUnsaved);
        }

        self.render_delete_confirmation(ctx);
        self.render_question(ctx);
        self.render_history_window(ctx);
        self.render_modlist_dialog(ctx);
        self.render_save_dialog(ctx);
        crate::ui::view_bisect::render_bisect_window(self, ctx);
        crate::ui::view_name_search::render_name_search_window(self, ctx);
        crate::ui::view_changelog::render_changelog_window(self, ctx);
        crate::ui::view_steamcmd::render_steamcmd_window(self, ctx);
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

        let (title, body, button) = match pending.kind {
            DeleteKind::Unsubscribe => (
                TranslationKey::ConfirmUnsubscribeTitle,
                TranslationKey::ConfirmUnsubscribeBody,
                TranslationKey::ConfirmUnsubscribeBtn,
            ),
            DeleteKind::Reinstall => (
                TranslationKey::ConfirmReinstallTitle,
                TranslationKey::ConfirmReinstallBody,
                TranslationKey::ConfirmReinstallBtn,
            ),
            DeleteKind::Freeze => (
                TranslationKey::ConfirmFreezeTitle,
                TranslationKey::ConfirmFreezeBody,
                TranslationKey::ConfirmFreezeBtn,
            ),
            DeleteKind::Unfreeze => (
                TranslationKey::ConfirmUnfreezeTitle,
                TranslationKey::ConfirmUnfreezeBody,
                TranslationKey::ConfirmUnfreezeBtn,
            ),
            _ => (
                TranslationKey::ConfirmDeleteTitle,
                TranslationKey::ConfirmDeleteBody,
                TranslationKey::ConfirmDeleteBtn,
            ),
        };
        let (_, max_height) = crate::ui::components::dialog_size(ctx, 560.0);
        egui::Window::new(t.get(title))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .max_height(max_height)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label(fmt(t.get(body), &[&pending.names.len().to_string()]));
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
                    if crate::ui::components::danger_button(ui, t.get(button)).clicked() {
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
        let vram_waylith = crate::optimizer::engine_wants_flipped(self.steam_paths.engine);
        #[derive(PartialEq)]
        enum Answer {
            None,
            Primary,
            Secondary,
            Cancel,
        }
        let mut answer = if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            Answer::Cancel
        } else {
            Answer::None
        };

        let (title, body, primary, secondary): (&str, String, &str, &str) = match &question {
            PendingQuestion::LaunchUnsaved => (
                tr!("Порядок модов не сохранён", "Load order not saved", "加载顺序未保存"),
                tr!(
                    "Игра загрузит порядок из ModsConfig.xml, а ваши изменения ещё не сохранены.",
                    "The game loads the order from ModsConfig.xml, and your changes are not saved yet.", "游戏从 ModsConfig.xml 读取加载顺序，而你的修改还没有保存。"
                )
                .to_string(),
                tr!("Сохранить и играть", "Save and play", "保存并运行游戏"),
                tr!("Играть без сохранения", "Play without saving", "不保存直接运行"),
            ),
            PendingQuestion::CloseUnsaved => (
                tr!("Порядок модов не сохранён", "Load order not saved", "加载顺序未保存"),
                tr!(
                    "Если закрыть программу сейчас, изменения порядка модов пропадут.",
                    "If you close the app now, your load order changes are lost.", "如果现在关闭程序，加载顺序的修改将会丢失。"
                )
                .to_string(),
                tr!("Сохранить и выйти", "Save and exit", "保存并退出"),
                tr!("Выйти без сохранения", "Exit without saving", "不保存直接退出"),
            ),
            PendingQuestion::LegacyDds(paths) => (
                tr!("DDS не из журнала программы", "DDS files not in the manager's log", "不在程序记录中的 DDS 文件"),
                tr!(
                    "Откат удалил все DDS, которые создала эта программа. Найдено ещё {}: их сделала старая версия оптимизатора, RimPy или автор мода. У каждого рядом лежит PNG/JPG, поэтому моды будут работать и без них. Авторские DDS лучше оставить.",
                    "Rollback removed every DDS this manager created. {} more were found: made by an older optimizer version, RimPy or the mod author. Each has a PNG/JPG next to it, so mods work without them. Better keep the author's DDS.", "回滚已删除本程序创建的所有 DDS。另外还找到 {} 个：来自旧版优化器、RimPy 或模组作者。它们旁边都有 PNG/JPG，所以没有它们模组也能正常工作。作者自带的 DDS 最好保留。",
                    paths.len()
                ),
                tr!("Оставить", "Keep them", "保留"),
                tr!("Удалить их тоже", "Delete them too", "也一起删除"),
            ),
            PendingQuestion::RestoreVersion(id) => {
                let name = self.mod_display_name(id);
                let date = self
                    .saved_version(id)
                    .map(|v| crate::ui::components::local_date_time(v.saved_at))
                    .unwrap_or_default();
                let subscription = self
                    .find_mod(id)
                    .is_some_and(|m| m.origin == crate::models::ModOrigin::Workshop);
                let body = if subscription {
                    TranslationKey::VersionsRestoreBodyWorkshop
                } else {
                    TranslationKey::VersionsRestoreBodyLocal
                };
                (
                    self.i18n.get(TranslationKey::VersionsRestoreTitle),
                    fmt(self.i18n.get(body), &[&name, &date]),
                    self.i18n.get(TranslationKey::VersionsRestoreBtn),
                    self.i18n.get(TranslationKey::BtnCancel),
                )
            }
            PendingQuestion::RemovedFromSave { save, names } => {
                const SHOWN: usize = 10;
                let mut list: Vec<String> = names.iter().take(SHOWN).map(|n| format!("• {n}")).collect();
                if names.len() > SHOWN {
                    list.push(fmt(
                        self.i18n.get(TranslationKey::DepMoreMissing),
                        &[&(names.len() - SHOWN).to_string()],
                    ));
                }
                (
                    self.i18n.get(TranslationKey::SaveModsOffTitle),
                    format!(
                        "{}\n\n{}",
                        fmt(self.i18n.get(TranslationKey::SaveModsOffBody), &[save, &names.len().to_string()]),
                        list.join("\n")
                    ),
                    self.i18n.get(TranslationKey::SaveModsOffUndo),
                    self.i18n.get(TranslationKey::SaveModsOffKeep),
                )
            }
            PendingQuestion::RestartAfterImport => (
                self.i18n.get(TranslationKey::SettingsBackupRestartTitle),
                self.i18n.get(TranslationKey::SettingsBackupRestartBody).to_string(),
                self.i18n.get(TranslationKey::SettingsBackupRestartNow),
                self.i18n.get(TranslationKey::SettingsBackupRestartLater),
            ),
            PendingQuestion::VramWarning {
                estimated_bytes,
                total_vram_bytes,
                measured,
                ..
            } => {
                let est_gb = (*estimated_bytes as f64) / 1_073_741_824.0;
                let tot_gb = (*total_vram_bytes as f64) / 1_073_741_824.0;
                let pct = ((*estimated_bytes as f64 / *total_vram_bytes as f64) * 100.0).round() as u64;
                // On 1.7 the optimizer is off (the game compresses textures itself).
                let (body, primary, secondary) = if vram_waylith {
                    (
                        TranslationKey::VramWarningBodyWaylith,
                        TranslationKey::VramBtnLaunchAnyway,
                        TranslationKey::BtnCancel,
                    )
                } else {
                    (
                        if *measured {
                            TranslationKey::VramWarningBodyMeasured
                        } else {
                            TranslationKey::VramWarningBody
                        },
                        TranslationKey::VramBtnOptimize,
                        TranslationKey::VramBtnLaunchAnyway,
                    )
                };
                (
                    self.i18n.get(TranslationKey::VramWarningTitle),
                    fmt(
                        self.i18n.get(body),
                        &[&format!("{:.1}", est_gb), &format!("{:.1}", tot_gb), &pct.to_string()],
                    ),
                    self.i18n.get(primary),
                    self.i18n.get(secondary),
                )
            }
        };

        let (width, max_height) = crate::ui::components::dialog_size(ctx, 560.0);
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .max_height(max_height)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.set_max_width(width);
                ui.label(body);
                if let PendingQuestion::LegacyDds(paths) = &question {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .id_salt("unlogged_dds_list")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for p in paths {
                                ui.label(
                                    egui::RichText::new(p.display().to_string())
                                        .size(11.0)
                                        .color(THEME.text_muted),
                                );
                            }
                        });
                }
                if let PendingQuestion::VramWarning { heavy_mods, .. } = &question {
                    if !heavy_mods.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(self.i18n.get(TranslationKey::VramHeavyModsTitle))
                                .strong()
                                .color(THEME.accent_amber),
                        );
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_salt("vram_heavy_mods_list")
                            .max_height(140.0)
                            .show(ui, |ui| {
                                for (name, bytes) in heavy_mods {
                                    let mb = (*bytes as f64 / 1_048_576.0).round() as u64;
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(format!("• {}", name)).color(THEME.text_primary));
                                        ui.label(
                                            egui::RichText::new(fmt(
                                                self.i18n.get(TranslationKey::VramModMb),
                                                &[&mb.to_string()],
                                            ))
                                            .color(THEME.text_muted)
                                            .size(11.0),
                                        );
                                    });
                                }
                            });
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if crate::ui::components::primary_button(ui, primary).clicked() {
                        answer = Answer::Primary;
                    }
                    if crate::ui::components::secondary_button(ui, secondary).clicked() {
                        answer = Answer::Secondary;
                    }
                    let own_cancel = matches!(
                        question,
                        PendingQuestion::LegacyDds(_) | PendingQuestion::RestoreVersion(_)
                    ) || (vram_waylith && matches!(question, PendingQuestion::VramWarning { .. }));
                    if !own_cancel
                        && crate::ui::components::secondary_button(ui, self.i18n.get(TranslationKey::BtnCancel))
                            .clicked()
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
                    self.check_vram_and_launch();
                }
            }
            (PendingQuestion::LaunchUnsaved, Answer::Secondary) => self.check_vram_and_launch(),
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
            (PendingQuestion::RestoreVersion(id), Answer::Primary) => self.restore_version_now(&id),
            (PendingQuestion::RemovedFromSave { .. }, Answer::Primary) => self.undo_active(),
            (PendingQuestion::RestartAfterImport, Answer::Primary) => self.restart(ctx),
            (PendingQuestion::VramWarning { .. }, Answer::Primary) if vram_waylith => self.launch_game_now(),
            (PendingQuestion::VramWarning { .. }, Answer::Primary) => {
                self.current_tab = AppTab::Optimizer;
            }
            (PendingQuestion::VramWarning { .. }, Answer::Secondary) if !vram_waylith => {
                self.launch_game_now();
            }
            _ => {}
        }
    }

    /// Window for a mod set action: name field (save as new, rename) or delete confirmation.
    fn render_modlist_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.modlist_dialog.clone() else {
            return;
        };
        let t = self.i18n;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut submit = false;
        let title = match &dialog {
            ModListDialog::SaveAs { .. } => t.get(TranslationKey::SetsDialogSaveTitle),
            ModListDialog::Rename { .. } => t.get(TranslationKey::SetsDialogRenameTitle),
            ModListDialog::Delete { .. } => t.get(TranslationKey::SetsDialogDeleteTitle),
            ModListDialog::Collection { .. } => t.get(TranslationKey::SetsDialogCollectionTitle),
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
                                egui::RichText::new(t.get(TranslationKey::SetsHint))
                                    .color(THEME.text_muted)
                                    .size(12.0),
                            )
                            .wrap(),
                        );
                    }
                    ModListDialog::Delete { name } => {
                        ui.add(egui::Label::new(fmt(t.get(TranslationKey::SetsDialogDeleteBody), &[name])).wrap());
                    }
                    ModListDialog::Collection { url, error } => {
                        ui.label(t.get(TranslationKey::SetsCollectionLabel));
                        let field = ui.add(egui::TextEdit::singleline(url).desired_width(f32::INFINITY));
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
                                egui::RichText::new(t.get(TranslationKey::SetsCollectionHint))
                                    .color(THEME.text_muted)
                                    .size(12.0),
                            )
                            .wrap(),
                        );
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (text, style) = match &dialog {
                        ModListDialog::SaveAs { .. } => (
                            t.get(TranslationKey::SetsBtnSave),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                        ModListDialog::Rename { .. } => (
                            t.get(TranslationKey::SetsBtnRename),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                        ModListDialog::Delete { .. } => (
                            t.get(TranslationKey::SetsBtnDelete),
                            crate::ui::components::ButtonStyle::Danger,
                        ),
                        ModListDialog::Collection { .. } => (
                            t.get(TranslationKey::SetsBtnImport),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
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
                ModListDialog::Collection { url, .. } => self.import_steam_collection(url),
            };
            self.modlist_dialog = match (result, dialog) {
                (Ok(()), _) => None,
                (Err(e), ModListDialog::SaveAs { name, .. }) => Some(ModListDialog::SaveAs { name, error: Some(e) }),
                (Err(e), ModListDialog::Rename { from, name, .. }) => Some(ModListDialog::Rename {
                    from,
                    name,
                    error: Some(e),
                }),
                (Err(e), ModListDialog::Collection { url, .. }) => {
                    Some(ModListDialog::Collection { url, error: Some(e) })
                }
                (Err(_), other) => Some(other),
            };
        } else if close {
            self.modlist_dialog = None;
        } else {
            self.modlist_dialog = Some(dialog);
        }
    }

    /// F7: window for renaming or deleting a save file.
    fn render_save_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.save_dialog.clone() else {
            return;
        };
        let t = self.i18n;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut submit = false;
        let title = match &dialog {
            SaveDialog::Rename { .. } => t.get(TranslationKey::SavesDialogRenameTitle),
            SaveDialog::Delete { .. } => t.get(TranslationKey::SavesDialogDeleteTitle),
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(400.0);
                match &mut dialog {
                    SaveDialog::Rename { name, error, .. } => {
                        ui.label(t.get(TranslationKey::SavesNameLabel));
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
                    }
                    SaveDialog::Delete { name } => {
                        ui.add(egui::Label::new(fmt(t.get(TranslationKey::SavesDialogDeleteBody), &[name])).wrap());
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (text, style) = match &dialog {
                        SaveDialog::Rename { .. } => (
                            t.get(TranslationKey::SetsBtnRename),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                        SaveDialog::Delete { .. } => (
                            t.get(TranslationKey::SetsBtnDelete),
                            crate::ui::components::ButtonStyle::Danger,
                        ),
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
                SaveDialog::Rename { from, name, .. } => self.rename_save(from, name),
                SaveDialog::Delete { name } => {
                    self.delete_save(name);
                    Ok(())
                }
            };
            self.save_dialog = match (result, dialog) {
                (Ok(()), _) => None,
                (Err(e), SaveDialog::Rename { from, name, .. }) => Some(SaveDialog::Rename {
                    from,
                    name,
                    error: Some(e),
                }),
                (Err(_), other) => Some(other),
            };
        } else if close {
            self.save_dialog = None;
        } else {
            self.save_dialog = Some(dialog);
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

        let (width, max_height) = crate::ui::components::dialog_size(ctx, 560.0);
        egui::Window::new(tr!("История порядка модов", "Load order history", "加载顺序历史"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .max_height(max_height)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.label(
                    egui::RichText::new(tr!(
                        "Перед каждым сохранением программа делает копию ModsConfig.xml (хранятся последние 30). Восстановленный список появится как несохранённый: файл изменится, только когда вы нажмёте «Сохранить порядок».",
                        "Before every save the manager copies ModsConfig.xml (the last 30 are kept). A restored list shows up as unsaved: the file changes only when you press \"Save Order\".", "每次保存前程序都会复制一份 ModsConfig.xml（保留最近 30 份）。恢复的列表会显示为未保存：只有点击「保存排序」后文件才会改变。"
                    ))
                    .color(THEME.text_secondary)
                    .size(12.0),
                );
                ui.add_space(8.0);

                if backups.is_empty() {
                    ui.label(
                        egui::RichText::new(tr!(
                            "Копий пока нет: они появятся после первого сохранения порядка в программе.",
                            "No copies yet: they appear after the first time you save the order here.", "还没有副本：在这里第一次保存排序后就会出现。"
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
                                inner_margin: egui::Margin::symmetric(10, 6),
                                outer_margin: egui::Margin::symmetric(0, 2),
                                corner_radius: CornerRadius::same(6),
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
                                            tr!("модов: 1", "1 mod", "1 个模组").to_string()
                                        } else {
                                            tr!("модов: {}", "{} mods", "{} 个模组", b.mods.len())
                                        })
                                            .color(THEME.text_secondary)
                                            .size(12.0),
                                    );
                                    history_diff_label(ui, diff);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_enabled_ui(!diff.is_same(), |ui| {
                                            if crate::ui::components::small_button(
                                                ui,
                                                tr!("Восстановить", "Restore", "恢复"),
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
                    if crate::ui::components::secondary_button_icon(ui, ICON_FOLDER, tr!("Открыть папку", "Open folder", "打开文件夹"))
                        .clicked()
                    {
                        let dir = crate::mods_config::backups_dir();
                        let _ = std::fs::create_dir_all(&dir);
                        crate::ui::components::open_external(dir);
                    }
                    if crate::ui::components::secondary_button(ui, tr!("Закрыть", "Close", "关闭")).clicked() {
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
        ui.label(
            egui::RichText::new(tr!("как сейчас", "same as now", "与当前相同"))
                .color(THEME.text_muted)
                .size(12.0),
        );
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
        parts.push(tr!("другой порядок", "different order", "顺序不同").to_string());
    }
    ui.label(
        egui::RichText::new(parts.join(" · "))
            .color(THEME.accent_amber)
            .size(12.0),
    )
    .on_hover_text(tr!(
        "По сравнению с текущим списком: + моды, которые вернутся, − моды, которые уберутся",
        "Compared with the current list: + mods that come back, − mods that go away",
        "与当前列表对比：+ 将恢复的模组，− 将移除的模组"
    ));
}

/// A run good enough to be the "last working" one (F9): no crash, no graphics card or memory
/// failure, no broken Prepatcher / Harmony patch. Ordinary errors are allowed.
pub fn is_working_run(report: &LogReport) -> bool {
    !report.has_hard_crash
        && report.system_issue.is_none()
        && report
            .culprits
            .iter()
            .all(|c| c.severity < crate::analyzer::ErrorSeverity::PrepatcherHarmonyError)
}

/// Returns whether `log_path` belongs to a RimWorld run launched during this session.
pub fn is_verified_launch_log(
    game_launch_time: Option<std::time::SystemTime>,
    bisect_active: bool,
    has_custom_log: bool,
    log_path: &Path,
) -> bool {
    let Some(launched_at) = game_launch_time else {
        return false;
    };
    if bisect_active || has_custom_log {
        return false;
    }
    std::fs::metadata(log_path)
        .and_then(|m| m.modified())
        .is_ok_and(|mtime| mtime >= launched_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_of_the_active_list() {
        let list = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut h = ActiveHistory::default();
        assert_eq!(h.observe(&list(&["a"])), None, "the first look only remembers");
        assert_eq!(h.observe(&list(&["a"])), None);
        assert_eq!(h.observe(&list(&["a", "b"])), Some(list(&["a"])));
        h.observe(&list(&["b", "a"]));

        let back = h.undo(&list(&["b", "a"])).unwrap();
        assert_eq!(back, list(&["a", "b"]));
        assert_eq!(h.observe(&back), None, "undo itself is not a new step");
        assert_eq!(h.undo(&back), Some(list(&["a"])));
        assert_eq!(h.undo(&list(&["a"])), None);
        assert_eq!(h.redo(&list(&["a"])), Some(list(&["a", "b"])));
        // A new edit drops what could be redone.
        h.observe(&list(&["a", "b", "c"]));
        assert_eq!(h.redo(&list(&["a", "b", "c"])), None);

        // A list read from disk is not an edit.
        h.settle(&list(&["x"]));
        assert_eq!(h.observe(&list(&["x"])), None);
        for i in 0..150 {
            h.observe(&[i.to_string()]);
        }
        assert_eq!(h.undo.len(), ActiveHistory::MAX_STEPS);
    }

    #[test]
    fn mods_the_last_save_needs() {
        let list = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let before = list(&["Ludeon.RimWorld", "a.mod", "B.Mod", "c.mod"]);
        let now = list(&["ludeon.rimworld", "c.mod"]);
        let save = list(&["ludeon.rimworld", "b.mod", "c.mod", "z.mod"]);
        assert_eq!(
            turned_off_from_save(&before, &now, &save),
            ["B.Mod"],
            "a.mod isn't in the save"
        );
        assert!(
            turned_off_from_save(&now, &before, &save).is_empty(),
            "turning mods on is fine"
        );
        assert!(
            turned_off_from_save(&before, &[], &save)
                .iter()
                .all(|id| id != "Ludeon.RimWorld"),
            "Core is never in the warning"
        );
    }

    #[test]
    fn folder_size_and_date_for_sorting() {
        let dir = std::env::temp_dir().join(format!("rmm_folder_size_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Textures")).unwrap();
        std::fs::write(dir.join("About.xml"), [0u8; 100]).unwrap();
        std::fs::write(dir.join("Textures").join("a.dds"), [0u8; 400]).unwrap();
        assert_eq!(folder_size(&dir), 500, "files below the folder count too");
        assert!(folder_added(&dir).is_some_and(|t| t > 1_700_000_000));
        assert_eq!(folder_size(&dir.join("missing")), 0);
        assert_eq!(folder_added(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(InactiveSort::Subscribers.needs_workshop() && InactiveSort::Updated.needs_workshop());
        assert!(!InactiveSort::Size.needs_workshop());
    }

    #[test]
    fn dlcs_a_mod_requires() {
        let m = ModInfo {
            dependencies: vec![
                " Ludeon.RimWorld.Biotech ".into(),
                "brrainz.harmony".into(),
                "ludeon.rimworld.royalty".into(),
                "ludeon.rimworld".into(),
            ],
            ..Default::default()
        };
        assert_eq!(
            required_dlcs(&m),
            ["ludeon.rimworld.royalty", "ludeon.rimworld.biotech"],
            "DLC order, Core is not a DLC"
        );
        assert_eq!(dlc_name("ludeon.rimworld.biotech"), "Biotech");
        assert!(required_dlcs(&ModInfo::default()).is_empty());
    }

    /// "Auto-fix" on the real mods and the saved list, in memory only (run by hand:
    /// `cargo test live_auto_fix -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_auto_fix() {
        let paths = detect_steam_paths();
        let scanned = scan_mods(
            paths.workshop_path.as_deref(),
            paths.local_mods_path.as_deref(),
            paths.game_data_path.as_deref(),
        );
        let (mut mods, _) = crate::scanner::dedupe_mods(scanned);
        crate::scanner::resolve_versioned_fields(&mut mods, paths.game_major_minor().as_deref());
        classify_and_link_translations(&mut mods);
        let config = get_default_mods_config_path().expect("ModsConfig.xml");
        let mut active = load_active_mods(&config);
        let db = crate::community_rules::load_effective_db();
        let show = |label: &str, active: &[String]| {
            let report = validate_dependencies(active, &mods);
            println!(
                "{label}: {} mods | missing {} disabled {} order {} incompatible {} | button shown: {}",
                active.len(),
                report.missing_count(),
                report.disabled_count(),
                report.order_count(),
                report.incompatible_count(),
                report.auto_fixable()
            );
            for p in report.problems.iter().take(12) {
                println!("  {p:?}");
            }
        };
        show("before", &active);
        let added = enable_dependencies_recursive(&mut active, &mods);
        let sorted = sort_load_order_with_db(&active, &mods, Some(&db));
        println!("enabled {added}");
        show("after one click", &sorted);
        let mut again = sorted.clone();
        enable_dependencies_recursive(&mut again, &mods);
        let again = sort_load_order_with_db(&again, &mods, Some(&db));
        println!("second click changes the order: {}", again != sorted);
    }

    #[test]
    fn a_collection_becomes_a_modpack_with_core_and_dlc_in_load_order() {
        let installed = |id: &str, name: &str| ModInfo {
            package_id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let all_mods = vec![
            installed("Ludeon.RimWorld.Royalty", "Royalty"),
            installed("brrainz.harmony", "Harmony"),
            installed("Ludeon.RimWorld", "Core"),
        ];
        let entry = |id: &str| crate::modlists::ModListEntry {
            id: id.into(),
            name: id.into(),
            steam_id: Some("1".into()),
        };
        // The second mod isn't downloaded yet.
        let pack = modpack_from_collection(vec![entry("author.mod"), entry("brrainz.harmony")], &all_mods, None);
        let ids: Vec<&str> = pack.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "brrainz.harmony",
                "ludeon.rimworld",
                "ludeon.rimworld.royalty",
                "author.mod"
            ]
        );
        assert_eq!((pack[1].name.as_str(), &pack[1].steam_id), ("Core", &None));

        // A list that already has Core keeps its one entry.
        let pack = modpack_from_collection(vec![entry("Ludeon.RimWorld"), entry("author.mod")], &all_mods, None);
        assert_eq!(
            pack.iter()
                .filter(|e| e.id.eq_ignore_ascii_case("ludeon.rimworld"))
                .count(),
            1
        );
    }

    #[test]
    fn lookup_by_index_understands_case_and_steam_suffix() {
        let mods = vec![
            ModInfo {
                package_id: "Brrainz.Harmony".into(),
                ..Default::default()
            },
            ModInfo {
                package_id: "UnlimitedHugs.HugsLib".into(),
                ..Default::default()
            },
        ];
        let index: std::collections::HashMap<String, usize> = mods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.package_id.to_lowercase(), i))
            .collect();
        assert_eq!(
            lookup_mod(&index, &mods, "brrainz.harmony").map(|m| m.package_id.as_str()),
            Some("Brrainz.Harmony")
        );
        assert_eq!(
            lookup_mod(&index, &mods, "unlimitedhugs.hugslib_steam").map(|m| m.package_id.as_str()),
            Some("UnlimitedHugs.HugsLib")
        );
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
        assert_eq!(
            active_after_rescan(true, false, &disk),
            None,
            "unsaved changes are kept"
        );
        assert_eq!(
            active_after_rescan(false, false, &disk),
            Some(disk.clone()),
            "no changes: follow the file"
        );
        assert_eq!(
            active_after_rescan(true, true, &disk),
            Some(disk),
            "other config picked: file wins"
        );
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

    #[test]
    fn verified_launch_log_rejects_unlaunched_or_stale_logs() {
        let dir = std::env::temp_dir().join(format!("rwmm_test_log_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log_path = dir.join("Player.log");
        std::fs::write(&log_path, "Mono path[0] = ...\n").unwrap();
        let mtime = std::fs::metadata(&log_path).unwrap().modified().unwrap();

        // Not launched in this session -> never verified (e.g. startup auto-check).
        assert!(!is_verified_launch_log(None, false, false, &log_path));

        // Log file older than launch timestamp -> stale log from a previous run.
        let future_launch = mtime + std::time::Duration::from_secs(10);
        assert!(!is_verified_launch_log(Some(future_launch), false, false, &log_path));

        // Fresh log after launch -> verified, unless bisect or custom log is active.
        let past_launch = mtime - std::time::Duration::from_secs(2);
        assert!(is_verified_launch_log(Some(past_launch), false, false, &log_path));
        assert!(!is_verified_launch_log(Some(past_launch), true, false, &log_path));
        assert!(!is_verified_launch_log(Some(past_launch), false, true, &log_path));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
