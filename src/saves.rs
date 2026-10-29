//! Saves (F7): the mod list a save was made with. A save starts with the same `<meta>`
//! block as a mod list file (game version, mod IDs, Workshop IDs, names), so only the
//! start of the file is read; saves are 15-30 MB. Old 1.5 saves may be gzip-compressed.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The `<meta>` block ends within the first few KB; stop looking after this much.
const META_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveInfo {
    /// File name without `.rws`.
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// When the file was last written (unix seconds).
    pub modified: Option<u64>,
    pub game_version: Option<String>,
    /// Package IDs in load order.
    pub ids: Vec<String>,
    pub names: Vec<String>,
    /// Workshop item IDs by lowercase package ID.
    pub steam_ids: HashMap<String, String>,
}

impl SaveInfo {
    pub fn steam_id(&self, id: &str) -> Option<&str> {
        self.steam_ids.get(&id.to_lowercase()).map(String::as_str)
    }

    /// The mod's name as the save remembers it.
    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.ids
            .iter()
            .position(|i| i.eq_ignore_ascii_case(id))
            .and_then(|i| self.names.get(i))
            .map(String::as_str)
    }
}

/// `...\RimWorld by Ludeon Studios\Config\ModsConfig.xml` -> `...\RimWorld by Ludeon Studios\Saves`.
pub fn saves_dir(mods_config: &Path) -> Option<PathBuf> {
    Some(mods_config.parent()?.parent()?.join("Saves"))
}

pub fn path_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.rws"))
}

/// Destination folder for save backups: `<saves_dir>/Backups` (F12).
pub fn save_backups_dir(saves_dir: &Path) -> PathBuf {
    saves_dir.join("Backups")
}

/// Backs up the most recently modified `.rws` save file to `<saves_dir>/Backups` (F12).
///
/// Keeps at most `keep` backups per save stem (e.g. `MyColony.00000000001761480000-000.rws`).
/// If `keep` is 0, no backup is made.
pub fn backup_latest_save(saves_dir: &Path, keep: usize) -> std::io::Result<Option<PathBuf>> {
    if keep == 0 {
        return Ok(None);
    }
    let Ok(entries) = std::fs::read_dir(saves_dir) else {
        return Ok(None);
    };

    // Find the newest .rws file in saves_dir (ignoring subdirectories like Backups)
    let mut newest_save: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("rws"))
        {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if newest_save.as_ref().is_none_or(|(_, best)| mtime > *best) {
                        newest_save = Some((path, mtime));
                    }
                }
            }
        }
    }

    let Some((latest_path, _)) = newest_save else {
        return Ok(None);
    };

    let backup_dir = save_backups_dir(saves_dir);
    std::fs::create_dir_all(&backup_dir)?;

    let stem = latest_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Save");

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut n = 0u32;
    let mut target = backup_dir.join(format!("{stem}.{secs:020}-{n:03}.rws"));
    while target.exists() {
        n += 1;
        target = backup_dir.join(format!("{stem}.{secs:020}-{n:03}.rws"));
    }

    std::fs::copy(&latest_path, &target)?;

    // Rotate backups for this save stem
    let stem_prefix = format!("{stem}.");
    let mut backups: Vec<PathBuf> = std::fs::read_dir(&backup_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&stem_prefix) && n.ends_with(".rws"))
                .unwrap_or(false)
        })
        .collect();

    backups.sort();
    if backups.len() > keep {
        for old in &backups[..backups.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }

    Ok(Some(target))
}

/// Lists all save backup files in `<saves_dir>/Backups`, newest first.
/// If `stem` is provided, filters to backups of that specific save stem.
pub fn list_save_backups(saves_dir: &Path, stem: Option<&str>) -> Vec<PathBuf> {
    let backup_dir = save_backups_dir(saves_dir);
    let Ok(entries) = std::fs::read_dir(&backup_dir) else {
        return Vec::new();
    };
    let prefix = stem.map(|s| format!("{s}."));
    let mut list: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            if !name.ends_with(".rws") {
                return false;
            }
            if let Some(ref pfx) = prefix {
                name.starts_with(pfx)
            } else {
                true
            }
        })
        .collect();
    list.sort();
    list.reverse();
    list
}

/// Parses the original save name from a backup filename like `MyColony.00000000001761480000-000.rws`.
pub fn parse_backup_stem(backup_name: &str) -> Option<&str> {
    let rest = backup_name.strip_suffix(".rws")?;
    let dot_idx = rest.rfind('.')?;
    let timestamp_part = &rest[dot_idx + 1..];
    if timestamp_part.len() == 24 && timestamp_part.contains('-') {
        Some(&rest[..dot_idx])
    } else {
        Some(rest)
    }
}

/// Restores a backup copy into `saves_dir` under its original save name.
pub fn restore_save_backup(backup_path: &Path, saves_dir: &Path) -> std::io::Result<PathBuf> {
    let file_name = backup_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("Invalid backup path"))?;
    let original_stem = parse_backup_stem(file_name).unwrap_or("RestoredSave");
    let target = saves_dir.join(format!("{original_stem}.rws"));
    std::fs::copy(backup_path, &target)?;
    Ok(target)
}

/// All saves in the folder, newest first (files without a readable header are skipped).
pub fn list_saves(dir: &Path) -> Vec<SaveInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut saves: Vec<SaveInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("rws"))
        })
        .filter_map(|p| read_save(&p))
        .collect();
    saves.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    saves
}

/// Reads the header of one save.
pub fn read_save(path: &Path) -> Option<SaveInfo> {
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let head = read_head(file)?;
    let mut info = parse_meta(&head)?;
    info.name = path.file_stem()?.to_string_lossy().to_string();
    info.path = path.to_path_buf();
    info.size = meta.len();
    info.modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    Some(info)
}

/// The start of the file up to and including `</meta>` (unpacked if gzip).
fn read_head(mut file: std::fs::File) -> Option<String> {
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).ok()?;
    let rest: Box<dyn Read> = if magic == [0x1f, 0x8b] {
        let chained = std::io::Cursor::new(magic).chain(file);
        Box::new(flate2::read::GzDecoder::new(chained))
    } else {
        Box::new(std::io::Cursor::new(magic).chain(file))
    };
    read_until_meta_end(rest)
}

fn read_until_meta_end(mut reader: impl Read) -> Option<String> {
    const END: &[u8] = b"</meta>";
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        // Search from a little before the new data, in case the tag was split.
        let from = buf.len().saturating_sub(END.len());
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf[from..].windows(END.len()).position(|w| w == END) {
            buf.truncate(from + pos + END.len());
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        if buf.len() > META_LIMIT {
            return None;
        }
    }
}

/// Parses `<meta>` (the text may stop right after it). Name and path are left empty.
pub fn parse_meta(text: &str) -> Option<SaveInfo> {
    let mut reader = Reader::from_str(text.trim_start_matches('\u{feff}'));
    reader.config_mut().trim_text(true);
    let mut stack: Vec<String> = Vec::new();
    let (mut ids, mut names, mut steam) = (Vec::new(), Vec::new(), Vec::new());
    let mut game_version = None;
    let mut saw_meta = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if tag == "meta" {
                    saw_meta = true;
                }
                stack.push(tag);
            }
            Ok(Event::End(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                stack.pop();
                if tag == "meta" {
                    break;
                }
            }
            Ok(Event::Text(t)) => {
                let value = t.unescape().map(|v| v.trim().to_string()).unwrap_or_default();
                if value.is_empty() {
                    continue;
                }
                let path: Vec<&str> = stack.iter().map(String::as_str).collect();
                match path.as_slice() {
                    [.., "meta", "gameversion"] => game_version = Some(value),
                    [.., "meta", "modids", "li"] => ids.push(value),
                    [.., "meta", "modnames", "li"] => names.push(value),
                    [.., "meta", "modsteamids", "li"] => steam.push(value),
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }
    if !saw_meta {
        return None;
    }
    let steam_ids = crate::modlists::steam_id_map(&ids, &steam);
    Some(SaveInfo {
        name: String::new(),
        path: PathBuf::new(),
        size: 0,
        modified: None,
        game_version,
        ids,
        names,
        steam_ids,
    })
}

/// How the mods of a save compare with a load order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SaveCompare {
    /// In the save and in the load order.
    pub enabled: Vec<String>,
    /// In the save, installed, but not in the load order.
    pub disabled: Vec<String>,
    /// In the save, not installed.
    pub missing: Vec<String>,
    /// In the load order, not in the save.
    pub extra: Vec<String>,
    /// Same mods, different order.
    pub reordered: bool,
}

impl SaveCompare {
    /// Loading the save with this load order needs no changes.
    pub fn matches(&self) -> bool {
        self.disabled.is_empty() && self.missing.is_empty() && self.extra.is_empty() && !self.reordered
    }
}

/// Compares the save's mods with `active` (IDs compared without case; `installed` says
/// whether a mod is on disk).
pub fn compare(save_ids: &[String], active: &[String], installed: impl Fn(&str) -> bool) -> SaveCompare {
    use std::collections::HashSet;
    let active_lower: Vec<String> = active.iter().map(|s| s.to_lowercase()).collect();
    let save_lower: Vec<String> = save_ids.iter().map(|s| s.to_lowercase()).collect();
    let active_set: HashSet<&String> = active_lower.iter().collect();
    let save_set: HashSet<&String> = save_lower.iter().collect();
    let mut out = SaveCompare::default();
    for (id, lower) in save_ids.iter().zip(&save_lower) {
        if active_set.contains(lower) {
            out.enabled.push(id.clone());
        } else if installed(id) {
            out.disabled.push(id.clone());
        } else {
            out.missing.push(id.clone());
        }
    }
    for (id, lower) in active.iter().zip(&active_lower) {
        if !save_set.contains(lower) {
            out.extra.push(id.clone());
        }
    }
    // Order of the mods both lists have.
    let common_in_save: Vec<&String> = save_lower.iter().filter(|id| active_set.contains(id)).collect();
    let common_in_active: Vec<&String> = active_lower.iter().filter(|id| save_set.contains(id)).collect();
    out.reordered = common_in_save != common_in_active;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<savegame>\n\t<meta>\n\t\t<gameVersion>1.6.4871 rev591</gameVersion>\n\t\t<modIds>\n\t\t\t<li>brrainz.harmony</li>\n\t\t\t<li>ludeon.rimworld</li>\n\t\t\t<li>brrainz.achtung</li>\n\t\t</modIds>\n\t\t<modSteamIds>\n\t\t\t<li>0</li>\n\t\t\t<li>0</li>\n\t\t\t<li>730936602</li>\n\t\t</modSteamIds>\n\t\t<modNames>\n\t\t\t<li>Harmony</li>\n\t\t\t<li>Core</li>\n\t\t\t<li>Achtung!</li>\n\t\t</modNames>\n\t</meta>\n\t<game>\n\t\t<currentMapIndex>0</currentMapIndex>";

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_saves_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_the_header_of_a_save() {
        let info = parse_meta(HEAD).unwrap();
        assert_eq!(info.game_version.as_deref(), Some("1.6.4871 rev591"));
        assert_eq!(info.ids, ["brrainz.harmony", "ludeon.rimworld", "brrainz.achtung"]);
        assert_eq!(info.name_of("Brrainz.Achtung"), Some("Achtung!"));
        assert_eq!(info.steam_id("brrainz.achtung"), Some("730936602"));
        assert_eq!(info.steam_id("brrainz.harmony"), None);
        assert!(parse_meta("<savegame><game/></savegame>").is_none());
    }

    #[test]
    fn reads_plain_and_gzip_files_without_loading_everything() {
        use std::io::Write;
        let dir = temp_dir("files");
        // A big body after the header: only the start is needed.
        let body = "<thing>x</thing>".repeat(200_000);
        let plain = format!("{HEAD}{body}</game></savegame>");
        std::fs::write(dir.join("Colony.rws"), &plain).unwrap();

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(plain.as_bytes()).unwrap();
        std::fs::write(dir.join("Old 1.5.rws"), gz.finish().unwrap()).unwrap();
        std::fs::write(dir.join("broken.rws"), "not a save").unwrap();
        std::fs::write(dir.join("notes.txt"), HEAD).unwrap();

        let saves = list_saves(&dir);
        let mut names: Vec<&str> = saves.iter().map(|s| s.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Colony", "Old 1.5"]);
        for save in &saves {
            assert_eq!(save.ids.len(), 3);
            assert!(save.size > 0 && save.modified.is_some());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn meta_end_split_between_chunks() {
        // The closing tag straddles the 64 KB read boundary.
        let pad = " ".repeat(64 * 1024 - HEAD.find("</meta>").unwrap() - 3);
        let text = HEAD.replacen("<savegame>", &format!("<savegame>{pad}"), 1);
        let head = read_until_meta_end(std::io::Cursor::new(text.into_bytes())).unwrap();
        assert!(head.ends_with("</meta>"));
        assert_eq!(parse_meta(&head).unwrap().ids.len(), 3);
    }

    #[test]
    fn compares_save_with_load_order() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let save = ids(&["brrainz.harmony", "ludeon.rimworld", "a.disabled", "b.missing"]);
        let active = ids(&["Ludeon.RimWorld", "brrainz.harmony", "c.extra"]);
        let installed = |id: &str| id != "b.missing";
        let cmp = compare(&save, &active, installed);
        assert_eq!(cmp.enabled, ["brrainz.harmony", "ludeon.rimworld"]);
        assert_eq!(cmp.disabled, ["a.disabled"]);
        assert_eq!(cmp.missing, ["b.missing"]);
        assert_eq!(cmp.extra, ["c.extra"]);
        assert!(cmp.reordered, "Harmony goes before Core in the save");
        assert!(!cmp.matches());

        let same = compare(&ids(&["a", "b"]), &ids(&["A", "b"]), |_| true);
        assert!(same.matches());
    }

    #[test]
    fn saves_folder_is_next_to_config() {
        let cfg =
            Path::new(r"C:\Users\u\AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Config\ModsConfig.xml");
        assert_eq!(
            saves_dir(cfg).unwrap(),
            PathBuf::from(r"C:\Users\u\AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Saves")
        );
    }

    #[test]
    fn save_backups_rotation_and_restore() {
        let dir = temp_dir("save_backups");
        let save_a = dir.join("ColonyA.rws");
        let save_b = dir.join("ColonyB.rws");

        std::fs::write(&save_a, "data_a").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&save_b, "data_b").unwrap();

        // Newest is save_b
        let b1 = backup_latest_save(&dir, 2).unwrap().unwrap();
        assert!(b1.exists());
        assert!(b1.file_name().unwrap().to_str().unwrap().starts_with("ColonyB."));

        // Multiple backups to trigger rotation
        let _b2 = backup_latest_save(&dir, 2).unwrap().unwrap();
        let b3 = backup_latest_save(&dir, 2).unwrap().unwrap();
        let backups_b = list_save_backups(&dir, Some("ColonyB"));
        assert_eq!(backups_b.len(), 2, "rotation keeps at most 2 backups for ColonyB");

        // Touch save_a to make it newer
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&save_a, "data_a_updated").unwrap();
        let ba = backup_latest_save(&dir, 2).unwrap().unwrap();
        assert!(ba.file_name().unwrap().to_str().unwrap().starts_with("ColonyA."));

        let backups_all = list_save_backups(&dir, None);
        assert_eq!(backups_all.len(), 3, "2 from ColonyB + 1 from ColonyA");

        // Restore ColonyB from backup
        std::fs::remove_file(&save_b).unwrap();
        assert!(!save_b.exists());
        let restored = restore_save_backup(&b3, &dir).unwrap();
        assert_eq!(restored, save_b);
        assert_eq!(std::fs::read_to_string(&restored).unwrap(), "data_b");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads the user's real saves (run by hand: `cargo test live_saves -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_saves() {
        let dir = PathBuf::from(std::env::var("USERPROFILE").unwrap())
            .join(r"AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Saves");
        let started = std::time::Instant::now();
        let saves = list_saves(&dir);
        for s in &saves {
            println!(
                "{} | {:?} | {} mods | {} MB",
                s.name,
                s.game_version,
                s.ids.len(),
                s.size / 1_048_576
            );
        }
        println!("{} saves in {:?}", saves.len(), started.elapsed());
        assert!(!saves.is_empty());
    }
}
