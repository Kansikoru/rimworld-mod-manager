//! Mod settings kept with a modpack: a copy of the game's `Config\Mod_*.xml` files (what
//! mods save from their settings windows), taken when the player asks for it and put back
//! when the modpack is enabled. Kept in `%LOCALAPPDATA%\RimWorldModManager\modlist_settings\<modpack>`.
//!
//! Putting a copy back only replaces the files the copy has; settings of other mods stay,
//! and every file that changes is copied to a backup folder first.

use std::path::{Path, PathBuf};

use crate::modlists::is_mod_config_name;

pub fn root() -> PathBuf {
    crate::game_install::manager_data_dir().join("modlist_settings")
}

/// Where replaced settings files go before a copy is put back.
pub fn backup_root() -> PathBuf {
    crate::game_install::manager_data_dir().join("mod_settings_backup")
}

/// A modpack's saved copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub files: usize,
    /// When the copy was taken (unix seconds).
    pub saved_at: Option<u64>,
}

/// What putting a copy back did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Restored {
    /// Files that differed and were replaced (or were missing and were added).
    pub written: usize,
    /// Replaced files copied to the backup folder first.
    pub backed_up: usize,
}

fn dir_for(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

/// `Mod_*.xml` files of a folder, by name.
fn settings_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            is_mod_config_name(&name).then(|| (name, e.path()))
        })
        .collect();
    files.sort();
    files
}

/// The modpack's copy, if it has one.
pub fn saved(root: &Path, name: &str) -> Option<Saved> {
    let dir = dir_for(root, name);
    if !dir.is_dir() {
        return None;
    }
    let saved_at = std::fs::metadata(&dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    Some(Saved {
        files: settings_files(&dir).len(),
        saved_at,
    })
}

/// Takes a new copy of the settings in `config_dir` (the old copy is replaced as a whole).
/// Returns how many files it has.
pub fn save(root: &Path, name: &str, config_dir: &Path) -> Result<usize, String> {
    let dir = dir_for(root, name);
    // Into a folder of its own first: a failure half-way leaves the old copy as it was.
    let fresh = root.join(format!("{name}.new"));
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
    let files = settings_files(config_dir);
    for (file, path) in &files {
        if let Err(e) = std::fs::copy(path, fresh.join(file)) {
            let _ = std::fs::remove_dir_all(&fresh);
            return Err(e.to_string());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&fresh, &dir).map_err(|e| e.to_string())?;
    Ok(files.len())
}

/// Puts the modpack's copy back into `config_dir`. A file that is already the same is left
/// alone; one that differs is copied to `backup_dir` first (and kept if that fails).
pub fn restore(root: &Path, name: &str, config_dir: &Path, backup_dir: &Path) -> Result<Restored, String> {
    let dir = dir_for(root, name);
    let mut done = Restored::default();
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    for (file, path) in settings_files(&dir) {
        let new = std::fs::read(&path).map_err(|e| e.to_string())?;
        let target = config_dir.join(&file);
        if let Ok(old) = std::fs::read(&target) {
            if old == new {
                continue;
            }
            let kept = std::fs::create_dir_all(backup_dir)
                .and_then(|()| std::fs::write(backup_dir.join(&file), &old))
                .is_ok();
            if !kept {
                continue;
            }
            done.backed_up += 1;
        }
        std::fs::write(&target, &new).map_err(|e| e.to_string())?;
        done.written += 1;
    }
    Ok(done)
}

/// Forgets the modpack's copy.
pub fn remove(root: &Path, name: &str) {
    let _ = std::fs::remove_dir_all(dir_for(root, name));
}

/// The copy follows a renamed modpack.
pub fn rename(root: &Path, from: &str, to: &str) {
    let source = dir_for(root, from);
    if source.is_dir() {
        let _ = std::fs::remove_dir_all(dir_for(root, to));
        let _ = std::fs::rename(source, dir_for(root, to));
    }
}

/// A duplicated modpack gets the same copy.
pub fn copy(root: &Path, from: &str, to: &str) {
    let source = dir_for(root, from);
    if !source.is_dir() {
        return;
    }
    let target = dir_for(root, to);
    if std::fs::create_dir_all(&target).is_ok() {
        for (file, path) in settings_files(&source) {
            let _ = std::fs::copy(path, target.join(file));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_set_settings_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_copy_is_taken_and_put_back_with_a_backup() {
        let dir = temp("roundtrip");
        let (root, config, backup) = (dir.join("kept"), dir.join("Config"), dir.join("backup"));
        std::fs::create_dir_all(&config).unwrap();
        let write = |file: &str, text: &str| std::fs::write(config.join(file), text).unwrap();
        let read = |file: &str| std::fs::read_to_string(config.join(file)).unwrap();
        write("Mod_1_A.xml", "a1");
        write("Mod_2_B.xml", "b1");
        write("ModsConfig.xml", "order");
        write("Prefs.xml", "prefs");

        assert_eq!(saved(&root, "Pack"), None);
        assert_eq!(save(&root, "Pack", &config), Ok(2));
        assert_eq!(saved(&root, "Pack").map(|s| s.files), Some(2));

        // The player changes one mod's settings and gets a new mod; the game's own files
        // change too.
        write("Mod_1_A.xml", "a2");
        write("Mod_3_C.xml", "c1");
        write("ModsConfig.xml", "other order");
        std::fs::remove_file(config.join("Mod_2_B.xml")).unwrap();

        let done = restore(&root, "Pack", &config, &backup).unwrap();
        assert_eq!(
            done,
            Restored {
                written: 2,
                backed_up: 1
            }
        );
        assert_eq!((read("Mod_1_A.xml"), read("Mod_2_B.xml")), ("a1".into(), "b1".into()));
        // What the copy doesn't have stays as it is.
        assert_eq!(
            (read("Mod_3_C.xml"), read("ModsConfig.xml"), read("Prefs.xml")),
            ("c1".into(), "other order".into(), "prefs".into())
        );
        assert_eq!(std::fs::read_to_string(backup.join("Mod_1_A.xml")).unwrap(), "a2");
        // A second time changes nothing.
        assert_eq!(restore(&root, "Pack", &config, &backup).unwrap(), Restored::default());

        // A new copy replaces the old one as a whole.
        std::fs::remove_file(config.join("Mod_2_B.xml")).unwrap();
        assert_eq!(save(&root, "Pack", &config), Ok(2));
        assert!(!root.join("Pack").join("Mod_2_B.xml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_copy_follows_the_modpack() {
        let dir = temp("follow");
        let (root, config) = (dir.join("kept"), dir.join("Config"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("Mod_1_A.xml"), "a").unwrap();
        save(&root, "Pack", &config).unwrap();

        copy(&root, "Pack", "Pack (copy)");
        rename(&root, "Pack", "Renamed");
        assert_eq!(saved(&root, "Pack"), None);
        assert_eq!(saved(&root, "Renamed").map(|s| s.files), Some(1));
        assert_eq!(saved(&root, "Pack (copy)").map(|s| s.files), Some(1));
        remove(&root, "Renamed");
        assert_eq!(saved(&root, "Renamed"), None);
        // A modpack without a copy has nothing to follow it.
        copy(&root, "Nothing", "Nothing (copy)");
        assert_eq!(saved(&root, "Nothing (copy)"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
