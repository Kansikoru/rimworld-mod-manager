use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::i18n::tr;

pub fn get_default_mods_config_path() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let appdata_path = Path::new(&appdata);
        // %APPDATA%\..\LocalLow\...
        if let Some(parent) = appdata_path.parent() {
            let config_path = parent
                .join("LocalLow")
                .join("Ludeon Studios")
                .join("RimWorld by Ludeon Studios")
                .join("Config")
                .join("ModsConfig.xml");
            if config_path.exists() {
                return Some(config_path);
            }
        }
    }

    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let config_path = Path::new(&userprofile)
            .join("AppData")
            .join("LocalLow")
            .join("Ludeon Studios")
            .join("RimWorld by Ludeon Studios")
            .join("Config")
            .join("ModsConfig.xml");
        if config_path.exists() {
            return Some(config_path);
        }
    }

    None
}

pub fn load_active_mods(config_path: &Path) -> Vec<String> {
    if !config_path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut active_mods = Vec::new();
    let mut in_active_mods = false;
    let mut current_tag = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = true;
                }
                current_tag = tag;
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(ref e)) => {
                if in_active_mods && current_tag.eq_ignore_ascii_case("li") {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    // RimWorld treats IDs case-insensitively: drop repeated entries.
                    if !text.is_empty() && !active_mods.iter().any(|m: &String| m.eq_ignore_ascii_case(&text)) {
                        active_mods.push(text);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    active_mods
}

pub fn save_active_mods(config_path: &Path, active_mods: &[String]) -> Result<(), String> {
    save_active_mods_with_backup(config_path, active_mods, &backups_dir())
}

/// Folder with copies of ModsConfig.xml made before each save.
pub fn backups_dir() -> PathBuf {
    crate::game_install::manager_data_dir().join("backups")
}

/// A copy of ModsConfig.xml made right before the manager saved a new order.
#[derive(Debug, Clone)]
pub struct ConfigBackup {
    pub path: PathBuf,
    /// Unix time of the save that replaced this list (from the file name).
    pub saved_at: u64,
    pub mods: Vec<String>,
}

/// Backups in `dir`, newest first.
pub fn list_backups(dir: &Path) -> Vec<ConfigBackup> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list: Vec<ConfigBackup> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let saved_at = backup_time(path.file_name()?.to_str()?)?;
            Some(ConfigBackup { mods: load_active_mods(&path), path, saved_at })
        })
        .collect();
    // Zero-padded names: sorting by name is sorting by time.
    list.sort_by(|a, b| b.path.cmp(&a.path));
    list
}

/// "ModsConfig.00000000001761480000-000.xml" -> 1761480000
fn backup_time(file_name: &str) -> Option<u64> {
    let rest = file_name.strip_prefix("ModsConfig.")?.strip_suffix(".xml")?;
    rest.split('-').next()?.parse().ok()
}

/// What changes when `current` is replaced by `other`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ListDiff {
    /// Mods in `other` that `current` doesn't have.
    pub added: usize,
    /// Mods in `current` that `other` doesn't have.
    pub removed: usize,
    /// The mods both lists have are in a different order.
    pub reordered: bool,
}

impl ListDiff {
    pub fn is_same(&self) -> bool {
        self.added == 0 && self.removed == 0 && !self.reordered
    }
}

pub fn diff_lists(current: &[String], other: &[String]) -> ListDiff {
    let cur: Vec<String> = current.iter().map(|s| s.to_lowercase()).collect();
    let oth: Vec<String> = other.iter().map(|s| s.to_lowercase()).collect();
    let cur_set: std::collections::HashSet<&String> = cur.iter().collect();
    let oth_set: std::collections::HashSet<&String> = oth.iter().collect();
    let common_cur: Vec<&String> = cur.iter().filter(|id| oth_set.contains(id)).collect();
    let common_oth: Vec<&String> = oth.iter().filter(|id| cur_set.contains(id)).collect();
    ListDiff {
        added: oth.iter().filter(|id| !cur_set.contains(id)).count(),
        removed: cur.iter().filter(|id| !oth_set.contains(id)).count(),
        reordered: common_cur != common_oth,
    }
}

/// Writes the active list. IDs are written in lower case, the way RimWorld itself writes
/// them: RimWorld 1.7 compares them case-sensitively, and a mixed-case ID ("Nals.FacialAnimation")
/// shows the mod in both the enabled and the disabled list in the game.
pub fn save_active_mods_with_backup(config_path: &Path, active_mods: &[String], backup_dir: &Path) -> Result<(), String> {
    if !config_path.exists() {
        return Err(tr!("Файл не найден: {}", "File not found: {}", config_path.display()));
    }

    backup_config(config_path, backup_dir, MAX_CONFIG_BACKUPS).map_err(|e| {
        tr!(
            "Не удалось сделать копию ModsConfig.xml в {}: {}",
            "Could not back up ModsConfig.xml to {}: {}",
            backup_dir.display(),
            e
        )
    })?;


    let mut unique: Vec<String> = Vec::with_capacity(active_mods.len());
    for m in active_mods {
        let id = m.trim().to_lowercase();
        if !id.is_empty() && !unique.contains(&id) {
            unique.push(id);
        }
    }
    let active_mods: &[String] = &unique;

    let content = std::fs::read_to_string(config_path)
        .map_err(|e| tr!("Ошибка чтения ModsConfig.xml: {}", "Could not read ModsConfig.xml: {}", e))?;

    let mut reader = Reader::from_str(&content);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    let mut in_active_mods = false;
    let mut found_active_mods = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = true;
                    found_active_mods = true;
                    // Write new activeMods block
                    let start = BytesStart::new("activeMods");
                    writer
                        .write_event(Event::Start(start.borrow()))
                        .map_err(|e| e.to_string())?;

                    for m in active_mods {
                        writer
                            .write_event(Event::Start(BytesStart::new("li")))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::Text(BytesText::new(m)))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::End(BytesEnd::new("li")))
                            .map_err(|e| e.to_string())?;
                    }
                } else if !in_active_mods {
                    writer
                        .write_event(Event::Start(e.clone()))
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = false;
                    writer
                        .write_event(Event::End(BytesEnd::new("activeMods")))
                        .map_err(|e| e.to_string())?;
                } else if !in_active_mods {
                    if tag.eq_ignore_ascii_case("modsconfigdata") && !found_active_mods {
                        // Inject activeMods before closing root tag
                        let start = BytesStart::new("activeMods");
                        writer.write_event(Event::Start(start.borrow())).unwrap();
                        for m in active_mods {
                            writer.write_event(Event::Start(BytesStart::new("li"))).unwrap();
                            writer.write_event(Event::Text(BytesText::new(m))).unwrap();
                            writer.write_event(Event::End(BytesEnd::new("li"))).unwrap();
                        }
                        writer.write_event(Event::End(BytesEnd::new("activeMods"))).unwrap();
                    }
                    writer
                        .write_event(Event::End(e.clone()))
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    found_active_mods = true;
                    let start = BytesStart::new("activeMods");
                    writer
                        .write_event(Event::Start(start.borrow()))
                        .map_err(|e| e.to_string())?;

                    for m in active_mods {
                        writer
                            .write_event(Event::Start(BytesStart::new("li")))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::Text(BytesText::new(m)))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::End(BytesEnd::new("li")))
                            .map_err(|e| e.to_string())?;
                    }
                    writer
                        .write_event(Event::End(BytesEnd::new("activeMods")))
                        .map_err(|e| e.to_string())?;
                } else if !in_active_mods {
                    writer.write_event(Event::Empty(e.clone())).map_err(|e| e.to_string())?;
                }
            }
            Ok(Event::Eof) => break,
            Ok(event) => {
                if !in_active_mods {
                    writer.write_event(event).map_err(|e| e.to_string())?;
                }
            }
            Err(e) => return Err(tr!("Ошибка в XML: {}", "XML error: {}", e)),
        }
        buf.clear();
    }

    let result_bytes = writer.into_inner().into_inner();
    let tmp_path = config_path.with_extension("xml.tmp");
    std::fs::write(&tmp_path, result_bytes)
        .map_err(|e| tr!("Ошибка записи ModsConfig.xml: {}", "Could not write ModsConfig.xml: {}", e))?;
    std::fs::rename(&tmp_path, config_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        tr!("Ошибка записи ModsConfig.xml: {}", "Could not write ModsConfig.xml: {}", e)
    })?;

    Ok(())
}

/// Copies are a few KB each, so a long history costs nothing.
const MAX_CONFIG_BACKUPS: usize = 30;

/// Copies ModsConfig.xml to `<backup_dir>/ModsConfig.<unix time>.xml` and keeps the newest `keep` copies.
pub fn backup_config(config_path: &Path, backup_dir: &Path, keep: usize) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(backup_dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Zero-padded names sort chronologically (file copies keep the source's modified time,
    // so timestamps on disk can't be used for ordering).
    let mut n = 0u32;
    let mut target = backup_dir.join(format!("ModsConfig.{secs:020}-{n:03}.xml"));
    while target.exists() {
        n += 1;
        target = backup_dir.join(format!("ModsConfig.{secs:020}-{n:03}.xml"));
    }
    std::fs::copy(config_path, &target)?;

    let mut backups: Vec<PathBuf> = std::fs::read_dir(backup_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("ModsConfig.") && n.ends_with(".xml"))
                .unwrap_or(false)
        })
        .collect();
    backups.sort();
    if backups.len() > keep {
        for old in &backups[..backups.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ModsConfigData>
  <version>1.7.4985 rev1269</version>
  <activeMods>
    <li>brrainz.harmony</li>
    <li>ludeon.rimworld</li>
    <li>Brrainz.Harmony</li>
    <li>someone.notinstalled</li>
  </activeMods>
  <knownExpansions>
    <li>ludeon.rimworld.royalty</li>
  </knownExpansions>
</ModsConfigData>
"#;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_cfg_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_dedupes_case_insensitively() {
        let dir = temp_dir("load");
        let cfg = dir.join("ModsConfig.xml");
        std::fs::write(&cfg, SAMPLE).unwrap();
        assert_eq!(
            load_active_mods(&cfg),
            vec!["brrainz.harmony", "ludeon.rimworld", "someone.notinstalled"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_roundtrip_keeps_other_sections() {
        let dir = temp_dir("save");
        let cfg = dir.join("ModsConfig.xml");
        std::fs::write(&cfg, SAMPLE).unwrap();
        let mods: Vec<String> =
            ["ludeon.rimworld", "brrainz.harmony", "BRRAINZ.HARMONY", "Nals.FacialAnimation", "someone.notinstalled"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        // Backups go to the test folder, never to the user's real backups.
        let backups = dir.join("backups");
        save_active_mods_with_backup(&cfg, &mods, &backups).unwrap();

        assert_eq!(
            load_active_mods(&cfg),
            vec!["ludeon.rimworld", "brrainz.harmony", "nals.facialanimation", "someone.notinstalled"],
            "written in lower case, like the game does"
        );
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("<knownExpansions>") && text.contains("1.7.4985 rev1269"));
        assert!(!cfg.with_extension("xml.tmp").exists());
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_are_rotated() {
        let dir = temp_dir("backup");
        let cfg = dir.join("ModsConfig.xml");
        std::fs::write(&cfg, SAMPLE).unwrap();
        let bdir = dir.join("backups");
        for _ in 0..5 {
            backup_config(&cfg, &bdir, 3).unwrap();
        }
        assert_eq!(std::fs::read_dir(&bdir).unwrap().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_are_listed_newest_first() {
        let dir = temp_dir("list");
        let one = "<ModsConfigData><activeMods><li>a.one</li></activeMods></ModsConfigData>";
        let two = "<ModsConfigData><activeMods><li>a.one</li><li>a.two</li></activeMods></ModsConfigData>";
        std::fs::write(dir.join("ModsConfig.00000000001700000000-000.xml"), one).unwrap();
        std::fs::write(dir.join("ModsConfig.00000000001800000000-000.xml"), two).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a backup").unwrap();

        let list = list_backups(&dir);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].saved_at, 1_800_000_000);
        assert_eq!(list[0].mods, ["a.one", "a.two"]);
        assert_eq!(list[1].saved_at, 1_700_000_000);
        assert!(list_backups(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_counts_added_removed_and_order() {
        let v = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let current = v(&["core", "a", "b", "c"]);
        assert!(diff_lists(&current, &v(&["Core", "A", "B", "C"])).is_same(), "case doesn't matter");
        assert_eq!(
            diff_lists(&current, &v(&["core", "a", "b", "d", "e"])),
            ListDiff { added: 2, removed: 1, reordered: false }
        );
        assert_eq!(
            diff_lists(&current, &v(&["core", "b", "a", "c"])),
            ListDiff { added: 0, removed: 0, reordered: true }
        );
    }
}
