//! The companion mod (F11): a tiny RimWorld 1.6 mod, built from `companion/` and kept inside
//! the manager. Once the game has finished loading it writes the loading time to Player.log;
//! the analyzer reads it from there. Exact, unlike the manager's own guess from outside.

use std::path::{Path, PathBuf};

pub const PACKAGE_ID: &str = "kansikoru.rmm.companion";
/// Folder name in the game's Mods folder.
pub const FOLDER: &str = "RimWorldModManagerCompanion";
const DLL_NAME: &str = "RimWorldModManagerCompanion.dll";

const ABOUT_XML: &[u8] = include_bytes!("../companion/About/About.xml");
const DLL: &[u8] = include_bytes!("../companion/Assemblies/RimWorldModManagerCompanion.dll");

/// The line the mod writes (`LoadTimeReporter.Marker` in companion/Source).
const MARKER: &str = "[RimWorld Mod Manager Companion] Loading finished in ";

/// The files of the mod: (path inside its folder, contents).
fn files() -> [(PathBuf, &'static [u8]); 2] {
    [
        (Path::new("About").join("About.xml"), ABOUT_XML),
        (Path::new("Assemblies").join(DLL_NAME), DLL),
    ]
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
    if same {
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

    #[test]
    fn embedded_mod_is_complete() {
        let about = std::str::from_utf8(ABOUT_XML).unwrap();
        assert!(about.contains(&format!("<packageId>{PACKAGE_ID}</packageId>")));
        // A .NET assembly: "MZ" header.
        assert_eq!(&DLL[..2], b"MZ");
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
        assert!(dir.join("Assemblies").join(DLL_NAME).is_file());

        // An older dll: outdated until installed again.
        std::fs::write(dir.join("Assemblies").join(DLL_NAME), b"old").unwrap();
        assert_eq!(state(&mods), State::Outdated);
        install(&mods).unwrap();
        assert_eq!(state(&mods), State::Current);
        assert!(!dir.join("Assemblies").join("RimWorldModManagerCompanion.tmp").exists());

        let _ = std::fs::remove_dir_all(&mods);
    }

    #[test]
    fn load_time_is_read_from_the_log() {
        let log = "Initialize engine version\n\
                   [RimWorld Mod Manager Companion] Loading finished in 143.27 s\n\
                   Some later line\n";
        assert_eq!(load_time_from_log(log), Some(143.27));
        assert_eq!(load_time_from_log("no such line"), None);
        assert_eq!(
            load_time_from_log("[RimWorld Mod Manager Companion] Loading finished in nonsense s"),
            None
        );
    }
}
