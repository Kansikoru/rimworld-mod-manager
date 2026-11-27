//! Previous versions of mods (F9 of the plan, item 20): when the update check finds a mod
//! with a newer Workshop version, the installed copy is saved before Steam or SteamCMD
//! replaces it, so "Restore previous version" can bring it back after a broken update.
//!
//! One copy per mod (the version before the newest update) in
//! `%LOCALAPPDATA%\RimWorldModManager\mod_versions\<lowercase package ID>\`, with
//! `rmm_version.json` beside the files. Restoring puts the copy into the game's Mods folder
//! frozen (`freeze.rs`): the game loads it, and nothing updates it until it is unfrozen.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The note file in a saved copy.
pub const META: &str = "rmm_version.json";

/// What a saved copy is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedVersion {
    pub package_id: String,
    pub name: String,
    #[serde(default)]
    pub workshop_id: Option<String>,
    /// Unix time the copy was saved.
    pub saved_at: u64,
    /// Newest file date of the copy (the same date means the same version).
    #[serde(default)]
    pub files_time: u64,
    /// Bytes on disk.
    #[serde(default)]
    pub size: u64,
    /// The folder with the copy (not stored: it is where the note lies).
    #[serde(skip)]
    pub dir: PathBuf,
}

/// One mod to save: package ID, name, Workshop ID, its folder now.
#[derive(Debug, Clone)]
pub struct SaveItem {
    pub package_id: String,
    pub name: String,
    pub workshop_id: Option<String>,
    pub dir: PathBuf,
}

pub fn versions_dir() -> PathBuf {
    crate::game_install::manager_data_dir().join("mod_versions")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Saved copies in `root`, by lowercase package ID. Folders without a readable note are skipped.
pub fn list(root: &Path) -> HashMap<String, SavedVersion> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let text = std::fs::read_to_string(dir.join(META)).ok()?;
            let mut v: SavedVersion = serde_json::from_str(&text).ok()?;
            v.dir = dir;
            Some((v.package_id.to_lowercase(), v))
        })
        .collect()
}

/// A folder name Windows accepts, from a package ID.
fn folder_for(root: &Path, package_id: &str) -> PathBuf {
    let name: String = package_id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    root.join(name)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    crate::fs_util::copy_dir(from, to).map_err(std::io::Error::from)
}

/// Saves the copies (blocking — call from a background task). A mod whose saved copy
/// already has the same files is skipped; a different older copy is replaced.
/// Returns the saved ones and the errors (one line per mod).
pub fn save(root: &Path, items: &[SaveItem]) -> (Vec<SavedVersion>, Vec<String>) {
    let existing = list(root);
    let mut saved = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let files_time = crate::mod_updates::newest_file_time(&item.dir).unwrap_or(0);
        if existing
            .get(&item.package_id.to_lowercase())
            .is_some_and(|v| v.files_time == files_time && files_time > 0)
        {
            continue;
        }
        let dest = folder_for(root, &item.package_id);
        // The new copy goes next to the old one first, so a failure keeps the old copy.
        let tmp = dest.with_extension("saving");
        let _ = std::fs::remove_dir_all(&tmp);
        let version = SavedVersion {
            package_id: item.package_id.to_lowercase(),
            name: item.name.clone(),
            workshop_id: item.workshop_id.clone(),
            saved_at: now(),
            files_time,
            size: crate::app::folder_size(&item.dir),
            dir: dest.clone(),
        };
        let result = copy_dir(&item.dir, &tmp)
            .and_then(|()| {
                let json = serde_json::to_string_pretty(&version).map_err(std::io::Error::other)?;
                std::fs::write(tmp.join(META), json)
            })
            .and_then(|()| {
                if dest.exists() {
                    std::fs::remove_dir_all(&dest)?;
                }
                std::fs::rename(&tmp, &dest)
            });
        match result {
            Ok(()) => saved.push(version),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                errors.push(format!("{}: {e}", item.name));
            }
        }
    }
    (saved, errors)
}

/// Puts a saved copy back (blocking): into a new frozen folder in `mods_dir` for a Steam
/// subscription (Steam keeps its own folder), or in place of `current_dir` for a mod that
/// lives in the Mods folder (the current files go to the Recycle Bin first).
/// Returns the folder the game loads now.
pub fn restore(
    version: &SavedVersion,
    current_dir: &Path,
    from_workshop: bool,
    mods_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let fail = |path: &Path, e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let dest = if from_workshop {
        let mods_dir =
            mods_dir.ok_or_else(|| crate::i18n::text(crate::i18n::TranslationKey::FreezeNoModsDir).to_string())?;
        crate::freeze::free_dir(mods_dir, &version.name)
    } else {
        crate::cleaner::move_mod_dir_to_trash(current_dir)?;
        current_dir.to_path_buf()
    };
    if let Err(e) = copy_dir(&version.dir, &dest) {
        if from_workshop {
            let _ = std::fs::remove_dir_all(&dest);
        }
        return Err(fail(&dest, &e));
    }
    let _ = std::fs::remove_file(dest.join(META));
    crate::freeze::mark_frozen(&dest, version.workshop_id.clone(), from_workshop)?;
    Ok(dest)
}

/// Deletes one saved copy, or all of them (`None`).
pub fn delete(root: &Path, package_id: Option<&str>) -> std::io::Result<()> {
    match package_id {
        Some(id) => {
            let dir = folder_for(root, id);
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
            Ok(())
        }
        None if root.exists() => std::fs::remove_dir_all(root),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_versions_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mod_dir(dir: &Path, text: &str) {
        std::fs::create_dir_all(dir.join("About")).unwrap();
        std::fs::write(dir.join("About").join("About.xml"), "<ModMetaData/>").unwrap();
        std::fs::write(dir.join("a.txt"), text).unwrap();
    }

    #[test]
    fn saved_once_per_version_and_restored_frozen() {
        let root = temp("save");
        let (store, workshop, mods) = (root.join("store"), root.join("workshop").join("123"), root.join("Mods"));
        std::fs::create_dir_all(&mods).unwrap();
        mod_dir(&workshop, "old");
        let item = SaveItem {
            package_id: "Author.Mod".into(),
            name: "My Mod".into(),
            workshop_id: Some("123".into()),
            dir: workshop.clone(),
        };

        let (saved, errors) = save(&store, std::slice::from_ref(&item));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(saved.len(), 1);
        let listed = list(&store);
        let v = &listed["author.mod"];
        assert_eq!((v.name.as_str(), v.workshop_id.as_deref()), ("My Mod", Some("123")));
        assert!(v.size > 0 && v.dir.join("a.txt").is_file());
        // The same files again: nothing new is saved.
        assert!(save(&store, std::slice::from_ref(&item)).0.is_empty());

        // Steam updates its folder; the old copy goes into Mods, frozen.
        std::fs::write(workshop.join("a.txt"), "new").unwrap();
        let dest = restore(v, &workshop, true, Some(&mods)).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "old");
        assert!(crate::freeze::is_frozen(&dest) && !dest.join(META).exists());
        assert_eq!(
            std::fs::read_to_string(workshop.join("a.txt")).unwrap(),
            "new",
            "Steam's folder untouched"
        );
        assert!(
            restore(v, &workshop, true, None).is_err(),
            "no Mods folder to restore into"
        );

        delete(&store, Some("AUTHOR.MOD")).unwrap();
        assert!(list(&store).is_empty());
        delete(&store, None).unwrap();
        assert!(!store.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
