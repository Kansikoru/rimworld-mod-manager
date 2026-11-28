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
pub const CASE_INSENSITIVE_PATHS: bool = false;

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

/// Unity's `$XDG_CONFIG_HOME/unity3d/<company>/<product>`, the same inside the Flatpak Steam
/// sandbox (a game started from Flatpak Steam keeps its data there), and the Windows build's
/// `AppData\LocalLow` inside its Proton prefix (`steamapps/compatdata/294100`).
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
    for steam in steam_dirs() {
        let proton = steam
            .join("steamapps")
            .join("compatdata")
            .join("294100")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("AppData")
            .join("LocalLow")
            .join(GAME_COMPANY)
            .join(GAME_DATA_NAME);
        if !dirs.contains(&proton) {
            dirs.push(proton);
        }
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

/// What a process has used so far: CPU seconds (user + system, all threads), bytes read
/// (every read, from the page cache too, like Windows counts them) and private memory, from
/// `/proc/<pid>/stat`, `io` and `status`.
pub fn process_usage(pid: u32) -> Option<(f64, u64, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let cpu_secs = cpu_secs_from_stat(&stat)?;
    // `io` is readable for one's own processes only; without it the reads count as 0.
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).unwrap_or_default();
    let read_bytes = proc_field(&io, "rchar:").unwrap_or(0);
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let private_bytes = proc_field(&status, "RssAnon:").unwrap_or(0) * 1024;
    Some((cpu_secs, read_bytes, private_bytes))
}

/// The kernel counts CPU time in ticks of `USER_HZ`, which is 100 on Linux.
const CLOCK_TICKS: f64 = 100.0;

/// `utime` + `stime` of a `/proc/<pid>/stat` line. The name in brackets may hold spaces and
/// brackets itself, so the fields are counted from the last `)`: the state is the first one
/// after it, `utime` and `stime` the 12th and 13th.
fn cpu_secs_from_stat(stat: &str) -> Option<f64> {
    let fields: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some((utime + stime) as f64 / CLOCK_TICKS)
}

/// The number after `name` in a `/proc` file of `Name: value` lines ("RssAnon:  1234 kB").
fn proc_field(text: &str, name: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(name)?.split_whitespace().next()?.parse().ok())
}

/// Dedicated video memory in bytes: what the `amdgpu` driver reports for each card, else
/// `nvidia-smi`; 0 when neither knows (Intel graphics, or a virtual machine).
pub fn dedicated_vram() -> u64 {
    let amd = std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("card") && !n.contains('-'))
        })
        .filter_map(|e| {
            std::fs::read_to_string(e.path().join("device").join("mem_info_vram_total"))
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    if amd > 0 {
        return amd;
    }
    nvidia_vram().unwrap_or(0)
}

/// `nvidia-smi` gives each card's memory in MiB, one per line.
fn nvidia_vram() -> Option<u64> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<u64>().ok())
        .max()
        .map(|mib| mib * 1024 * 1024)
}

/// Starts a game through the Steam client (`steam -applaunch <id> <args>`; the Flatpak client
/// when `steam` isn't on the PATH): for a game only Steam can start, like the Windows build
/// under Proton.
pub fn launch_through_steam(app_id: u32, args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).collect();
    let id = app_id.to_string();
    let native = std::process::Command::new("steam")
        .args(["-applaunch", &id])
        .args(&args)
        .spawn();
    if native.is_ok() {
        return Ok(());
    }
    std::process::Command::new("flatpak")
        .args(["run", "com.valvesoftware.Steam", "-applaunch", &id])
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|_| {
            crate::i18n::tr!(
                "Не нашлось ни Linux-версии игры, ни клиента Steam, чтобы её запустить",
                "Neither the Linux build of the game nor a Steam client to start it was found",
                "既没有找到游戏的 Linux 版本，也没有找到可启动它的 Steam 客户端"
            )
            .to_string()
        })
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
    std::fs::read(system_font_path(file)?).ok()
}

/// Whether the system has this font file.
pub fn has_system_font(file: &str) -> bool {
    system_font_path(file).is_some()
}

fn system_font_path(file: &str) -> Option<PathBuf> {
    font_dirs().into_iter().find_map(|dir| {
        walkdir::WalkDir::new(dir)
            .max_depth(4)
            .into_iter()
            .flatten()
            .find(|e| e.file_type().is_file() && e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(file)))
            .map(|e| e.into_path())
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
    fn cpu_time_and_fields_from_proc() {
        // A name with a space and a bracket; utime 250, stime 50 ticks.
        let stat = "1234 (Rim World) x) S 1 1234 1234 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 30 0 5 0 0";
        assert_eq!(cpu_secs_from_stat(stat), Some(3.0));
        let status = "Name:\tRimWorldLinux\nRssAnon:\t  204800 kB\nRssFile:\t 1000 kB\n";
        assert_eq!(proc_field(status, "RssAnon:"), Some(204_800));
        assert_eq!(proc_field("rchar: 777\nwchar: 1\n", "rchar:"), Some(777));
        assert_eq!(proc_field(status, "VmSwap:"), None);
    }

    #[test]
    fn steam_api_is_looked_up_in_the_game_and_unity_plugins() {
        let game = Path::new("/games/RimWorld");
        let found = steam_api_in_game(game);
        assert_eq!(found[0], game.join("libsteam_api.so"));
        assert!(found.contains(&game.join("RimWorldLinux_Data/Plugins/x86_64/libsteam_api.so")));
    }
}
