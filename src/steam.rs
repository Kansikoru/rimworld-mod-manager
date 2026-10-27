use std::path::{Path, PathBuf};
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
unsafe fn with_steam_ugc<F, R>(dll_path: &Path, action: F) -> Result<R, String>
where
    F: FnOnce(*mut std::ffi::c_void, &libloading::Library) -> Result<R, String>,
{
    let _session = STEAM_API_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
                "Could not start the Steam API (is the Steam client running?)"
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
            "The Steam Workshop (UGC) interface is not available in this steam_api64.dll"
        )
        .to_string());
    }

    let result = action(ugc_ptr, &lib);

    // Requests are asynchronous: give Steam a moment to receive them before shutting down.
    if let Ok(run_callbacks) = lib.get::<unsafe extern "C" fn()>(b"SteamAPI_RunCallbacks") {
        let started = std::time::Instant::now();
        while started.elapsed() < CALLBACK_PUMP {
            run_callbacks();
            std::thread::sleep(std::time::Duration::from_millis(100));
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
        with_steam_ugc(dll_path, |ugc_ptr, lib| {
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
        with_steam_ugc(dll_path, |ugc_ptr, lib| {
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

fn parse_item_id(item_id_str: &str) -> Result<u64, String> {
    item_id_str
        .parse::<u64>()
        .map_err(|_| tr!("Неверный Steam Item ID: {}", "Invalid Steam item ID: {}", item_id_str))
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
    let game_dir = steam_paths
        .game_path
        .as_ref()
        .ok_or(tr!("Папка игры RimWorld не найдена", "RimWorld game folder not found"))?;

    let exe_path = steam_paths
        .exe_path
        .clone()
        .or_else(|| game_install::game_exe_path(game_dir, steam_paths.engine))
        .ok_or_else(|| {
            tr!(
                "Не найден исполняемый файл игры ({} или {}) в {}",
                "Game executable ({} or {}) not found in {}",
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
            e
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
