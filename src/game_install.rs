//! Detects which RimWorld build is installed and where its engine-specific files are.
//!
//! RimWorld 1.6 and older run on Unity:
//!   `RimWorldWin64.exe`, `RimWorldWin64_Data/Plugins/x86_64/steam_api64.dll`.
//! RimWorld 1.7+ runs on Ludeon's own Waylith engine (.NET 10):
//!   `RimWorld.exe` (+ `Waylith.dll`), `steam_api64.dll` in the game root.

use std::path::{Path, PathBuf};

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameEngine {
    #[default]
    Unknown,
    /// RimWorld 1.6 and older.
    Unity,
    /// RimWorld 1.7 and newer.
    Waylith,
}

impl GameEngine {
    pub fn label(&self) -> &'static str {
        match self {
            GameEngine::Unknown => "?",
            GameEngine::Unity => "Unity",
            GameEngine::Waylith => "Waylith",
        }
    }
}

pub const UNITY_EXE: &str = "RimWorldWin64.exe";
pub const WAYLITH_EXE: &str = "RimWorld.exe";

/// Detects the engine from the files in the game folder.
pub fn detect_engine(game_dir: &Path) -> GameEngine {
    let has_waylith = game_dir.join("Waylith.dll").is_file() || game_dir.join("RimWorld.dll").is_file();
    let has_waylith_exe = game_dir.join(WAYLITH_EXE).is_file();
    let has_unity_exe = game_dir.join(UNITY_EXE).is_file();

    if has_waylith && has_waylith_exe {
        GameEngine::Waylith
    } else if has_unity_exe {
        GameEngine::Unity
    } else if has_waylith_exe {
        GameEngine::Waylith
    } else {
        GameEngine::Unknown
    }
}

/// Reads `Version.txt`, e.g. "1.7.4985 rev1269".
pub fn read_game_version(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_dir.join("Version.txt")).ok()?;
    let line = text.lines().next()?.trim().to_string();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// "1.7.4985 rev1269" -> "1.7"
pub fn major_minor(version: &str) -> Option<String> {
    let mut parts = version.trim().split(|c: char| c == '.' || c.is_whitespace());
    let major = parts.next()?.trim();
    let minor = parts.next()?.trim();
    if major.chars().all(|c| c.is_ascii_digit())
        && minor.chars().all(|c| c.is_ascii_digit())
        && !major.is_empty()
        && !minor.is_empty()
    {
        Some(format!("{major}.{minor}"))
    } else {
        None
    }
}

/// Game executable for the detected engine.
pub fn game_exe_path(game_dir: &Path, engine: GameEngine) -> Option<PathBuf> {
    let candidates: &[&str] = match engine {
        GameEngine::Waylith => &[WAYLITH_EXE, UNITY_EXE],
        GameEngine::Unity => &[UNITY_EXE, WAYLITH_EXE],
        GameEngine::Unknown => &[WAYLITH_EXE, UNITY_EXE],
    };
    candidates.iter().map(|name| game_dir.join(name)).find(|p| p.is_file())
}

/// `steam_api64.dll` shipped with the game: game root (1.7) or Unity plugin folder (1.6).
pub fn game_steam_dll_path(game_dir: &Path) -> Option<PathBuf> {
    [
        game_dir.join("steam_api64.dll"),
        game_dir
            .join("RimWorldWin64_Data")
            .join("Plugins")
            .join("x86_64")
            .join("steam_api64.dll"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// RimWorld user data folder: `%USERPROFILE%\AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios`.
pub fn user_data_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("USERPROFILE").map(|p| {
            PathBuf::from(p)
                .join("AppData")
                .join("LocalLow")
                .join("Ludeon Studios")
                .join("RimWorld by Ludeon Studios")
        }),
        std::env::var_os("LOCALAPPDATA").and_then(|p| {
            PathBuf::from(p).parent().map(|parent| {
                parent
                    .join("LocalLow")
                    .join("Ludeon Studios")
                    .join("RimWorld by Ludeon Studios")
            })
        }),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}

/// The manager's own data folder: `%LOCALAPPDATA%\RimWorldModManager` (created if missing).
pub fn manager_data_dir() -> PathBuf {
    let dir = if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(local).join("RimWorldModManager")
    } else if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        exe_dir.join("RimWorldModManagerData")
    } else {
        PathBuf::from("RimWorldModManagerData")
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Whether RimWorld is running right now (1.6: RimWorldWin64.exe, 1.7: RimWorld.exe).
pub fn is_game_running() -> bool {
    game_pid().is_some()
}

/// Process ID of the running game (either engine), if any.
pub fn game_pid() -> Option<u32> {
    let mut cmd = std::process::Command::new("tasklist");
    cmd.args(["/NH", "/FO", "CSV"]);
    #[cfg(windows)]
    {
        // No console window flashing: this app has no console of its own.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
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

/// What the game process is doing right now (F11): CPU time used so far (user + kernel, all
/// threads, seconds), bytes read from disk so far, private memory, and whether its window
/// answers (`None`: no visible window yet; `Some(false)`: "Not responding").
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProcessStats {
    pub cpu_secs: f64,
    pub read_bytes: u64,
    pub private_bytes: u64,
    pub responsive: Option<bool>,
}

#[cfg(windows)]
pub fn process_stats(pid: u32) -> Option<ProcessStats> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_ops: u64,
        write_ops: u64,
        other_ops: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct MemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_non_paged_pool: usize,
        quota_non_paged_pool: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn GetProcessIoCounters(process: *mut c_void, counters: *mut IoCounters) -> i32;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut MemoryCounters, cb: u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const PROCESS_VM_READ: u32 = 0x0010;
    let ticks = |t: &FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
    // SAFETY: plain Win32 calls; the handle is checked and closed, the out structs are ours
    // and sized as the API expects.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        let (mut c, mut e, mut k, mut u) = Default::default();
        let times_ok = GetProcessTimes(handle, &mut c, &mut e, &mut k, &mut u) != 0;
        let mut io = IoCounters::default();
        let io_ok = GetProcessIoCounters(handle, &mut io) != 0;
        let mut mem = MemoryCounters {
            cb: std::mem::size_of::<MemoryCounters>() as u32,
            ..Default::default()
        };
        let mem_ok = K32GetProcessMemoryInfo(handle, &mut mem, mem.cb) != 0;
        CloseHandle(handle);
        if !(times_ok && io_ok && mem_ok) {
            return None;
        }
        Some(ProcessStats {
            // FILETIME counts 100 ns ticks.
            cpu_secs: (ticks(&k) + ticks(&u)) as f64 / 10_000_000.0,
            read_bytes: io.read_bytes,
            private_bytes: mem.private_usage as u64,
            responsive: window_responsive(pid),
        })
    }
}

#[cfg(not(windows))]
pub fn process_stats(_pid: u32) -> Option<ProcessStats> {
    None
}

/// Whether the process's main window answers (Windows calls it "Not responding" after 5 s
/// without an answer, and then shows a ghost window of its own instead). `None`: no visible
/// top-level window.
#[cfg(windows)]
fn window_responsive(pid: u32) -> Option<bool> {
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(*mut c_void, isize) -> i32, param: isize) -> i32;
        fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(window: *mut c_void) -> i32;
        fn GetWindow(window: *mut c_void, cmd: u32) -> *mut c_void;
        fn IsHungAppWindow(window: *mut c_void) -> i32;
    }
    const GW_OWNER: u32 = 4;
    struct Search {
        pid: u32,
        found: Option<bool>,
    }
    unsafe extern "system" fn visit(window: *mut c_void, param: isize) -> i32 {
        // SAFETY: `param` is the `Search` below, alive for the whole EnumWindows call.
        let search = unsafe { &mut *(param as *mut Search) };
        let mut owner_pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut owner_pid) };
        let main_window = unsafe { IsWindowVisible(window) != 0 && GetWindow(window, GW_OWNER).is_null() };
        if owner_pid == search.pid && main_window {
            search.found = Some(unsafe { IsHungAppWindow(window) } == 0);
            return 0; // stop
        }
        1
    }
    let mut search = Search { pid, found: None };
    // SAFETY: the callback only reads windows and writes `search`, which outlives the call.
    unsafe { EnumWindows(visit, &mut search as *mut Search as isize) };
    search.found
}

/// Folders where RimWorld 1.7 keeps its own compressed textures and compiled shaders.
pub const GAME_CACHE_DIRS: &[&str] = &["TextureCache", "ShaderCache"];

/// Total size in bytes of the game's cache folders (0 if there are none).
pub fn game_cache_size() -> u64 {
    let Some(base) = user_data_dir() else {
        return 0;
    };
    GAME_CACHE_DIRS
        .iter()
        .map(|d| base.join(d))
        .filter(|p| p.is_dir())
        .flat_map(|p| walkdir::WalkDir::new(p).into_iter().flatten())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
        .sum()
}

/// Moves the 1.7 texture and shader caches to the Recycle Bin; the game rebuilds them on the
/// next start. Refuses while the game is running (it keeps the cache files open).
pub fn clear_game_caches() -> Result<Vec<String>, String> {
    if is_game_running() {
        return Err(tr!(
            "Закройте RimWorld: игра держит файлы кэша открытыми",
            "Close RimWorld first: the game keeps its cache files open",
            "请先关闭 RimWorld：游戏会占用其缓存文件"
        )
        .to_string());
    }
    let base = user_data_dir().ok_or(tr!(
        "Папка данных RimWorld не найдена",
        "RimWorld data folder not found",
        "找不到 RimWorld 数据文件夹"
    ))?;
    let mut cleared = Vec::new();
    for dir in GAME_CACHE_DIRS {
        let path = base.join(dir);
        if path.is_dir() {
            trash::delete(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            cleared.push(dir.to_string());
        }
    }
    Ok(cleared)
}

/// Launch flags that make sense for the engine. Returns (flag, verified_for_engine).
pub fn known_launch_flags(engine: GameEngine) -> &'static [(&'static str, bool)] {
    match engine {
        GameEngine::Waylith => &[
            ("-ColdBoot", true),
            ("-console", true),
            ("-NoFileLogCallstacks", true),
            ("-Verbose", true),
            ("-quicktest", false),
        ],
        _ => &[("-quicktest", true), ("-logfile", true), ("-popupwindow", true)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn cpu_time_of_a_running_process_is_read() {
        // Burn a little CPU so the counter is above zero.
        let mut x = 0u64;
        for i in 0..20_000_000u64 {
            x = x.wrapping_add(i ^ (x >> 3));
        }
        std::hint::black_box(x);
        let stats = process_stats(std::process::id()).expect("own process");
        assert!(stats.cpu_secs > 0.0, "{stats:?}");
        assert!(stats.private_bytes > 0, "{stats:?}");
        assert_eq!(process_stats(0xFFFF_FFF0), None, "no such process");
    }

    /// Live: Explorer always has a visible window (the taskbar), which answers.
    /// `cargo test explorer_window_answers -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn explorer_window_answers() {
        let out = std::process::Command::new("tasklist")
            .args(["/NH", "/FO", "CSV", "/FI", "IMAGENAME eq explorer.exe"])
            .output()
            .unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        let pid: u32 = list
            .lines()
            .find_map(|r| r.split(',').nth(1)?.trim_matches('"').parse().ok())
            .expect("explorer.exe");
        let stats = process_stats(pid).expect("explorer stats");
        println!("{stats:?}");
        assert_eq!(stats.responsive, Some(true));
        // The test process has no window.
        assert_eq!(process_stats(std::process::id()).unwrap().responsive, None);
    }

    #[test]
    fn game_pid_is_read_from_tasklist_csv() {
        let list = "\"System\",\"4\",\"Services\",\"0\",\"144 K\"\r\n\
                    \"RimWorldWin64.exe\",\"12345\",\"Console\",\"1\",\"3,104,220 K\"\r\n";
        assert_eq!(find_game_pid(list), Some(12345));
        assert_eq!(
            find_game_pid("\"RimWorld.exe\",\"77\",\"Console\",\"1\",\"1 K\""),
            Some(77)
        );
        // A name that only contains the exe name is another program.
        assert_eq!(
            find_game_pid("\"MyRimWorldWin64.exe\",\"9\",\"Console\",\"1\",\"1 K\""),
            None
        );
    }

    #[test]
    fn parses_major_minor() {
        assert_eq!(major_minor("1.7.4985 rev1269").as_deref(), Some("1.7"));
        assert_eq!(major_minor("1.6.4871 rev591").as_deref(), Some("1.6"));
        assert_eq!(major_minor("garbage"), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_waylith_layout() {
        let dir = temp_dir("waylith");
        std::fs::write(dir.join("RimWorld.exe"), b"").unwrap();
        std::fs::write(dir.join("Waylith.dll"), b"").unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"").unwrap();
        assert_eq!(detect_engine(&dir), GameEngine::Waylith);
        assert_eq!(game_exe_path(&dir, GameEngine::Waylith), Some(dir.join("RimWorld.exe")));
        assert_eq!(game_steam_dll_path(&dir), Some(dir.join("steam_api64.dll")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read-only check against the RimWorld installed on this machine:
    /// `cargo test real_install_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_install_smoke() {
        let paths = crate::steam::detect_steam_paths();
        println!("game:    {:?}", paths.game_path);
        println!("version: {:?} -> {:?}", paths.game_version, paths.game_major_minor());
        println!("engine:  {:?}", paths.engine);
        println!("exe:     {:?}", paths.exe_path);
        println!("dll:     {:?}", paths.steam_dll_path);
        println!("mods:    {:?} | data: {:?}", paths.workshop_path, paths.game_data_path);
        let logs = crate::analyzer::detect_rimworld_log_paths();
        println!("logs:    {:?}", logs);

        let mut mods = crate::scanner::scan_mods(
            paths.workshop_path.as_deref(),
            paths.local_mods_path.as_deref(),
            paths.game_data_path.as_deref(),
        );
        crate::matcher::classify_and_link_translations(&mut mods);
        crate::categories::assign_categories(&mut mods, &Default::default());
        for m in &mods {
            println!(
                "  {:40} trans={:5} dll={:5} cats={:?} stats={:?}",
                m.package_id, m.is_translation_mod, m.has_assemblies, m.categories, m.def_stats
            );
        }
        if let Some((_, log)) = logs.first() {
            let r = crate::analyzer::analyze_rimworld_log(log, &mods);
            println!(
                "log: waylith={} lines={} errors={} crash={} culprits={:?}",
                r.is_waylith,
                r.total_lines,
                r.total_errors,
                r.has_hard_crash,
                r.culprits.iter().map(|c| (&c.mod_name, c.severity)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn detects_unity_layout() {
        let dir = temp_dir("unity");
        std::fs::write(dir.join("RimWorldWin64.exe"), b"").unwrap();
        let plugins = dir.join("RimWorldWin64_Data").join("Plugins").join("x86_64");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("steam_api64.dll"), b"").unwrap();
        assert_eq!(detect_engine(&dir), GameEngine::Unity);
        assert_eq!(
            game_exe_path(&dir, GameEngine::Unity),
            Some(dir.join("RimWorldWin64.exe"))
        );
        assert_eq!(game_steam_dll_path(&dir), Some(plugins.join("steam_api64.dll")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
