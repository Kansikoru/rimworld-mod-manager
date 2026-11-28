//! What differs between operating systems: where Steam, the game and this manager keep
//! their files, the names of the game's executable and Steam library, how to tell whether
//! the game runs. The rest of the app asks here instead of reading Windows paths itself.
//!
//! Each OS has a file with the same set of items:
//! - `UNITY_EXE`, `WAYLITH_EXE`: the game's executable for 1.6 (Unity) and 1.7 (Waylith);
//! - `STEAM_API_LIB`: the file name of the Steam API library;
//! - `steam_api_in_game(game_dir)`: where the game keeps that library, most likely first;
//! - `steam_install_dir()`: the Steam client's folder;
//! - `game_data_dirs()`: the game's user data folder (Config, Saves, ModLists, Player.log),
//!   most likely first;
//! - `manager_data_base()`: the folder that holds this manager's `RimWorldModManager` folder;
//! - `desktop_dir()`: the user's desktop;
//! - `game_pid()`: the running game's process ID;
//! - `system_font(file)`, `has_system_font(file)`: a font file of the system, by its file name;
//! - `STEAMCMD_INSTALLER_URL`, `STEAMCMD_EXE`: Valve's SteamCMD for this OS and the file that
//!   starts it;
//! - `STEAMCMD_STEAM_ROOT`: the folder inside SteamCMD's own one that holds its `steamapps`
//!   (empty when it is that folder itself);
//! - `steamcmd_missing_requirement()`: what the system lacks to run SteamCMD, if anything;
//! - `CASE_INSENSITIVE_PATHS`: whether `C:\Mods` and `c:\mods` are the same folder.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("This port builds for Windows and Linux; macOS is done in its own port (3.3-macos).");

use std::path::PathBuf;

/// The folder name the game uses under its company folder, on every OS.
pub const GAME_DATA_NAME: &str = "RimWorld by Ludeon Studios";
/// The game's company folder.
pub const GAME_COMPANY: &str = "Ludeon Studios";

/// A path as text for comparing and as a map key: without a trailing slash, and in lower case
/// where the file system ignores case (Windows). On Linux `Mods` and `mods` are two folders.
pub fn path_key(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    let text = text.trim_end_matches(['\\', '/']);
    if CASE_INSENSITIVE_PATHS {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The game's user data folder that exists, else the most likely one (the game creates it
/// on its first start).
pub fn game_data_dir() -> Option<PathBuf> {
    let candidates = game_data_dirs();
    candidates
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .or_else(|| candidates.into_iter().next())
}
