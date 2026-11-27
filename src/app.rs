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

mod analyzer_tab;
mod bisect_tab;
mod modpacks_tab;
mod optimizer_tab;
mod saves_tab;
mod steamcmd_tab;
mod translations_tab;
mod workshop_tab;

pub use analyzer_tab::AnalyzerState;
pub use bisect_tab::BisectUiState;
pub use modpacks_tab::ModpacksState;
pub use optimizer_tab::OptimizerState;
pub use saves_tab::SavesState;
pub use steamcmd_tab::SteamCmdState;
pub use translations_tab::TranslationsState;
pub use workshop_tab::WorkshopTabState;

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

    // Translations tab: the manual Workshop search and the modpack-wide one.
    pub translations: TranslationsState,

    // Start views for screenshots (RWMM_TAB).
    /// RWMM_TAB=saves-findname: "find by name" for the selected save once the mods and the
    /// saves are read (screenshots).
    name_search_after_load: bool,
    /// RWMM_TAB=mods-updates: check the Workshop for updates once the mods are scanned (screenshots).
    updates_after_scan: bool,

    // Optimizer tab and the VRAM check.
    pub optimizer: OptimizerState,

    // Log Analyzer: the log report, the companion's reports, the last working run.
    pub analyzer: AnalyzerState,

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
    /// Modpacks tab (F4): the sets, their pictures, tags and dialogs.
    pub modpacks: ModpacksState,
    /// Mods already warned about in this session: enabled without declaring the game version.
    warned_no_version: std::collections::HashSet<String>,
    /// The "What changed" window of a Workshop mod, and its notes being downloaded.
    pub changelog: Option<ChangelogWindow>,
    changelog_rx: Option<Receiver<ChangelogLoaded>>,

    // Saves (F7): headers of the game's saves, read in the background.
    pub saves: SavesState,

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

    // Broken-mod search (F8).
    pub bisect: BisectUiState,

    /// "History" window: ModsConfig.xml backups, newest first (`Some` = window open).
    pub history: Option<Vec<crate::mods_config::ConfigBackup>>,
    /// Look for a new version on GitHub once a day (Settings).
    pub check_updates: bool,
    /// Workshop tab may ask the Steam client when the website refuses (Settings).
    pub workshop_via_steam_client: bool,
    /// SteamCMD downloads: settings, the running job, its window.
    pub steamcmd: SteamCmdState,

    // Workshop tab.
    pub workshop: WorkshopTabState,

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

        let bisect_session = bisect_tab::load_bisect_session();
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

            translations: TranslationsState {
                target_lang: TranslationTargetLang::Ru,
                search_query: String::new(),
                search_results: Vec::new(),
                search_rx: None,
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
            },
            bisect: BisectUiState {
                open_after_scan: false,
                // An unfinished search opens again where it stopped.
                open: bisect_session.is_some(),
                session: bisect_session,
                keep: None,
                filter: String::new(),
                note: None,
                watch_rx: None,
            },
            analyzer: AnalyzerState {
                report: None,
                companion_report: crate::companion_report::load(),
                tick_profile: crate::tick_profile::load(),
                measure_next_launch: false,
                odd_textures: HashMap::new(),
                odd_textures_rx: None,
                odd_textures_for: None,
                texture_sides_cache: None,
                custom_log_path: None,
                last_good: crate::last_good::LastGoodSnapshot::load(),
                last_good_diff: None,
                last_good_rx: None,
                last_good_save_rx: None,
                last_good_rerun: false,
                filter_text: String::new(),
                level_filter: crate::analyzer::LogLevelFilter::default(),
                scroll_to_companion_report: false,
                rx: None,
                silent: false,
                initial_log_checked: false,
            },
            name_search_after_load: false,
            updates_after_scan: false,

            optimizer: OptimizerState {
                scope: OptimizeScope::AllInstalled,
                max_size: s.opt_max_size.unwrap_or_default(),
                force: false,
                delete_source: false,
                threads: default_threads,
                max_system_threads,
                is_running: false,
                done: 0,
                total: 0,
                errors: 0,
                current_file: String::new(),
                stop_flag: Arc::new(AtomicBool::new(false)),
                rx: None,
                ignore_list: opt_ignore_list,
                wrong_orientation: 0,
                orient_rx: None,
                scan_rx: None,
                rollback_rx: None,
                vram_report: None,
                vram_rx: None,
                vram_cache: HashMap::new(),
                vram_recheck: false,
                launch_after_vram_check: false,
            },

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
            modpacks: ModpacksState {
                lists: Vec::new(),
                pictures: HashMap::new(),
                tags: crate::modlist_tags::load(&crate::modlist_tags::tags_path()),
                tag_filter: std::collections::HashSet::new(),
                current: None,
                // RWMM_SET=<name> opens that modpack on the Modpacks tab (screenshots).
                selected: std::env::var("RWMM_SET").ok().filter(|s| !s.is_empty()),
                search: String::new(),
                dialog: None,
                collection_rx: None,
                settings: HashMap::new(),
                compare: None,
            },
            warned_no_version: std::collections::HashSet::new(),
            changelog: None,
            changelog_rx: None,
            saves: SavesState {
                list: Vec::new(),
                rx: None,
                loaded_at: None,
                // RWMM_SAVE=<name> opens that save on the Saves tab (screenshots).
                selected: std::env::var("RWMM_SAVE").ok().filter(|s| !s.is_empty()),
                search: String::new(),
                dialog: None,
            },
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
            check_updates: last_check_updates,
            workshop_via_steam_client,
            steamcmd: SteamCmdState {
                enabled: use_steamcmd,
                enabled_set: use_steamcmd_set,
                custom_dir: custom_steamcmd_dir,
                job: None,
                rx: None,
                kill: Arc::new(std::sync::Mutex::new(None)),
                window: false,
                fresh_install: false,
                console: false,
                console_input: String::new(),
            },
            workshop: WorkshopTabState {
                query: crate::workshop_browse::BrowseQuery {
                    page: 1,
                    ..Default::default()
                },
                search: String::new(),
                items: Vec::new(),
                selected: None,
                error: None,
                rx: None,
                source: crate::workshop_browse::Source::Web,
                loaded: false,
                collection: None,
                collection_rx: None,
                requested: std::collections::HashSet::new(),
                previews: std::collections::HashMap::new(),
                preview_tx: None,
                preview_rx: None,
            },
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
        app.translations.batch_search_after_scan = start_view == "translations-batch";
        app.bisect.open_after_scan = start_view == "bisect";
        app.name_search_after_load = start_view == "saves-findname";
        app.updates_after_scan = start_view == "mods-updates";
        app.analyzer.scroll_to_companion_report = start_view == "analyzer-companion";
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
        self.poll_saves();
        if let Some(rx) = &self.rescan_rx {
            if let Ok(res) = rx.try_recv() {
                self.rescan_rx = None;
                self.apply_rescan(res);
            }
        }
        self.poll_translation_search();
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
        if let Some(rx) = &self.analyzer.rx {
            if let Ok(report) = rx.try_recv() {
                self.analyzer.rx = None;
                // F9 / F11 are recorded only from the launch watcher, once the game has
                // closed: a log analyzed by hand may be of a game still loading.
                if !std::mem::take(&mut self.analyzer.silent) {
                    self.status_message = self.crash_summary_text(&report);
                }
                self.analyzer.report = Some(report);
                self.analyzer.companion_report = crate::companion_report::load();
                self.analyzer.tick_profile = crate::tick_profile::load();
            }
        }
        if let Some(rx) = &self.launch_watch_rx {
            if let Ok(opt_report) = rx.try_recv() {
                self.launch_watch_rx = None;
                if let Some(report) = opt_report {
                    self.handle_verified_launch_report(&report);
                    self.status_message = self.crash_summary_text(&report);
                    self.analyzer.report = Some(report);
                    self.analyzer.companion_report = crate::companion_report::load();
                    self.analyzer.tick_profile = crate::tick_profile::load();
                }
            }
        }
        self.poll_bisect_watch();
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
        if let Some(rx) = &self.modpacks.collection_rx {
            if let Ok(res) = rx.try_recv() {
                self.modpacks.collection_rx = None;
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
        if let Some(ref report) = self.analyzer.report {
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
            opt_threads: Some(self.optimizer.threads),
            opt_max_size: Some(self.optimizer.max_size),
            check_updates: self.check_updates,
            use_steamcmd: self.steamcmd.enabled_set.then_some(self.steamcmd.enabled),
            steamcmd_dir: self.steamcmd.custom_dir.clone(),
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
        self.optimizer.wrong_orientation = res.optimizer_dds;
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
        if std::mem::take(&mut self.translations.batch_search_after_scan) {
            self.start_batch_translation_search();
        }
        if std::mem::take(&mut self.bisect.open_after_scan) {
            self.open_bisect();
        }
        if std::mem::take(&mut self.updates_after_scan) {
            self.check_mod_updates();
        }
        if !self.analyzer.initial_log_checked && self.analyzer.report.is_none() && self.analyzer.rx.is_none() {
            self.analyzer.initial_log_checked = true;
            let log_target = self
                .analyzer
                .custom_log_path
                .clone()
                .or_else(|| detect_rimworld_log_paths().into_iter().next().map(|(_, p)| p));
            if let Some(path) = log_target {
                let mods = self.all_mods.clone();
                self.analyzer.silent = true;
                self.analyzer.rx = Some(self.spawn_task(move || analyze_rimworld_log(&path, &mods)));
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
                self.workshop
                    .items
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

    /// The game's Mods folder, where links to downloaded mods go.
    pub fn game_mods_dir(&self) -> Option<PathBuf> {
        self.steam_paths
            .local_mods_path
            .clone()
            .or_else(|| self.steam_paths.game_path.as_ref().map(|g| g.join("Mods")))
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
        if !self.name_search_after_load || self.all_mods.is_empty() || self.saves.list.is_empty() {
            return;
        }
        self.name_search_after_load = false;
        let name = self
            .saves
            .selected
            .clone()
            .or_else(|| self.saves.list.first().map(|s| s.name.clone()));
        let Some(save) = name.and_then(|n| self.saves.list.iter().find(|s| s.name == n)) else {
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

    // ----- Watching the game after a launch -----

    /// Spawns a background task that waits for RimWorld to exit and then analyzes a fresh Player.log.
    fn spawn_game_log_watch(&self) -> Option<Receiver<Option<LogReport>>> {
        let log_target = self
            .analyzer
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

    /// Watches for RimWorld to exit after a normal launch so F9 / F11 only record verified runs.
    fn start_launch_game_watch(&mut self) {
        self.launch_watch_rx = self.spawn_game_log_watch();
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
        self.steam_rx.is_some() || self.steamcmd.rx.is_some()
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

    /// "Play": with an unsaved load order, asks first; then checks VRAM before launch (V2).
    pub fn launch_game(&mut self) {
        if self.has_unsaved_changes() {
            self.pending_question = Some(PendingQuestion::LaunchUnsaved);
        } else {
            self.check_vram_and_launch();
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
        if std::mem::take(&mut self.analyzer.measure_next_launch) {
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
                if self.bisect.session.is_none() {
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
        if self.optimizer.scan_rx.is_some() || self.optimizer.rollback_rx.is_some() || self.optimizer.vram_rx.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Receive background optimizer messages
        let mut opt_finished = false;
        if let Some(ref rx) = self.optimizer.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    OptimizationMsg::Progress {
                        done,
                        total,
                        current_file,
                        ..
                    } => {
                        self.optimizer.done = done;
                        self.optimizer.total = total;
                        self.optimizer.current_file = current_file;
                    }
                    OptimizationMsg::FileError(_) => {
                        self.optimizer.errors += 1;
                    }
                    OptimizationMsg::Done(s) => {
                        self.optimizer.is_running = false;
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
                        self.optimizer.is_running = false;
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
                let bisect_text = (self.bisect.session.is_some() && !self.bisect.open).then(|| {
                    let step = self.bisect.session.as_ref().map_or(1, |s| s.stage.answers + 1);
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
                    AppTab::Sets => Some((self.modpacks.lists.len(), false)).filter(|(n, _)| *n > 0),
                    AppTab::Saves => Some((self.saves.list.len(), false)).filter(|(n, _)| *n > 0),
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
        if self.saves.loaded_at.is_none() {
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

        if self.optimizer.is_running {
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
