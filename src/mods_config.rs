use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::io::Cursor;
use std::path::{Path, PathBuf};

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
    if !config_path.exists() {
        return Err(format!("Файл конфига не найден: {:?}", config_path));
    }

    let backup_dir = crate::game_install::manager_data_dir().join("backups");
    backup_config(config_path, &backup_dir, MAX_CONFIG_BACKUPS)
        .map_err(|e| format!("Не удалось сделать бэкап ModsConfig.xml в {:?}: {}", backup_dir, e))?;

    let mut unique: Vec<String> = Vec::with_capacity(active_mods.len());
    for m in active_mods {
        if !unique.iter().any(|u| u.eq_ignore_ascii_case(m)) {
            unique.push(m.clone());
        }
    }
    let active_mods: &[String] = &unique;

    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Ошибка чтения ModsConfig.xml: {}", e))?;

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
            Err(e) => return Err(format!("Ошибка парсинга XML: {}", e)),
        }
        buf.clear();
    }

    let result_bytes = writer.into_inner().into_inner();
    let tmp_path = config_path.with_extension("xml.tmp");
    std::fs::write(&tmp_path, result_bytes)
        .map_err(|e| format!("Ошибка записи ModsConfig.xml: {}", e))?;
    std::fs::rename(&tmp_path, config_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("Ошибка записи ModsConfig.xml: {}", e)
    })?;

    Ok(())
}

const MAX_CONFIG_BACKUPS: usize = 10;

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
        let mods: Vec<String> = ["ludeon.rimworld", "brrainz.harmony", "BRRAINZ.HARMONY", "someone.notinstalled"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        save_active_mods(&cfg, &mods).unwrap();

        assert_eq!(
            load_active_mods(&cfg),
            vec!["ludeon.rimworld", "brrainz.harmony", "someone.notinstalled"]
        );
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("<knownExpansions>") && text.contains("1.7.4985 rev1269"));
        assert!(!cfg.with_extension("xml.tmp").exists());
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
}
