//! The companion mod (F11): a tiny RimWorld 1.6 and 1.7 mod, built from `companion/` and kept
//! inside the manager. Once the game has finished loading it writes the loading time to
//! Player.log; the analyzer reads it from there. Exact, unlike the manager's own guess from
//! outside.

use std::path::{Path, PathBuf};

pub const PACKAGE_ID: &str = "kansikoru.rmm.companion";
/// Folder name in the game's Mods folder.
pub const FOLDER: &str = "RimWorldModManagerCompanion";
const DLL_NAME: &str = "RimWorldModManagerCompanion.dll";

const ABOUT_XML: &[u8] = include_bytes!("../companion/About/About.xml");
/// For 1.6 (Unity, .NET Framework) and 1.7 (.NET 10): each game loads its own version folder.
const DLL_1_6: &[u8] = include_bytes!("../companion/1.6/Assemblies/RimWorldModManagerCompanion.dll");
const DLL_1_7: &[u8] = include_bytes!("../companion/1.7/Assemblies/RimWorldModManagerCompanion.dll");

/// The line the mod writes (`LoadTimeReporter.Marker` in companion/Source).
const MARKER: &str = "[RimWorld Mod Manager Companion] Loading finished in ";

/// The files of the mod: (path inside its folder, contents).
fn files() -> [(PathBuf, &'static [u8]); 3] {
    [
        (Path::new("About").join("About.xml"), ABOUT_XML),
        (Path::new("1.6").join("Assemblies").join(DLL_NAME), DLL_1_6),
        (Path::new("1.7").join("Assemblies").join(DLL_NAME), DLL_1_7),
    ]
}

/// The dll of 2.1.4, outside the version folders: the game would load it as well.
fn legacy_dll() -> PathBuf {
    Path::new("Assemblies").join(DLL_NAME)
}

/// Where the mod lives in the game's Mods folder.
pub fn install_dir(mods_dir: &Path) -> PathBuf {
    mods_dir.join(FOLDER)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Missing,
    /// Installed, but older (or changed): the manager's copy is newer.
    Outdated,
    Current,
}

pub fn state(mods_dir: &Path) -> State {
    let dir = install_dir(mods_dir);
    if !dir.join("About").join("About.xml").is_file() {
        return State::Missing;
    }
    let same = files()
        .iter()
        .all(|(rel, bytes)| std::fs::read(dir.join(rel)).is_ok_and(|on_disk| on_disk == *bytes));
    if same && !dir.join(legacy_dll()).exists() {
        State::Current
    } else {
        State::Outdated
    }
}

/// Writes (or rewrites) the mod into the game's Mods folder.
pub fn install(mods_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = install_dir(mods_dir);
    for (rel, bytes) in files() {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Through a temporary file: a half-written dll would break the game's loading.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
    }
    let legacy = dir.join(legacy_dll());
    if legacy.exists() {
        std::fs::remove_file(&legacy)?;
        if let Some(parent) = legacy.parent() {
            // Only goes if empty.
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(dir)
}

/// Moves the mod's folder to the Recycle Bin.
pub fn remove(mods_dir: &Path) -> Result<(), String> {
    let dir = install_dir(mods_dir);
    if !dir.exists() {
        return Ok(());
    }
    trash::delete(&dir).map_err(|e| e.to_string())
}

/// The loading time the mod wrote to the log, in seconds (the last one: a log has one run).
pub fn load_time_from_log(content: &str) -> Option<f32> {
    content.lines().rev().find_map(|line| {
        let rest = &line[line.find(MARKER)? + MARKER.len()..];
        let secs: f32 = rest.split_whitespace().next()?.parse().ok()?;
        (secs > 0.0 && secs < 3600.0).then_some(secs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_companion_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn embedded_mod_is_complete() {
        let about = std::str::from_utf8(ABOUT_XML).unwrap();
        assert!(about.contains(&format!("<packageId>{PACKAGE_ID}</packageId>")));
        assert!(about.contains("<li>1.6</li>") && about.contains("<li>1.7</li>"));
        // .NET assemblies ("MZ" header), each built for its game.
        for (dll, framework) in [
            (DLL_1_6, ".NETFramework,Version=v4.7.2"),
            (DLL_1_7, ".NETCoreApp,Version=v10.0"),
        ] {
            assert_eq!(&dll[..2], b"MZ");
            assert!(contains(dll, framework), "built for {framework}");
            assert!(contains(dll, "Assembly-CSharp"), "built against the game");
        }
        let source = include_str!("../companion/Source/LoadTimeReporter.cs");
        assert!(source.contains(MARKER), "the mod writes the line the manager reads");
    }

    #[test]
    fn install_update_and_state() {
        let mods = temp_dir("install");
        assert_eq!(state(&mods), State::Missing);
        let dir = install(&mods).unwrap();
        assert_eq!(dir, mods.join(FOLDER));
        assert_eq!(state(&mods), State::Current);
        let dll_1_6 = dir.join("1.6").join("Assemblies").join(DLL_NAME);
        assert!(dll_1_6.is_file());
        assert!(dir.join("1.7").join("Assemblies").join(DLL_NAME).is_file());

        // An older dll: outdated until installed again.
        std::fs::write(&dll_1_6, b"old").unwrap();
        assert_eq!(state(&mods), State::Outdated);
        install(&mods).unwrap();
        assert_eq!(state(&mods), State::Current);
        assert!(!dll_1_6.with_extension("tmp").exists());

        let _ = std::fs::remove_dir_all(&mods);
    }

    /// 2.1.4 kept the dll in Assemblies/; updating removes it, or 1.6 would load it twice.
    #[test]
    fn update_removes_the_old_layout() {
        let mods = temp_dir("legacy");
        let dir = install_dir(&mods);
        std::fs::create_dir_all(dir.join("About")).unwrap();
        std::fs::write(dir.join("About").join("About.xml"), b"old").unwrap();
        std::fs::create_dir_all(dir.join("Assemblies")).unwrap();
        std::fs::write(dir.join(legacy_dll()), DLL_1_6).unwrap();
        assert_eq!(state(&mods), State::Outdated);

        install(&mods).unwrap();
        assert!(!dir.join("Assemblies").exists());
        assert_eq!(state(&mods), State::Current);

        // Even with the new files in place, a leftover old dll still needs the update.
        std::fs::create_dir_all(dir.join("Assemblies")).unwrap();
        std::fs::write(dir.join(legacy_dll()), DLL_1_6).unwrap();
        assert_eq!(state(&mods), State::Outdated);

        let _ = std::fs::remove_dir_all(&mods);
    }

    #[test]
    fn load_time_is_read_from_the_log() {
        let log = "Initialize engine version\n\
                   [RimWorld Mod Manager Companion] Loading finished in 143.27 s\n\
                   Some later line\n";
        assert_eq!(load_time_from_log(log), Some(143.27));
        // 1.7 puts the time and level before a message and the stack after it.
        let log = "2026-11-02 12:19:01.123  INFO [RimWorld Mod Manager Companion] Loading finished in 95.10 s\n    \
                   at Brutal.Logging.LogCategory.Info(LogString`1 message) [0x0]\n";
        assert_eq!(load_time_from_log(log), Some(95.10));
        assert_eq!(load_time_from_log("no such line"), None);
        assert_eq!(
            load_time_from_log("[RimWorld Mod Manager Companion] Loading finished in nonsense s"),
            None
        );
    }
}
