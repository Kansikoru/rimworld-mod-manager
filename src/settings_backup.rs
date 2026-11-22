//! Backup and transfer of the program's own settings in one file (F9 of the plan, item 22):
//! a zip of what the player set up in `%LOCALAPPDATA%\RimWorldModManager`. Caches,
//! logs, saved mod versions and ModsConfig.xml backups stay out: they are rebuilt or
//! belong to this computer. Paths in `settings.json` that don't exist on the other
//! computer are ignored at start, so the file moves between computers as it is.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Files the player made (settings, own rules, colours, tags, categories, hidden copies...).
pub const FILES: [&str; 9] = [
    "settings.json",
    "ui_prefs.json",
    "user_order_rules.json",
    "mod_colors.json",
    "modlist_tags.json",
    "category_overrides.json",
    "ignored_duplicates.json",
    "optimizer_ignore.txt",
    "community_rules.json",
];

/// Folders with the modpacks' pictures and kept mod settings.
pub const DIRS: [&str; 2] = ["modlist_images", "modlist_settings"];

/// The copy of the current settings taken before an import replaces them.
pub const BEFORE_IMPORT: &str = "settings_before_import.zip";

/// A file bigger than this is not ours (the biggest, the rules database, is a few MB).
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// Paths of the zip entries (with `/`), relative to `data_dir`, that exist now.
fn collect(data_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = FILES
        .iter()
        .filter(|f| data_dir.join(f).is_file())
        .map(|f| f.to_string())
        .collect();
    for dir in DIRS {
        for entry in walkdir::WalkDir::new(data_dir.join(dir)).into_iter().flatten() {
            if entry.file_type().is_file() {
                if let Ok(rel) = entry.path().strip_prefix(data_dir) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out
}

/// Writes the settings into `target` (a zip); returns how many files went in.
pub fn export(data_dir: &Path, target: &Path) -> Result<usize, String> {
    let entries = collect(data_dir);
    if entries.is_empty() {
        return Err(crate::i18n::text(crate::i18n::TranslationKey::SettingsBackupNothing).to_string());
    }
    let tmp = target.with_extension("zip.tmp");
    let write = || -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for name in &entries {
            let bytes = std::fs::read(data_dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
            zip.start_file(name.as_str(), opts).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, target).map_err(|e| e.to_string())?;
    Ok(entries.len())
}

/// A zip entry this program may write: one of `FILES`, or a file inside one of `DIRS`
/// (no `..`, no absolute paths).
fn allowed(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let parts: Vec<&str> = path.iter().filter_map(|p| p.to_str()).collect();
    if parts.iter().any(|p| *p == ".." || p.is_empty() || p.contains(':')) || path.is_absolute() {
        return None;
    }
    match parts.as_slice() {
        [file] if FILES.contains(file) => Some(PathBuf::from(file)),
        [dir, rest @ ..] if DIRS.contains(dir) && !rest.is_empty() => Some(parts.iter().collect()),
        _ => None,
    }
}

/// Puts the settings from `source` into `data_dir`; the current ones are kept first in
/// `BEFORE_IMPORT`. A zip with none of our files is refused and nothing changes.
/// Returns how many files were taken.
pub fn import(source: &Path, data_dir: &Path) -> Result<usize, String> {
    let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() || entry.size() > MAX_ENTRY_BYTES {
            continue;
        }
        let Some(rel) = allowed(entry.name()) else { continue };
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_ENTRY_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        files.push((rel, bytes));
    }
    if files.is_empty() {
        return Err(crate::i18n::text(crate::i18n::TranslationKey::SettingsBackupNotOurs).to_string());
    }
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    // Nothing to keep yet is fine; a failed copy of existing settings stops the import.
    match export(data_dir, &data_dir.join(BEFORE_IMPORT)) {
        Ok(_) => {}
        Err(_) if collect(data_dir).is_empty() => {}
        Err(e) => return Err(e),
    }
    for (rel, bytes) in &files {
        let dest = data_dir.join(rel);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&dest, bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_settings_backup_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn exported_and_imported_on_another_computer() {
        let root = temp("roundtrip");
        let (here, there) = (root.join("here"), root.join("there"));
        std::fs::create_dir_all(here.join("modlist_images")).unwrap();
        std::fs::write(here.join("settings.json"), r#"{"language":"English"}"#).unwrap();
        std::fs::write(here.join("mod_colors.json"), r#"{"a.mod":"red"}"#).unwrap();
        std::fs::write(here.join("modlist_images").join("Run.png"), [1u8, 2, 3]).unwrap();
        // Caches stay out.
        std::fs::write(here.join("category_cache.json"), "{}").unwrap();

        let zip = root.join("backup.zip");
        assert_eq!(export(&here, &zip).unwrap(), 3);

        std::fs::create_dir_all(&there).unwrap();
        std::fs::write(there.join("settings.json"), r#"{"language":"Russian"}"#).unwrap();
        assert_eq!(import(&zip, &there).unwrap(), 3);
        assert_eq!(
            std::fs::read_to_string(there.join("settings.json")).unwrap(),
            r#"{"language":"English"}"#
        );
        assert_eq!(
            std::fs::read(there.join("modlist_images").join("Run.png")).unwrap(),
            [1, 2, 3]
        );
        assert!(!there.join("category_cache.json").exists());
        // The settings it replaced are kept.
        assert!(there.join(BEFORE_IMPORT).is_file());

        assert!(
            export(&root.join("empty"), &root.join("none.zip")).is_err(),
            "nothing to save"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn foreign_entries_are_refused() {
        assert_eq!(allowed("settings.json"), Some(PathBuf::from("settings.json")));
        assert_eq!(
            allowed("modlist_images/Run.png"),
            Some(PathBuf::from("modlist_images").join("Run.png"))
        );
        for bad in [
            "../settings.json",
            "modlist_images/../../x",
            "C:/Windows/x",
            "other.json",
            "modlist_images",
            "/settings.json",
        ] {
            assert_eq!(allowed(bad), None, "{bad}");
        }

        let root = temp("foreign");
        let zip_path = root.join("foreign.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zip.start_file("../evil.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let data = root.join("data");
        assert!(import(&zip_path, &data).is_err());
        assert!(!root.join("evil.txt").exists() && !data.exists(), "nothing written");
        let _ = std::fs::remove_dir_all(&root);
    }
}
