//! Windows: Steam from the registry, the game's data in `AppData\LocalLow`, this manager's
//! in `%LOCALAPPDATA%`, the running game from `tasklist`.

use std::path::{Path, PathBuf};

use super::{home_dir, GAME_COMPANY, GAME_DATA_NAME};

pub const UNITY_EXE: &str = "RimWorldWin64.exe";
pub const WAYLITH_EXE: &str = "RimWorld.exe";
pub const STEAM_API_LIB: &str = "steam_api64.dll";
pub const CASE_INSENSITIVE_PATHS: bool = true;

pub const STEAMCMD_INSTALLER_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";
pub const STEAMCMD_EXE: &str = "steamcmd.exe";
/// SteamCMD keeps `steamapps` right in its folder.
pub const STEAMCMD_STEAM_ROOT: &str = "";

/// Nothing extra is needed on Windows.
pub fn steamcmd_missing_requirement() -> Option<String> {
    None
}

/// Game root (1.7) or Unity's plugin folder (1.6).
pub fn steam_api_in_game(game_dir: &Path) -> Vec<PathBuf> {
    vec![
        game_dir.join(STEAM_API_LIB),
        game_dir
            .join("RimWorldWin64_Data")
            .join("Plugins")
            .join("x86_64")
            .join(STEAM_API_LIB),
    ]
}

/// `SteamPath` of the current user, else the machine-wide `InstallPath`.
pub fn steam_install_dir() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

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

/// `%USERPROFILE%\AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios`, and the
/// same next to `%LOCALAPPDATA%` / `%APPDATA%` when the profile folder is elsewhere.
pub fn game_data_dirs() -> Vec<PathBuf> {
    let in_local_low = |local_low: PathBuf| local_low.join(GAME_COMPANY).join(GAME_DATA_NAME);
    let mut dirs = Vec::new();
    if let Some(home) = home_dir() {
        dirs.push(in_local_low(home.join("AppData").join("LocalLow")));
    }
    for var in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(parent) = std::env::var_os(var).and_then(|p| PathBuf::from(p).parent().map(Path::to_path_buf)) {
            let dir = in_local_low(parent.join("LocalLow"));
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// `%LOCALAPPDATA%`.
pub fn manager_data_base() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn desktop_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Desktop"))
}

/// The running game (either engine), from `tasklist`.
pub fn game_pid() -> Option<u32> {
    use std::os::windows::process::CommandExt;
    // No console window flashing: this app has no console of its own.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("tasklist")
        .args(["/NH", "/FO", "CSV"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    find_game_pid(&String::from_utf8_lossy(&out.stdout))
}

/// The game's PID in `tasklist /NH /FO CSV` output. Rows start with the quoted image name
/// and PID: "RimWorldWin64.exe","1234",...
fn find_game_pid(list: &str) -> Option<u32> {
    let exes = [UNITY_EXE.to_lowercase(), WAYLITH_EXE.to_lowercase()];
    list.lines().find_map(|row| {
        let mut cols = row.split(',').map(|c| c.trim().trim_matches('"'));
        let name = cols.next()?.to_lowercase();
        if !exes.contains(&name) {
            return None;
        }
        cols.next()?.parse().ok()
    })
}

/// `C:\Windows\Fonts\<file>`.
fn system_font_path(file: &str) -> Option<PathBuf> {
    let windir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    Some(windir.join("Fonts").join(file)).filter(|p| p.is_file())
}

/// A font file of the system, read.
pub fn system_font(file: &str) -> Option<Vec<u8>> {
    std::fs::read(system_font_path(file)?).ok()
}

/// Whether the system has this font file.
pub fn has_system_font(file: &str) -> bool {
    system_font_path(file).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_pid_is_read_from_tasklist_csv() {
        let list = "\"System\",\"4\",\"Services\",\"0\",\"136 K\"\r\n\
                    \"RimWorldWin64.exe\",\"12345\",\"Console\",\"1\",\"3,104,220 K\"\r\n";
        assert_eq!(find_game_pid(list), Some(12345));
        assert_eq!(
            find_game_pid("\"RimWorld.exe\",\"77\",\"Console\",\"1\",\"1 K\""),
            Some(77)
        );
        assert_eq!(
            find_game_pid("\"MyRimWorldWin64.exe\",\"9\",\"Console\",\"1\",\"1 K\""),
            None
        );
    }
}
