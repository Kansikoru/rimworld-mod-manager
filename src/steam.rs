use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

use crate::game_install::{self, GameEngine};
use crate::i18n::tr;

pub const RIMWORLD_APP_ID: u32 = 294100;

#[derive(Debug, Clone, Default)]
pub struct SteamPaths {
    pub steam_path: Option<PathBuf>,
    pub workshop_path: Option<PathBuf>,
    pub local_mods_path: Option<PathBuf>,
    pub game_data_path: Option<PathBuf>,
    pub game_path: Option<PathBuf>,
    pub steam_dll_path: Option<PathBuf>,
    /// Unity (1.6 and older) or Waylith (1.7+).
    pub engine: GameEngine,
    /// Contents of Version.txt, e.g. "1.7.4985 rev1269".
    pub game_version: Option<String>,
    pub exe_path: Option<PathBuf>,
}

impl SteamPaths {
    /// Recomputes everything that depends on the game folder (Mods, Data, exe, Steam DLL, engine).
    pub fn set_game_dir(&mut self, game: PathBuf) {
        let local_mods = game.join("Mods");
        self.local_mods_path = local_mods.is_dir().then_some(local_mods);
        let game_data = game.join("Data");
        self.game_data_path = game_data.is_dir().then_some(game_data);

        self.engine = game_install::detect_engine(&game);
        self.game_version = game_install::read_game_version(&game);
        self.exe_path = game_install::game_exe_path(&game, self.engine);
        self.steam_dll_path = game_install::game_steam_dll_path(&game).or_else(fallback_steam_dll);
        self.game_path = Some(game);
    }

    /// "1.7", "1.6", ... from Version.txt.
    pub fn game_major_minor(&self) -> Option<String> {
        self.game_version.as_deref().and_then(game_install::major_minor)
    }
}

/// steam_api64.dll next to Steam itself or next to this manager's exe.
fn fallback_steam_dll() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(s_dir) = get_steam_install_dir() {
        candidates.push(s_dir.join("steam_api64.dll"));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join("steam_api64.dll"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn detect_steam_paths() -> SteamPaths {
    let mut paths = SteamPaths::default();

    let steam_dir = get_steam_install_dir();
    paths.steam_path.clone_from(&steam_dir);

    let mut libraries = Vec::new();
    if let Some(ref s_dir) = steam_dir {
        libraries.push(s_dir.clone());
        let vdf_path = s_dir.join("steamapps").join("libraryfolders.vdf");
        if vdf_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&vdf_path) {
                for lib in parse_libraryfolders_vdf(&content) {
                    if !libraries.contains(&lib) {
                        libraries.push(lib);
                    }
                }
            }
        }
    }

    for lib in &libraries {
        let game = lib.join("steamapps").join("common").join("RimWorld");
        let acf = lib
            .join("steamapps")
            .join("workshop")
            .join(format!("appworkshop_{RIMWORLD_APP_ID}.acf"));

        if game.is_dir() || acf.is_file() {
            if paths.game_path.is_none() && game.is_dir() {
                paths.set_game_dir(game.clone());
            }

            let workshop = lib
                .join("steamapps")
                .join("workshop")
                .join("content")
                .join(RIMWORLD_APP_ID.to_string());
            if workshop.is_dir() && paths.workshop_path.is_none() {
                paths.workshop_path = Some(workshop);
            }
        }
    }

    if paths.steam_dll_path.is_none() {
        paths.steam_dll_path = fallback_steam_dll();
    }

    paths
}

fn get_steam_install_dir() -> Option<PathBuf> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(steam_key) = hkcu.open_subkey(r"Software\Valve\Steam") {
        if let Ok(val) = steam_key.get_value::<String, _>("SteamPath") {
            let p = PathBuf::from(val);
            if p.exists() {
                return Some(p);
            }
        }
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(steam_key) = hklm.open_subkey(r"SOFTWARE\WOW6432Node\Valve\Steam") {
        if let Ok(val) = steam_key.get_value::<String, _>("InstallPath") {
            let p = PathBuf::from(val);
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

static LIBRARYFOLDERS_PATH_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r#""path"\s+"([^"]+)""#).unwrap());

pub fn parse_libraryfolders_vdf(content: &str) -> Vec<PathBuf> {
    let mut libs = Vec::new();
    for cap in LIBRARYFOLDERS_PATH_RE.captures_iter(content) {
        if let Some(m) = cap.get(1) {
            let path_str = m.as_str().replace(r"\\", r"\");
            let p = PathBuf::from(path_str);
            if p.is_dir() {
                libs.push(p);
            }
        }
    }
    libs
}

/// One Steam API session at a time: two threads starting and shutting down the API
/// in the same process at once (e.g. a delete and a subscribe) is not safe.
static STEAM_API_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What a Steam API session is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    /// Requests the Steam client carries out on its own (subscribe, unsubscribe...): waits
    /// for the API however long, and gives Steam a moment to receive them before shutdown.
    Requests,
    /// A query that waits for its own answer: gives up when another session keeps the API
    /// (a download holds it for minutes).
    Query,
}

/// How long a query waits for another Steam API session to end.
const QUERY_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

fn session_lock(session: Session) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    session_lock_within(session, QUERY_LOCK_WAIT)
}

fn session_lock_within(
    session: Session,
    wait: std::time::Duration,
) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    if session == Session::Requests {
        return Ok(STEAM_API_LOCK.lock().unwrap_or_else(|e| e.into_inner()));
    }
    let started = std::time::Instant::now();
    loop {
        match STEAM_API_LOCK.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(e)) => return Ok(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if started.elapsed() < wait => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(crate::i18n::text(crate::i18n::TranslationKey::WsSteamBusy).to_string());
            }
        }
    }
}

struct SteamShutdownGuard<'a> {
    lib: &'a libloading::Library,
}

impl Drop for SteamShutdownGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            if let Ok(shutdown) = self.lib.get::<unsafe extern "C" fn()>(b"SteamAPI_Shutdown") {
                shutdown();
            }
        }
    }
}

/// Steam API return code meaning "OK" for `SteamAPI_InitFlat` (ESteamAPIInitResult).
const STEAM_INIT_OK: i32 = 0;

/// How long to pump `SteamAPI_RunCallbacks` after sending requests, so they reach the
/// Steam client before `SteamAPI_Shutdown` (the calls are asynchronous).
const CALLBACK_PUMP: std::time::Duration = std::time::Duration::from_millis(1500);

type UgcFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> u64;

/// Starts the Steam API with the game's app id, gets the UGC (Workshop) interface,
/// runs `action`, lets Steam process the requests and shuts the API down.
///
/// Handles both DLL generations:
/// - new SDKs export `SteamAPI_InitFlat`, which returns an error code (0 = OK, not a bool);
/// - older ones (RimWorld 1.6 and 1.7 ship 6.91.21.57) export `SteamAPI_InitSafe` / `SteamAPI_Init`.
///
/// The UGC accessor is versioned (`SteamAPI_SteamUGC_v016` today); several versions are tried.
unsafe fn with_steam_ugc<F, R>(dll_path: &Path, session: Session, action: F) -> Result<R, String>
where
    F: FnOnce(*mut std::ffi::c_void, &libloading::Library) -> Result<R, String>,
{
    let _session = session_lock(session)?;
    let lib = libloading::Library::new(dll_path).map_err(|e| e.to_string())?;

    unsafe {
        std::env::set_var("SteamAppId", RIMWORLD_APP_ID.to_string());
    }

    if let Ok(init_flat) = lib.get::<unsafe extern "C" fn(*mut u8) -> i32>(b"SteamAPI_InitFlat") {
        let mut err_buf = [0u8; 1024];
        let code = init_flat(err_buf.as_mut_ptr());
        if code != STEAM_INIT_OK {
            let len = err_buf.iter().position(|&b| b == 0).unwrap_or(err_buf.len());
            let msg = String::from_utf8_lossy(&err_buf[..len]).to_string();
            return Err(tr!(
                "Steam API не запустился (код {}): {}",
                "Steam API failed to start (code {}): {}",
                "Steam API 启动失败（代码 {}）：{}",
                code,
                msg
            ));
        }
    } else {
        let init: libloading::Symbol<unsafe extern "C" fn() -> bool> = lib
            .get(b"SteamAPI_InitSafe")
            .or_else(|_| lib.get(b"SteamAPI_Init"))
            .map_err(|e| e.to_string())?;
        if !init() {
            return Err(tr!(
                "Не удалось запустить Steam API (возможно, клиент Steam закрыт)",
                "Could not start the Steam API (is the Steam client running?)",
                "无法启动 Steam API（Steam 客户端是否在运行？）"
            )
            .to_string());
        }
    }

    let _guard = SteamShutdownGuard { lib: &lib };

    let mut ugc_ptr = std::ptr::null_mut();
    for version in (12..=30).rev() {
        let name = format!("SteamAPI_SteamUGC_v{version:03}\0");
        if let Ok(f) = lib.get::<unsafe extern "C" fn() -> *mut std::ffi::c_void>(name.as_bytes()) {
            ugc_ptr = f();
            break;
        }
    }
    if ugc_ptr.is_null() {
        if let Ok(f) = lib.get::<unsafe extern "C" fn() -> *mut std::ffi::c_void>(b"SteamAPI_SteamUGC") {
            ugc_ptr = f();
        }
    }
    if ugc_ptr.is_null() {
        return Err(tr!(
            "Интерфейс Steam Workshop (UGC) недоступен в этой версии steam_api64.dll",
            "The Steam Workshop (UGC) interface is not available in this steam_api64.dll",
            "此 steam_api64.dll 中没有 Steam 创意工坊 (UGC) 接口"
        )
        .to_string());
    }

    let result = action(ugc_ptr, &lib);

    // Requests are asynchronous: give Steam a moment to receive them before shutting down.
    if session == Session::Requests {
        if let Ok(run_callbacks) = lib.get::<unsafe extern "C" fn()>(b"SteamAPI_RunCallbacks") {
            let started = std::time::Instant::now();
            while started.elapsed() < CALLBACK_PUMP {
                run_callbacks();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    result
}

/// Sends one Subscribe/Unsubscribe request per item in a single Steam API session.
/// Returns how many requests were sent.
fn ugc_batch(dll_path: &Path, symbol: &[u8], item_ids: &[&str]) -> Result<usize, String> {
    let ids: Vec<u64> = item_ids.iter().filter_map(|s| s.trim().parse().ok()).collect();
    if ids.is_empty() {
        return Ok(0);
    }
    unsafe {
        with_steam_ugc(dll_path, Session::Requests, |ugc_ptr, lib| {
            let call: libloading::Symbol<UgcFn> = lib.get(symbol).map_err(|e| e.to_string())?;
            for id in &ids {
                let _call_handle = call(ugc_ptr, *id);
            }
            Ok(ids.len())
        })
    }
}

/// Read-only check that the Steam API works: starts it and asks how many Workshop items
/// the user is subscribed to for RimWorld. Changes nothing.
pub fn steam_api_check(dll_path: &Path) -> Result<u32, String> {
    unsafe {
        with_steam_ugc(dll_path, Session::Requests, |ugc_ptr, lib| {
            let count: libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void) -> u32> = lib
                .get(b"SteamAPI_ISteamUGC_GetNumSubscribedItems\0")
                .map_err(|e| e.to_string())?;
            Ok(count(ugc_ptr))
        })
    }
}

pub fn unsubscribe_workshop_items_batch_native(dll_path: &Path, item_ids: &[&str]) -> Result<usize, String> {
    ugc_batch(dll_path, b"SteamAPI_ISteamUGC_UnsubscribeItem\0", item_ids)
}

pub fn subscribe_workshop_items_batch_native(dll_path: &Path, item_ids: &[&str]) -> Result<usize, String> {
    ugc_batch(dll_path, b"SteamAPI_ISteamUGC_SubscribeItem\0", item_ids)
}

/// Steam `EItemState` flags returned by `SteamAPI_ISteamUGC_GetItemState`.
const ITEM_STATE_INSTALLED: u32 = 4;
const ITEM_STATE_NEEDS_UPDATE: u32 = 8;
const ITEM_STATE_DOWNLOADING: u32 = 16;
const ITEM_STATE_DOWNLOAD_PENDING: u32 = 32;

/// Live download progress reported by the Steam client (`GetItemState` + `GetItemDownloadInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamDownloadProgress {
    pub installed: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub active: bool,
}

impl SteamDownloadProgress {
    /// 0..=100 percentage if byte counts are known, else by item count.
    pub fn percent(&self) -> u32 {
        if let Some(p) = self.bytes_done.saturating_mul(100).checked_div(self.bytes_total) {
            p.min(100) as u32
        } else if let Some(p) = (self.installed * 100).checked_div(self.total) {
            p.min(100) as u32
        } else {
            0
        }
    }
}

/// Subscribes to `item_ids` and polls `GetItemState` + `GetItemDownloadInfo` while the Steam
/// client downloads them, calling `on_progress` with updates. Returns `(subscribed_count, installed_count)`.
/// `stop_flag` allows stopping the wait early to release the Steam API session (S1).
/// `force_download` (reinstall): also asks Steam to download the items now, and does not
/// trust an "installed" state in the first seconds, which Steam may still report for files
/// that were just removed.
pub fn subscribe_workshop_items_with_progress(
    dll_path: &Path,
    item_ids: &[&str],
    stop_flag: Option<Arc<AtomicBool>>,
    force_download: bool,
    mut on_progress: impl FnMut(SteamDownloadProgress),
) -> Result<(usize, usize), String> {
    let ids: Vec<u64> = item_ids.iter().filter_map(|s| s.trim().parse().ok()).collect();
    if ids.is_empty() {
        return Ok((0, 0));
    }
    unsafe {
        with_steam_ugc(dll_path, Session::Requests, |ugc_ptr, lib| {
            let call: libloading::Symbol<UgcFn> = lib
                .get(b"SteamAPI_ISteamUGC_SubscribeItem\0")
                .map_err(|e| e.to_string())?;
            for id in &ids {
                let _call_handle = call(ugc_ptr, *id);
            }
            if force_download {
                type DownloadFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64, bool) -> bool;
                if let Ok(download) = lib.get::<DownloadFn>(b"SteamAPI_ISteamUGC_DownloadItem\0") {
                    for id in &ids {
                        download(ugc_ptr, *id, true);
                    }
                }
            }
            let trust_installed_after = if force_download {
                std::time::Duration::from_secs(3)
            } else {
                std::time::Duration::ZERO
            };

            type GetStateFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> u32;
            type GetDlInfoFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64, *mut u64, *mut u64) -> bool;
            let Ok(get_state) = lib.get::<GetStateFn>(b"SteamAPI_ISteamUGC_GetItemState\0") else {
                return Ok((ids.len(), 0));
            };
            let get_dl_info = lib.get::<GetDlInfoFn>(b"SteamAPI_ISteamUGC_GetItemDownloadInfo\0").ok();
            let run_callbacks = lib.get::<unsafe extern "C" fn()>(b"SteamAPI_RunCallbacks").ok();

            let started = std::time::Instant::now();
            let mut last_installed = 0usize;
            while started.elapsed() < std::time::Duration::from_secs(300) {
                if stop_flag.as_ref().is_some_and(|f| f.load(Ordering::Relaxed)) {
                    break;
                }
                if let Some(ref cb) = run_callbacks {
                    cb();
                }
                let mut installed = 0usize;
                let mut active = false;
                let mut bytes_done = 0u64;
                let mut bytes_total = 0u64;
                for &id in &ids {
                    let state = get_state(ugc_ptr, id);
                    let busy_bits = ITEM_STATE_DOWNLOADING | ITEM_STATE_DOWNLOAD_PENDING | ITEM_STATE_NEEDS_UPDATE;
                    if (state & ITEM_STATE_INSTALLED) != 0 && (state & busy_bits) == 0 {
                        installed += 1;
                    } else if (state & busy_bits) != 0 {
                        active = true;
                        if let Some(ref dl_fn) = get_dl_info {
                            let (mut done, mut tot) = (0u64, 0u64);
                            if dl_fn(ugc_ptr, id, &mut done, &mut tot) && tot > 0 {
                                bytes_done = bytes_done.saturating_add(done);
                                bytes_total = bytes_total.saturating_add(tot);
                            }
                        }
                    }
                }
                last_installed = installed;
                on_progress(SteamDownloadProgress {
                    installed,
                    total: ids.len(),
                    bytes_done,
                    bytes_total,
                    active: installed < ids.len() && active,
                });
                if installed >= ids.len() && started.elapsed() >= trust_installed_after {
                    break;
                }
                // Give the Steam client up to 5 seconds to start downloading; if nothing is queued, stop waiting.
                if !active && started.elapsed() >= std::time::Duration::from_secs(5) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            Ok((ids.len(), last_installed))
        })
    }
}

// ----- Workshop queries through the Steam client -----
//
// When the Steam website refuses the app, the Workshop tab asks the Steam client instead:
// the query goes out from the account the user is signed in to, the way a game's own mod
// browser asks. (In Australia since September 2026 Steam shows the community pages of R18+
// games, RimWorld among them, only to age-verified accounts, and the app asks the site
// without signing in.)

/// `EUGCQuery` values the Workshop tab sorts by.
pub const UGC_RANKED_BY_PUBLICATION_DATE: i32 = 1;
pub const UGC_RANKED_BY_TREND: i32 = 3;
pub const UGC_RANKED_BY_TEXT_SEARCH: i32 = 11;
pub const UGC_RANKED_BY_TOTAL_UNIQUE_SUBSCRIPTIONS: i32 = 12;
pub const UGC_RANKED_BY_LAST_UPDATED_DATE: i32 = 19;
/// `EUGCMatchingUGCType`: mods ready to use, collections.
pub const UGC_MATCHING_READY_TO_USE: i32 = 2;
pub const UGC_MATCHING_COLLECTIONS: i32 = 3;
/// Items in one page of a Steam client query (`kNumUGCResultsPerPage`).
pub const UGC_PER_PAGE: usize = 50;

const UGC_QUERY_HANDLE_INVALID: u64 = u64::MAX;
const API_CALL_INVALID: u64 = 0;
/// `SteamUGCQueryCompleted_t::k_iCallback` (`k_iSteamUGCCallbacks + 1`).
const UGC_QUERY_COMPLETED_CALLBACK: i32 = 3401;
const E_RESULT_OK: i32 = 1;
/// `EItemStatistic`.
const STAT_SUBSCRIPTIONS: i32 = 0;
const STAT_FAVORITES: i32 = 1;
/// `EWorkshopFileType` of a collection.
const FILE_TYPE_COLLECTION: i32 = 2;
const UGC_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// `SteamUGCDetails_t` of the SDK the game ships (1.5x). Steam packs its structs at 8 on
/// Windows, which is what `repr(C)` gives here: no field needs more.
#[repr(C)]
struct UgcDetailsRaw {
    published_file_id: u64,
    result: i32,
    file_type: i32,
    _creator_app_id: u32,
    _consumer_app_id: u32,
    title: [u8; 129],
    description: [u8; 8000],
    _steam_id_owner: u64,
    _time_created: u32,
    time_updated: u32,
    _time_added_to_user_list: u32,
    _visibility: i32,
    _banned: u8,
    _accepted_for_use: u8,
    _tags_truncated: u8,
    tags: [u8; 1025],
    _file: u64,
    _preview_file: u64,
    _file_name: [u8; 260],
    file_size: i32,
    _preview_file_size: i32,
    _url: [u8; 256],
    _votes_up: u32,
    _votes_down: u32,
    _score: f32,
    num_children: u32,
}

/// Newer SDKs add fields at the end: room for them, so the DLL never writes past the buffer.
#[repr(C)]
struct UgcDetailsBuf {
    details: UgcDetailsRaw,
    _spare: [u8; 4096],
}

/// `SteamUGCQueryCompleted_t`.
#[repr(C)]
struct UgcQueryCompleted {
    _handle: u64,
    result: i32,
    num_results_returned: u32,
    _total_matching_results: u32,
    _cached_data: u8,
    _next_cursor: [u8; 256],
}

const _: () = assert!(std::mem::size_of::<UgcDetailsRaw>() == 9776);
const _: () = assert!(std::mem::size_of::<UgcQueryCompleted>() == 280);

/// What to look for: a page of the catalog or of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UgcPageQuery<'a> {
    /// `UGC_RANKED_BY_*`.
    pub query_type: i32,
    /// `UGC_MATCHING_*`.
    pub matching: i32,
    /// The period of `UGC_RANKED_BY_TREND` (0 = Steam's default).
    pub trend_days: u32,
    pub text: &'a str,
    pub required_tag: Option<&'a str>,
}

/// A Workshop item as the Steam client describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UgcItem {
    pub id: u64,
    pub title: String,
    /// BBCode, as on the Workshop page.
    pub description: String,
    pub tags: Vec<String>,
    pub preview_url: Option<String>,
    pub subscriptions: u64,
    pub favorited: u64,
    pub file_size: u64,
    pub time_updated: u64,
    pub is_collection: bool,
    /// A collection's items in its order.
    pub children: Vec<u64>,
}

type Ptr = *mut std::ffi::c_void;
type CText = *const std::ffi::c_char;
type SetBoolFn = unsafe extern "C" fn(Ptr, u64, bool) -> bool;
type CallResultFn = unsafe extern "C" fn(Ptr, u64, Ptr, i32, i32, *mut bool) -> bool;

/// The ISteamUGC and ISteamUtils calls a query needs (valid while the DLL is loaded).
struct UgcQueryApi {
    ugc: Ptr,
    utils: Ptr,
    create_all: unsafe extern "C" fn(Ptr, i32, i32, u32, u32, u32) -> u64,
    create_details: unsafe extern "C" fn(Ptr, *const u64, u32) -> u64,
    set_search_text: unsafe extern "C" fn(Ptr, u64, CText) -> bool,
    set_trend_days: unsafe extern "C" fn(Ptr, u64, u32) -> bool,
    add_required_tag: unsafe extern "C" fn(Ptr, u64, CText) -> bool,
    set_long_description: SetBoolFn,
    set_children: SetBoolFn,
    send: unsafe extern "C" fn(Ptr, u64) -> u64,
    release: unsafe extern "C" fn(Ptr, u64) -> bool,
    result: unsafe extern "C" fn(Ptr, u64, u32, *mut UgcDetailsRaw) -> bool,
    preview_url: unsafe extern "C" fn(Ptr, u64, u32, *mut std::ffi::c_char, u32) -> bool,
    statistic: unsafe extern "C" fn(Ptr, u64, u32, i32, *mut u64) -> bool,
    children: unsafe extern "C" fn(Ptr, u64, u32, *mut u64, u32) -> bool,
    is_completed: unsafe extern "C" fn(Ptr, u64, *mut bool) -> bool,
    call_result: CallResultFn,
}

fn no_answer() -> String {
    crate::i18n::text(crate::i18n::TranslationKey::WsSteamNoAnswer).to_string()
}

/// Text of a C string field (up to its first zero byte).
fn c_text(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).trim().to_string()
}

/// Steam's comma-separated tag list.
fn split_tags(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

unsafe fn steam_utils(lib: &libloading::Library) -> Option<Ptr> {
    for version in (5..=15).rev() {
        let name = format!("SteamAPI_SteamUtils_v{version:03}\0");
        if let Ok(f) = lib.get::<unsafe extern "C" fn() -> Ptr>(name.as_bytes()) {
            let utils = f();
            if !utils.is_null() {
                return Some(utils);
            }
        }
    }
    None
}

unsafe fn symbol<T: Copy>(lib: &libloading::Library, name: &[u8]) -> Result<T, String> {
    lib.get::<T>(name).map(|s| *s).map_err(|e| e.to_string())
}

impl UgcQueryApi {
    unsafe fn load(ugc: Ptr, lib: &libloading::Library) -> Result<Self, String> {
        Ok(Self {
            ugc,
            utils: steam_utils(lib).ok_or_else(no_answer)?,
            create_all: symbol(lib, b"SteamAPI_ISteamUGC_CreateQueryAllUGCRequestPage\0")?,
            create_details: symbol(lib, b"SteamAPI_ISteamUGC_CreateQueryUGCDetailsRequest\0")?,
            set_search_text: symbol(lib, b"SteamAPI_ISteamUGC_SetSearchText\0")?,
            set_trend_days: symbol(lib, b"SteamAPI_ISteamUGC_SetRankedByTrendDays\0")?,
            add_required_tag: symbol(lib, b"SteamAPI_ISteamUGC_AddRequiredTag\0")?,
            set_long_description: symbol(lib, b"SteamAPI_ISteamUGC_SetReturnLongDescription\0")?,
            set_children: symbol(lib, b"SteamAPI_ISteamUGC_SetReturnChildren\0")?,
            send: symbol(lib, b"SteamAPI_ISteamUGC_SendQueryUGCRequest\0")?,
            release: symbol(lib, b"SteamAPI_ISteamUGC_ReleaseQueryUGCRequest\0")?,
            result: symbol(lib, b"SteamAPI_ISteamUGC_GetQueryUGCResult\0")?,
            preview_url: symbol(lib, b"SteamAPI_ISteamUGC_GetQueryUGCPreviewURL\0")?,
            statistic: symbol(lib, b"SteamAPI_ISteamUGC_GetQueryUGCStatistic\0")?,
            children: symbol(lib, b"SteamAPI_ISteamUGC_GetQueryUGCChildren\0")?,
            is_completed: symbol(lib, b"SteamAPI_ISteamUtils_IsAPICallCompleted\0")?,
            call_result: symbol(lib, b"SteamAPI_ISteamUtils_GetAPICallResult\0")?,
        })
    }

    /// Sends a prepared query, reads its items and releases it. Also returns how many
    /// results Steam gave (hidden or removed items among them).
    unsafe fn run(&self, handle: u64) -> Result<(Vec<UgcItem>, u32), String> {
        (self.set_long_description)(self.ugc, handle, true);
        (self.set_children)(self.ugc, handle, true);
        let result = self.send_and_read(handle);
        (self.release)(self.ugc, handle);
        result
    }

    unsafe fn send_and_read(&self, handle: u64) -> Result<(Vec<UgcItem>, u32), String> {
        let call = (self.send)(self.ugc, handle);
        if call == API_CALL_INVALID {
            return Err(no_answer());
        }
        let started = std::time::Instant::now();
        loop {
            let mut failed = false;
            if (self.is_completed)(self.utils, call, &mut failed) {
                if failed {
                    return Err(no_answer());
                }
                break;
            }
            if started.elapsed() > UGC_QUERY_TIMEOUT {
                return Err(no_answer());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let mut done: UgcQueryCompleted = std::mem::zeroed();
        let mut failed = false;
        let known = (self.call_result)(
            self.utils,
            call,
            (&mut done as *mut UgcQueryCompleted).cast(),
            std::mem::size_of::<UgcQueryCompleted>() as i32,
            UGC_QUERY_COMPLETED_CALLBACK,
            &mut failed,
        ) && !failed;
        if known && done.result != E_RESULT_OK {
            return Err(crate::i18n::fmt(
                crate::i18n::text(crate::i18n::TranslationKey::WsSteamQueryFailed),
                &[&done.result.to_string()],
            ));
        }
        // Without the answer's header (a DLL with another layout of it) the results are read
        // until they run out.
        let count = if known {
            done.num_results_returned.min(1000)
        } else {
            UGC_PER_PAGE as u32
        };

        let buf: *mut UgcDetailsBuf = Box::into_raw(Box::<UgcDetailsBuf>::new_zeroed().assume_init());
        let mut items = Vec::new();
        let mut returned = 0;
        for index in 0..count {
            if !(self.result)(self.ugc, handle, index, buf.cast()) {
                if known {
                    continue;
                }
                break;
            }
            returned += 1;
            let d = &(*buf).details;
            if d.result == E_RESULT_OK && d.published_file_id != 0 {
                items.push(self.item(handle, index, d));
            }
        }
        drop(Box::from_raw(buf));
        Ok((items, if known { done.num_results_returned } else { returned }))
    }

    unsafe fn item(&self, handle: u64, index: u32, d: &UgcDetailsRaw) -> UgcItem {
        let stat = |kind: i32| {
            let mut value = 0u64;
            if (self.statistic)(self.ugc, handle, index, kind, &mut value) {
                value
            } else {
                0
            }
        };
        let mut url = [0u8; 1024];
        let preview_url = (self.preview_url)(self.ugc, handle, index, url.as_mut_ptr().cast(), url.len() as u32)
            .then(|| c_text(&url))
            .filter(|u| !u.is_empty());
        let mut children = vec![0u64; d.num_children.min(10_000) as usize];
        if !children.is_empty()
            && !(self.children)(self.ugc, handle, index, children.as_mut_ptr(), children.len() as u32)
        {
            children.clear();
        }
        children.retain(|&c| c != 0);
        UgcItem {
            id: d.published_file_id,
            title: c_text(&d.title),
            description: c_text(&d.description),
            tags: split_tags(&c_text(&d.tags)),
            preview_url,
            subscriptions: stat(STAT_SUBSCRIPTIONS),
            favorited: stat(STAT_FAVORITES),
            file_size: d.file_size.max(0) as u64,
            time_updated: u64::from(d.time_updated),
            is_collection: d.file_type == FILE_TYPE_COLLECTION,
            children,
        }
    }
}

/// Pages `pages` (from 1, `UGC_PER_PAGE` items each) of a Workshop query through the Steam
/// client, in one Steam API session; stops after a page that isn't full.
pub fn ugc_query_pages(
    dll_path: &Path,
    q: &UgcPageQuery,
    pages: std::ops::RangeInclusive<u32>,
) -> Result<Vec<UgcItem>, String> {
    let text = std::ffi::CString::new(q.text.trim()).unwrap_or_default();
    let tag = q.required_tag.and_then(|t| std::ffi::CString::new(t).ok());
    unsafe {
        with_steam_ugc(dll_path, Session::Query, |ugc, lib| {
            let api = UgcQueryApi::load(ugc, lib)?;
            let mut out = Vec::new();
            for page in pages {
                let handle = (api.create_all)(ugc, q.query_type, q.matching, RIMWORLD_APP_ID, RIMWORLD_APP_ID, page);
                if handle == UGC_QUERY_HANDLE_INVALID {
                    return Err(no_answer());
                }
                if !text.as_bytes().is_empty() {
                    (api.set_search_text)(ugc, handle, text.as_ptr());
                }
                if q.trend_days > 0 {
                    (api.set_trend_days)(ugc, handle, q.trend_days);
                }
                if let Some(tag) = &tag {
                    (api.add_required_tag)(ugc, handle, tag.as_ptr());
                }
                let (items, returned) = api.run(handle)?;
                out.extend(items);
                if (returned as usize) < UGC_PER_PAGE {
                    break;
                }
            }
            Ok(out)
        })
    }
}

/// Given items through the Steam client (hidden or removed ones left out), 50 per query.
pub fn ugc_item_details(dll_path: &Path, ids: &[u64]) -> Result<Vec<UgcItem>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    unsafe {
        with_steam_ugc(dll_path, Session::Query, |ugc, lib| {
            let api = UgcQueryApi::load(ugc, lib)?;
            let mut out = Vec::new();
            for chunk in ids.chunks(UGC_PER_PAGE) {
                let handle = (api.create_details)(ugc, chunk.as_ptr(), chunk.len() as u32);
                if handle == UGC_QUERY_HANDLE_INVALID {
                    return Err(no_answer());
                }
                out.extend(api.run(handle)?.0);
            }
            Ok(out)
        })
    }
}

fn parse_item_id(item_id_str: &str) -> Result<u64, String> {
    item_id_str.parse::<u64>().map_err(|_| {
        tr!(
            "Неверный Steam Item ID: {}",
            "Invalid Steam item ID: {}",
            "无效的 Steam 物品 ID：{}",
            item_id_str
        )
    })
}

pub fn unsubscribe_workshop_item_native(dll_path: &Path, item_id_str: &str) -> Result<bool, String> {
    parse_item_id(item_id_str)?;
    Ok(unsubscribe_workshop_items_batch_native(dll_path, &[item_id_str])? > 0)
}

pub fn subscribe_workshop_item_native(dll_path: &Path, item_id_str: &str) -> Result<bool, String> {
    parse_item_id(item_id_str)?;
    Ok(subscribe_workshop_items_batch_native(dll_path, &[item_id_str])? > 0)
}

/// Steam Workshop page of an item.
pub fn workshop_url(item_id: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={item_id}")
}

/// Starts the game executable that matches the installed engine
/// (`RimWorldWin64.exe` for 1.6/Unity, `RimWorld.exe` for 1.7/Waylith).
pub fn launch_rimworld(steam_paths: &SteamPaths, args: &[String]) -> Result<(), String> {
    let game_dir = steam_paths.game_path.as_ref().ok_or(tr!(
        "Папка игры RimWorld не найдена",
        "RimWorld game folder not found",
        "找不到 RimWorld 游戏文件夹"
    ))?;

    let exe_path = steam_paths
        .exe_path
        .clone()
        .or_else(|| game_install::game_exe_path(game_dir, steam_paths.engine))
        .ok_or_else(|| {
            tr!(
                "Не найден исполняемый файл игры ({} или {}) в {}",
                "Game executable ({} or {}) not found in {}",
                "在 {2} 中找不到游戏程序（{0} 或 {1}）",
                game_install::WAYLITH_EXE,
                game_install::UNITY_EXE,
                game_dir.display()
            )
        })?;

    let mut cmd = std::process::Command::new(&exe_path);
    cmd.current_dir(game_dir);
    cmd.args(args.iter().filter(|a| !a.trim().is_empty()));

    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(tr!(
            "Не удалось запустить процесс: {}",
            "Could not start the process: {}",
            "无法启动进程：{}",
            e
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_progress_percentage() {
        let by_bytes = SteamDownloadProgress {
            installed: 0,
            total: 2,
            bytes_done: 25 * 1024 * 1024,
            bytes_total: 100 * 1024 * 1024,
            active: true,
        };
        assert_eq!(by_bytes.percent(), 25);

        let by_items = SteamDownloadProgress {
            installed: 1,
            total: 4,
            bytes_done: 0,
            bytes_total: 0,
            active: true,
        };
        assert_eq!(by_items.percent(), 25);
    }

    #[test]
    fn subscribe_empty_items_returns_zero_immediately() {
        let flag = Arc::new(AtomicBool::new(false));
        let res = subscribe_workshop_items_with_progress(Path::new("dummy.dll"), &[], Some(flag), false, |_| {});
        assert_eq!(res, Ok((0, 0)));
    }

    #[test]
    fn text_fields_of_steam_structures() {
        let mut title = [0u8; 16];
        title[..9].copy_from_slice(b" HugsLib ");
        assert_eq!(c_text(&title), "HugsLib");
        assert_eq!(c_text(b"no zero at the end"), "no zero at the end");
        assert_eq!(split_tags("Mod, 1.5,1.6,,"), ["Mod", "1.5", "1.6"]);
        assert!(split_tags("").is_empty());
    }

    #[test]
    fn a_query_gives_up_on_a_busy_steam_api() {
        let wait = std::time::Duration::from_millis(300);
        let download = STEAM_API_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let started = std::time::Instant::now();
        std::thread::scope(|s| {
            let query = s.spawn(|| session_lock_within(Session::Query, wait).map(|_| ()));
            assert!(query.join().unwrap().is_err());
        });
        assert!(started.elapsed() >= wait);
        drop(download);
        assert!(session_lock_within(Session::Query, wait).is_ok());
    }

    /// Live, read-only: `cargo test real_steam_api_check -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_steam_api_check() {
        let paths = detect_steam_paths();
        let dll = paths.steam_dll_path.expect("steam_api64.dll not found");
        println!("dll: {}", dll.display());
        println!("subscribed items: {:?}", steam_api_check(&dll));
    }
}
