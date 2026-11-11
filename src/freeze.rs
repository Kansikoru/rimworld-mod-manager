//! Freezing a mod: the version installed now stays, whatever its author publishes later.
//!
//! - A Steam subscription is copied into the game's Mods folder. RimWorld (and this
//!   manager) prefer a local copy to the Workshop one with the same packageId, so Steam
//!   goes on updating its own folder and the game goes on loading the copy.
//! - A mod that already lives in the Mods folder (SteamCMD downloads) is only marked:
//!   nothing updates it but this manager, and the manager leaves a frozen mod alone.
//!
//! The mark is a small file in the mod's folder, so it survives a rescan and a reinstall
//! of the manager.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::i18n::{fmt, text, TranslationKey};
use crate::models::{ModInfo, ModOrigin};

/// The mark's file name in the mod's folder.
pub const MARKER: &str = "rmm_frozen.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Frozen {
    #[serde(default)]
    pub workshop_id: Option<String>,
    /// Unix time of the freeze.
    #[serde(default)]
    pub frozen_at: u64,
    /// The folder is a copy made by freezing (unfreezing removes it); `false`: the mod's
    /// own folder, only marked.
    #[serde(default)]
    pub copy: bool,
}

pub fn is_frozen(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

pub fn read(dir: &Path) -> Option<Frozen> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(MARKER)).ok()?).ok()
}

fn failed(path: &Path, e: impl std::fmt::Display) -> String {
    fmt(
        text(TranslationKey::FreezeFileError),
        &[&path.display().to_string(), &e.to_string()],
    )
}

fn write_marker(dir: &Path, frozen: &Frozen) -> Result<(), String> {
    let path = dir.join(MARKER);
    let json = serde_json::to_string_pretty(frozen).map_err(|e| failed(&path, e))?;
    std::fs::write(&path, json).map_err(|e| failed(&path, e))
}

/// A folder name Windows accepts, from a mod's name.
fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim();
    if cleaned.is_empty() {
        "mod".to_string()
    } else {
        cleaned.to_string()
    }
}

/// A folder in `mods_dir` that doesn't exist yet: "Name (frozen)", "Name (frozen 2)", ...
fn free_dir(mods_dir: &Path, name: &str) -> PathBuf {
    let base = folder_name(name);
    let first = mods_dir.join(format!("{base} (frozen)"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| mods_dir.join(format!("{base} (frozen {n})")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| failed(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| failed(from, e))? {
        let entry = entry.map_err(|e| failed(from, e))?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| failed(&src, e))?;
        }
    }
    Ok(())
}

/// Freezes a mod; returns the folder the game will load from now on.
/// `mods_dir` is the game's Mods folder (needed for a Steam subscription).
pub fn freeze(m: &ModInfo, workshop_id: Option<String>, mods_dir: Option<&Path>) -> Result<PathBuf, String> {
    let mut frozen = Frozen {
        workshop_id,
        frozen_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        copy: false,
    };
    if m.origin != ModOrigin::Workshop {
        write_marker(&m.dir_path, &frozen)?;
        return Ok(m.dir_path.clone());
    }
    let mods_dir = mods_dir.ok_or_else(|| text(TranslationKey::FreezeNoModsDir).to_string())?;
    let dest = free_dir(mods_dir, &m.name);
    frozen.copy = true;
    if let Err(e) = copy_dir(&m.dir_path, &dest).and_then(|()| write_marker(&dest, &frozen)) {
        // Half a mod is worse than none.
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    Ok(dest)
}

/// The Workshop's own folder of a frozen copy, when the subscription is still there.
fn original_dir(frozen: &Frozen, workshop_dir: Option<&Path>) -> Option<PathBuf> {
    let dir = workshop_dir?.join(frozen.workshop_id.as_deref()?);
    dir.join("About").join("About.xml").is_file().then_some(dir)
}

/// Unfreezing removes a copy only when the mod stays installed without it.
fn removes_copy(frozen: &Frozen, workshop_dir: Option<&Path>) -> bool {
    frozen.copy && original_dir(frozen, workshop_dir).is_some()
}

/// Unfreezes a mod: a copy made by freezing goes to the Recycle Bin and the Workshop
/// version is loaded again; without a subscription (or for a mod that was only marked)
/// the folder stays and just loses the mark. Returns whether the copy was removed.
pub fn unfreeze(dir: &Path, workshop_dir: Option<&Path>) -> Result<bool, String> {
    let frozen = read(dir).unwrap_or_default();
    if removes_copy(&frozen, workshop_dir) {
        crate::cleaner::move_mod_dir_to_trash(dir)?;
        return Ok(true);
    }
    let marker = dir.join(MARKER);
    std::fs::remove_file(&marker).map_err(|e| failed(&marker, e))?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_freeze_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mod_in(dir: &Path, origin: ModOrigin) -> ModInfo {
        std::fs::create_dir_all(dir.join("About")).unwrap();
        std::fs::create_dir_all(dir.join("Defs").join("Things")).unwrap();
        std::fs::write(dir.join("About").join("About.xml"), "<ModMetaData/>").unwrap();
        std::fs::write(dir.join("Defs").join("Things").join("a.xml"), "<Defs/>").unwrap();
        ModInfo {
            package_id: "author.mod".into(),
            name: "Mod: the <best>".into(),
            dir_path: dir.to_path_buf(),
            origin,
            ..Default::default()
        }
    }

    #[test]
    fn a_subscription_is_copied_into_mods() {
        let root = temp("copy");
        let (workshop, mods) = (root.join("workshop"), root.join("Mods"));
        std::fs::create_dir_all(&mods).unwrap();
        let m = mod_in(&workshop.join("123"), ModOrigin::Workshop);

        let dest = freeze(&m, Some("123".into()), Some(&mods)).unwrap();
        assert_eq!(dest, mods.join("Mod_ the _best_ (frozen)"));
        assert!(dest.join("Defs").join("Things").join("a.xml").is_file());
        let frozen = read(&dest).unwrap();
        assert!(frozen.copy && frozen.workshop_id.as_deref() == Some("123") && frozen.frozen_at > 0);
        // The subscription's own folder is not touched.
        assert!(!is_frozen(&m.dir_path));
        // A second freeze of the same name gets its own folder.
        assert_eq!(free_dir(&mods, &m.name), mods.join("Mod_ the _best_ (frozen 2)"));

        // While subscribed, unfreezing removes the copy; unsubscribed, the copy is all
        // there is, so it stays as a plain local mod.
        assert!(removes_copy(&frozen, Some(&workshop)));
        std::fs::remove_dir_all(&workshop).unwrap();
        assert!(!removes_copy(&frozen, Some(&workshop)));
        assert_eq!(unfreeze(&dest, Some(&workshop)), Ok(false));
        assert!(dest.is_dir() && !is_frozen(&dest));

        // Without the game's Mods folder there is nowhere to copy to.
        assert!(freeze(&m, None, None).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mod_in_mods_is_only_marked() {
        let root = temp("mark");
        let m = mod_in(&root.join("Mods").join("123"), ModOrigin::Local);

        assert_eq!(freeze(&m, Some("123".into()), None).unwrap(), m.dir_path);
        let frozen = read(&m.dir_path).unwrap();
        assert!(!frozen.copy);
        assert!(!removes_copy(&frozen, Some(&root.join("Mods"))));
        assert_eq!(unfreeze(&m.dir_path, None), Ok(false));
        assert!(m.dir_path.join("About").join("About.xml").is_file() && !is_frozen(&m.dir_path));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_names_for_windows() {
        assert_eq!(folder_name("  A/B: C?  "), "A_B_ C_");
        assert_eq!(folder_name("..."), "mod");
        assert_eq!(folder_name("Mod v1.0."), "Mod v1.0");
    }
}
