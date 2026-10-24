use std::path::{Path, PathBuf};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

use crate::game_install::{self, GameEngine};

pub const RIMWORLD_APP_ID: u32 = 294100;

#[derive(Debug, Clone, Default)]
pub struct SteamPaths {
    pub steam_path: Option<PathBuf>,
    pub workshop_path: Option<PathBuf>,
    pub local_mods_path: Option<PathBuf>,
    pub game_data_path: Option<PathBuf>,
    pub game_path: Option<PathBuf>,
    pub steam_dll_path: Option<PathBuf>,
    pub acf_path: Option<PathBuf>,
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
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
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
            if acf.is_file() && paths.acf_path.is_none() {
                paths.acf_path = Some(acf);
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

pub fn remove_workshop_item_from_acf(acf_path: &Path, item_id: &str) -> Result<(), String> {
    if !acf_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(acf_path).map_err(|e| e.to_string())?;
    let pattern = format!(r#""{}"\s*\{{[^}}]*\}}"#, regex::escape(item_id));
    let re = regex::Regex::new(&pattern).map_err(|e| e.to_string())?;

    let new_content = re.replace_all(&content, "").to_string();
    if new_content != content {
        std::fs::write(acf_path, new_content).map_err(|e| e.to_string())?;
    }

    Ok(())
}

static ACF_SUB_ITEM_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r#""(\d{7,12})"\s*\{"#).unwrap());

pub fn parse_subscribed_items_from_acf(acf_path: &Path) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    if let Ok(content) = std::fs::read_to_string(acf_path) {
        for cap in ACF_SUB_ITEM_RE.captures_iter(&content) {
            if let Some(m) = cap.get(1) {
                ids.insert(m.as_str().to_string());
            }
        }
    }
    ids
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

unsafe fn with_steam_ugc<F, R>(dll_path: &Path, action: F) -> Result<R, String>
where
    F: FnOnce(*mut std::ffi::c_void, &libloading::Library) -> Result<R, String>,
{
    let lib = libloading::Library::new(dll_path).map_err(|e| e.to_string())?;

    unsafe {
        std::env::set_var("SteamAppId", RIMWORLD_APP_ID.to_string());
    }

    let init_flat: Result<libloading::Symbol<unsafe extern "C" fn(*mut u8) -> bool>, _> =
        lib.get(b"SteamAPI_InitFlat");
    let init_ok = if let Ok(f) = init_flat {
        let mut err_buf = [0u8; 1024];
        f(err_buf.as_mut_ptr())
    } else {
        let init_std: libloading::Symbol<unsafe extern "C" fn() -> bool> =
            lib.get(b"SteamAPI_Init").map_err(|e| e.to_string())?;
        init_std()
    };

    if !init_ok {
        return Err(
            "Не удалось инициализировать Steam API (возможно клиент Steam закрыт)".to_string(),
        );
    }

    let _guard = SteamShutdownGuard { lib: &lib };

    let get_ugc: Result<libloading::Symbol<unsafe extern "C" fn() -> *mut std::ffi::c_void>, _> =
        lib.get(b"SteamAPI_SteamUGC_v016");
    let ugc_ptr = if let Ok(f) = get_ugc {
        f()
    } else if let Ok(get_ugc_def) =
        lib.get::<unsafe extern "C" fn() -> *mut std::ffi::c_void>(b"SteamAPI_SteamUGC")
    {
        get_ugc_def()
    } else {
        std::ptr::null_mut()
    };

    if ugc_ptr.is_null() {
        Err("Интерфейс Steam UGC недоступен".to_string())
    } else {
        action(ugc_ptr, &lib)
    }
}

pub fn unsubscribe_workshop_item_native(
    dll_path: &Path,
    item_id_str: &str,
) -> Result<bool, String> {
    let item_id: u64 = item_id_str
        .parse()
        .map_err(|_| format!("Неверный Steam Item ID: {item_id_str}"))?;

    unsafe {
        with_steam_ugc(dll_path, |ugc_ptr, lib| {
            let unsub_fn: Result<
                libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> u64>,
                _,
            > = lib.get(b"SteamAPI_ISteamUGC_UnsubscribeItem");
            match unsub_fn {
                Ok(unsub) => {
                    let _call_handle = unsub(ugc_ptr, item_id);
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        })
    }
}

pub fn subscribe_workshop_item_native(dll_path: &Path, item_id_str: &str) -> Result<bool, String> {
    let item_id: u64 = item_id_str
        .parse()
        .map_err(|_| format!("Неверный Steam Item ID: {item_id_str}"))?;

    unsafe {
        with_steam_ugc(dll_path, |ugc_ptr, lib| {
            let sub_fn: Result<
                libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> u64>,
                _,
            > = lib.get(b"SteamAPI_ISteamUGC_SubscribeItem");
            match sub_fn {
                Ok(sub) => {
                    let _call_handle = sub(ugc_ptr, item_id);
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        })
    }
}

pub fn subscribe_workshop_items_batch_native(
    dll_path: &Path,
    item_ids: &[&str],
) -> Result<usize, String> {
    if item_ids.is_empty() {
        return Ok(0);
    }
    unsafe {
        with_steam_ugc(dll_path, |ugc_ptr, lib| {
            let sub_fn: Result<
                libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> u64>,
                _,
            > = lib.get(b"SteamAPI_ISteamUGC_SubscribeItem");
            match sub_fn {
                Ok(sub) => {
                    let mut count = 0;
                    for id_str in item_ids {
                        if let Ok(item_id) = id_str.parse::<u64>() {
                            let _call_handle = sub(ugc_ptr, item_id);
                            count += 1;
                        }
                    }
                    Ok(count)
                }
                Err(_) => Ok(0),
            }
        })
    }
}

/// Starts the game executable that matches the installed engine
/// (`RimWorldWin64.exe` for 1.6/Unity, `RimWorld.exe` for 1.7/Waylith).
pub fn launch_rimworld(steam_paths: &SteamPaths, args: &[String]) -> Result<(), String> {
    let game_dir = steam_paths
        .game_path
        .as_ref()
        .ok_or("Путь к игре (RimWorld) не найден")?;

    let exe_path = steam_paths
        .exe_path
        .clone()
        .or_else(|| game_install::game_exe_path(game_dir, steam_paths.engine))
        .ok_or_else(|| {
            format!(
                "Не найден исполняемый файл игры ({} или {}) в {}",
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
        Err(e) => Err(format!("Не удалось запустить процесс: {}", e)),
    }
}
