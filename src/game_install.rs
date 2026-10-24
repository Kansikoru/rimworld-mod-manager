//! Detects which RimWorld build is installed and where its engine-specific files are.
//!
//! RimWorld 1.6 and older run on Unity:
//!   `RimWorldWin64.exe`, `RimWorldWin64_Data/Plugins/x86_64/steam_api64.dll`.
//! RimWorld 1.7+ runs on Ludeon's own Waylith engine (.NET 10):
//!   `RimWorld.exe` (+ `Waylith.dll`), `steam_api64.dll` in the game root.

use std::path::{Path, PathBuf};

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
    if major.chars().all(|c| c.is_ascii_digit()) && minor.chars().all(|c| c.is_ascii_digit()) && !major.is_empty() && !minor.is_empty() {
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
    candidates
        .iter()
        .map(|name| game_dir.join(name))
        .find(|p| p.is_file())
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
    } else if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        exe_dir.join("RimWorldModManagerData")
    } else {
        PathBuf::from("RimWorldModManagerData")
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
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
        for m in &mods {
            println!(
                "  {:40} trans={:5} defs={:5} dll={:5} other={:5} langs={:?}",
                m.package_id, m.is_translation_mod, m.has_defs, m.has_assemblies, m.has_other_content, m.languages
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
        assert_eq!(game_exe_path(&dir, GameEngine::Unity), Some(dir.join("RimWorldWin64.exe")));
        assert_eq!(game_steam_dll_path(&dir), Some(plugins.join("steam_api64.dll")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
