//! Linux (and SteamOS): Steam in one of its usual folders (native, Flatpak, Snap), the game's
//! data where Unity puts it (`~/.config/unity3d/...`), this manager's in `~/.local/share`
//! (XDG), the running game from `/proc`.
//!
//! To check on a real install: the 1.7 (Waylith) executable name and where 1.7 keeps its data.

use std::path::{Path, PathBuf};

use super::{home_dir, GAME_COMPANY, GAME_DATA_NAME};

pub const UNITY_EXE: &str = "RimWorldLinux";
/// Not confirmed yet: the name of the 1.7 executable on Linux.
pub const WAYLITH_EXE: &str = "RimWorld";
pub const STEAM_API_LIB: &str = "libsteam_api.so";

pub const STEAMCMD_INSTALLER_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz";
pub const STEAMCMD_EXE: &str = "steamcmd.sh";
/// SteamCMD keeps its library in `$HOME/Steam`; it is started with `HOME` set to its own
/// folder, so everything it makes stays in there.
pub const STEAMCMD_STEAM_ROOT: &str = "Steam";

/// SteamCMD is a 32-bit program: without the 32-bit C library it doesn't start at all, and
/// says so only on stderr. Steam itself needs the same, so systems with Steam (SteamOS too)
/// have it.
pub fn steamcmd_missing_requirement() -> Option<String> {
    let loaders = ["/lib/ld-linux.so.2", "/lib32/ld-linux.so.2", "/usr/lib32/ld-linux.so.2"];
    if loaders.iter().any(|p| Path::new(p).exists()) {
        return None;
    }
    Some(crate::i18n::tr!(
        "Для SteamCMD нужны 32-битные библиотеки. Установите их и попробуйте снова:\nArch, CachyOS, Manjaro: sudo pacman -S lib32-glibc lib32-gcc-libs\nUbuntu, Debian, Mint: sudo apt install lib32gcc-s1\nFedora: sudo dnf install glibc.i686 libstdc++.i686",
        "SteamCMD needs the 32-bit libraries. Install them and try again:\nArch, CachyOS, Manjaro: sudo pacman -S lib32-glibc lib32-gcc-libs\nUbuntu, Debian, Mint: sudo apt install lib32gcc-s1\nFedora: sudo dnf install glibc.i686 libstdc++.i686",
        "SteamCMD 需要 32 位库。请安装后重试：\nArch、CachyOS、Manjaro：sudo pacman -S lib32-glibc lib32-gcc-libs\nUbuntu、Debian、Mint：sudo apt install lib32gcc-s1\nFedora：sudo dnf install glibc.i686 libstdc++.i686"
    )
    .to_string())
}

/// `$XDG_*_HOME` when set (and absolute), else `~/<fallback>`.
fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home_dir().map(|h| h.join(fallback)))
}

/// Game root (1.7), Unity's plugin folders (1.6).
pub fn steam_api_in_game(game_dir: &Path) -> Vec<PathBuf> {
    let plugins = game_dir.join("RimWorldLinux_Data").join("Plugins");
    vec![
        game_dir.join(STEAM_API_LIB),
        plugins.join(STEAM_API_LIB),
        plugins.join("x86_64").join(STEAM_API_LIB),
    ]
}

/// The Steam client's folders, in the order they are tried. `~/.steam/steam` is a link
/// the client keeps to wherever it is installed; the Flatpak and Snap clients live in
/// their own sandboxes.
fn steam_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home_dir() {
        dirs.push(home.join(".steam").join("steam"));
        dirs.push(home.join(".steam").join("root"));
    }
    if let Some(data) = xdg_dir("XDG_DATA_HOME", ".local/share") {
        dirs.push(data.join("Steam"));
    }
    if let Some(home) = home_dir() {
        dirs.push(home.join(".local").join("share").join("Steam"));
        dirs.push(
            home.join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join(".local")
                .join("share")
                .join("Steam"),
        );
        dirs.push(
            home.join("snap")
                .join("steam")
                .join("common")
                .join(".local")
                .join("share")
                .join("Steam"),
        );
    }
    dirs.dedup();
    dirs
}

/// The first Steam folder that has a library (`steamapps`).
pub fn steam_install_dir() -> Option<PathBuf> {
    steam_dirs().into_iter().find(|d| d.join("steamapps").is_dir())
}

/// Unity's `$XDG_CONFIG_HOME/unity3d/<company>/<product>`, and the same inside the Flatpak
/// Steam sandbox (a game started from Flatpak Steam keeps its data there).
pub fn game_data_dirs() -> Vec<PathBuf> {
    let in_config = |config: PathBuf| config.join("unity3d").join(GAME_COMPANY).join(GAME_DATA_NAME);
    let mut dirs = Vec::new();
    if let Some(config) = xdg_dir("XDG_CONFIG_HOME", ".config") {
        dirs.push(in_config(config));
    }
    if let Some(home) = home_dir() {
        let plain = in_config(home.join(".config"));
        if !dirs.contains(&plain) {
            dirs.push(plain);
        }
        dirs.push(in_config(
            home.join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join(".config"),
        ));
    }
    dirs
}

/// `$XDG_DATA_HOME`, else `~/.local/share`.
pub fn manager_data_base() -> Option<PathBuf> {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_DESKTOP_DIR`, else `~/Desktop`.
pub fn desktop_dir() -> Option<PathBuf> {
    xdg_dir("XDG_DESKTOP_DIR", "Desktop")
}

/// The running game, from the process names in `/proc/<pid>/comm`.
pub fn game_pid() -> Option<u32> {
    let entries = std::fs::read_dir("/proc").ok()?;
    entries.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
        is_game_process(&comm).then_some(pid)
    })
}

/// `comm` holds the executable's name cut to 15 bytes, with a line break.
fn is_game_process(comm: &str) -> bool {
    let name = comm.trim_end();
    [UNITY_EXE, WAYLITH_EXE]
        .iter()
        .any(|exe| name == &exe[..exe.len().min(15)])
}

/// Where fonts are installed: the system's folders and the user's own.
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(data) = xdg_dir("XDG_DATA_HOME", ".local/share") {
        dirs.push(data.join("fonts"));
    }
    if let Some(home) = home_dir() {
        dirs.push(home.join(".fonts"));
    }
    dirs
}

/// A font file by its name anywhere in the font folders (they keep fonts in subfolders
/// per family, e.g. `/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc`).
pub fn system_font(file: &str) -> Option<Vec<u8>> {
    font_dirs().into_iter().find_map(|dir| {
        walkdir::WalkDir::new(dir)
            .max_depth(4)
            .into_iter()
            .flatten()
            .find(|e| e.file_type().is_file() && e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(file)))
            .and_then(|e| std::fs::read(e.path()).ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_process_names_from_proc() {
        assert!(is_game_process("RimWorldLinux\n"));
        assert!(is_game_process("RimWorld\n"));
        assert!(!is_game_process("RimWorldModMan\n"));
        assert!(!is_game_process("steam\n"));
    }

    #[test]
    fn steam_api_is_looked_up_in_the_game_and_unity_plugins() {
        let game = Path::new("/games/RimWorld");
        let found = steam_api_in_game(game);
        assert_eq!(found[0], game.join("libsteam_api.so"));
        assert!(found.contains(&game.join("RimWorldLinux_Data/Plugins/x86_64/libsteam_api.so")));
    }
}
