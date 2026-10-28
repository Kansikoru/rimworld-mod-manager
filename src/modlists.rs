//! Mod sets (F4): saved load orders in the game's own `ModLists\*.rml` files, so sets made
//! here show up in the game's "Mod lists" menu and the game's lists show up here.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::path::{Path, PathBuf};

/// One saved set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModList {
    /// File name without `.rml`.
    pub name: String,
    pub path: PathBuf,
    pub game_version: Option<String>,
    /// Package IDs in load order.
    pub ids: Vec<String>,
    /// Mod names in the same order (may be shorter in hand-made files).
    pub names: Vec<String>,
    /// When the file was last written (unix seconds).
    pub modified: Option<u64>,
    /// Workshop item IDs by lowercase package ID (from `meta`; local mods, Core and DLC have none).
    pub steam_ids: std::collections::HashMap<String, String>,
}

impl ModList {
    /// The mod's Workshop item ID, if the set knows one.
    pub fn steam_id(&self, id: &str) -> Option<&str> {
        self.steam_ids.get(&id.to_lowercase()).map(String::as_str)
    }
}

/// A mod to write into a set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModListEntry {
    pub id: String,
    pub name: String,
    /// Workshop item ID; `None` for local mods, Core and DLC (the game writes 0).
    pub steam_id: Option<String>,
}

/// Why a set name can't be used as a file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    BadCharacters,
    TooLong,
}

/// `...\RimWorld by Ludeon Studios\Config\ModsConfig.xml` -> `...\RimWorld by Ludeon Studios\ModLists`.
pub fn modlists_dir(mods_config: &Path) -> Option<PathBuf> {
    Some(mods_config.parent()?.parent()?.join("ModLists"))
}

/// The trimmed name, or why it can't be a file name on Windows.
pub fn check_name(name: &str) -> Result<String, NameProblem> {
    let name = name.trim().trim_end_matches('.').trim();
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if name.chars().count() > 80 {
        return Err(NameProblem::TooLong);
    }
    if name
        .chars()
        .any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
    {
        return Err(NameProblem::BadCharacters);
    }
    Ok(name.to_string())
}

pub fn path_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.rml"))
}

/// All sets in the folder, sorted by name (unreadable files are skipped).
pub fn list_modlists(dir: &Path) -> Vec<ModList> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut lists: Vec<ModList> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("rml"))
        })
        .filter_map(|p| read_rml(&p))
        .collect();
    lists.sort_by_key(|l| l.name.to_lowercase());
    lists
}

/// Reads a `.rml` file: the order from `modList/ids`, or from `meta/modIds` in older files.
pub fn read_rml(path: &Path) -> Option<ModList> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = path.file_stem()?.to_string_lossy().to_string();
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    parse_rml_str(&text, name, path.to_path_buf(), modified)
}

pub fn parse_rml_str(text: &str, name: String, path: PathBuf, modified: Option<u64>) -> Option<ModList> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<String> = Vec::new();
    let (mut ids, mut names, mut meta_ids, mut meta_names) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut meta_steam: Vec<String> = Vec::new();
    let mut game_version = None;
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if stack.is_empty() && tag == "savedmodlist" {
                    saw_root = true;
                }
                stack.push(tag);
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Text(t)) => {
                let value = t.unescape().map(|v| v.trim().to_string()).unwrap_or_default();
                if value.is_empty() {
                    continue;
                }
                let path: Vec<&str> = stack.iter().map(String::as_str).collect();
                match path.as_slice() {
                    [.., "modlist", "ids", "li"] => ids.push(value),
                    [.., "modlist", "names", "li"] => names.push(value),
                    [.., "meta", "modids", "li"] => meta_ids.push(value),
                    [.., "meta", "modnames", "li"] => meta_names.push(value),
                    [.., "meta", "modsteamids", "li"] => meta_steam.push(value),
                    [.., "meta", "gameversion"] => game_version = Some(value),
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }
    if !saw_root {
        return None;
    }
    let steam_ids = steam_id_map(&meta_ids, &meta_steam);
    if ids.is_empty() {
        ids = meta_ids;
        names = meta_names;
    }
    Some(ModList {
        name,
        path,
        game_version,
        ids,
        names,
        modified,
        steam_ids,
    })
}

/// A set from a file someone shared: a `.rml`, or a `ModsConfig.xml` (its active mods,
/// without Workshop IDs). The name comes from the file name.
pub fn read_shared(path: &Path) -> Option<ModList> {
    if let Some(list) = read_rml(path) {
        return Some(list);
    }
    let text = std::fs::read_to_string(path).ok()?;
    if !text.contains("<ModsConfigData") {
        return None;
    }
    let ids = crate::mods_config::load_active_mods(path);
    if ids.is_empty() {
        return None;
    }
    let game_version = text
        .split_once("<version>")
        .and_then(|(_, rest)| rest.split_once("</version>"))
        .map(|(v, _)| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Some(ModList {
        name: path.file_stem()?.to_string_lossy().to_string(),
        path: path.to_path_buf(),
        game_version,
        ids,
        names: Vec::new(),
        modified: None,
        steam_ids: Default::default(),
    })
}

/// `name` if no set has it yet, else "name (2)", "name (3)", ...
pub fn unique_name(dir: &Path, name: &str) -> String {
    if !path_for(dir, name).exists() {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|candidate| !path_for(dir, candidate).exists())
        .unwrap_or_else(|| name.to_string())
}

/// The set as plain text for a chat or forum: one mod per line, with its Workshop link
/// when it has one.
pub fn share_text(title: &str, mods: &[(String, Option<String>)]) -> String {
    let mut out = format!("{title} ({})\n", mods.len());
    for (i, (name, steam_id)) in mods.iter().enumerate() {
        match steam_id {
            Some(id) => out.push_str(&format!(
                "{}. {name} — https://steamcommunity.com/sharedfiles/filedetails/?id={id}\n",
                i + 1
            )),
            None => out.push_str(&format!("{}. {name}\n", i + 1)),
        }
    }
    out
}

/// `meta`'s ID and Steam ID lists go side by side; "0" means "not from the Workshop".
/// Returns Workshop IDs by lowercase package ID.
pub fn steam_id_map(ids: &[String], steam_ids: &[String]) -> std::collections::HashMap<String, String> {
    ids.iter()
        .zip(steam_ids)
        .filter(|(_, steam)| is_workshop_id(steam))
        .map(|(id, steam)| (id.to_lowercase(), steam.clone()))
        .collect()
}

/// A Workshop item ID: digits, not 0.
pub fn is_workshop_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.bytes().any(|b| b != b'0')
}

/// A free name for a copy: "Name (copy)", "Name (copy 2)", ... `copy_word` is the
/// translated word for "copy".
pub fn copy_name(dir: &Path, name: &str, copy_word: &str) -> String {
    let first = format!("{name} ({copy_word})");
    if !path_for(dir, &first).exists() {
        return first;
    }
    (2..)
        .map(|n| format!("{name} ({copy_word} {n})"))
        .find(|candidate| !path_for(dir, candidate).exists())
        .unwrap_or(first)
}

fn escape(s: &str) -> String {
    quick_xml::escape::escape(s).to_string()
}

/// The file as the game writes it: `<meta>` with the game version and the mods, then
/// `<modList>` with the order the game loads.
pub fn rml_text(game_version: &str, entries: &[ModListEntry]) -> String {
    let li = |values: &mut dyn Iterator<Item = String>, indent: &str| -> String {
        values.map(|v| format!("{indent}<li>{}</li>\n", escape(&v))).collect()
    };
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<savedModList>\n\t<meta>\n");
    out.push_str(&format!("\t\t<gameVersion>{}</gameVersion>\n", escape(game_version)));
    out.push_str("\t\t<modIds>\n");
    out.push_str(&li(&mut entries.iter().map(|e| e.id.clone()), "\t\t\t"));
    out.push_str("\t\t</modIds>\n\t\t<modSteamIds>\n");
    out.push_str(&li(
        &mut entries.iter().map(|e| e.steam_id.clone().unwrap_or_else(|| "0".into())),
        "\t\t\t",
    ));
    out.push_str("\t\t</modSteamIds>\n\t\t<modNames>\n");
    out.push_str(&li(&mut entries.iter().map(|e| e.name.clone()), "\t\t\t"));
    out.push_str("\t\t</modNames>\n\t</meta>\n\t<modList>\n\t\t<ids>\n");
    out.push_str(&li(&mut entries.iter().map(|e| e.id.clone()), "\t\t\t"));
    out.push_str("\t\t</ids>\n\t\t<names>\n");
    out.push_str(&li(&mut entries.iter().map(|e| e.name.clone()), "\t\t\t"));
    out.push_str("\t\t</names>\n\t</modList>\n</savedModList>");
    out
}

/// Writes a set through a temporary file, so a crash never leaves half a file.
pub fn write_rml(path: &Path, game_version: &str, entries: &[ModListEntry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("rml.tmp");
    std::fs::write(&tmp, rml_text(game_version, entries))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A safe `Mod_*.xml` settings file name in RimWorld's `Config` folder.
pub fn is_mod_config_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("mod_")
        && lower.ends_with(".xml")
        && lower != "modsconfig.xml"
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name.chars().all(|c| !c.is_control())
}

/// F5: a ZIP pack with a `.rml` modlist and `Config/Mod_*.xml` mod settings files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipPack {
    pub list: ModList,
    /// `(file_name, bytes)` for `Config/Mod_*.xml`.
    pub configs: Vec<(String, Vec<u8>)>,
}

/// F5: writes a ZIP pack containing `<name>.rml` and `Config/Mod_*.xml` from `config_dir`.
/// Returns how many `Mod_*.xml` files were packed.
pub fn write_zip_pack(
    zip_path: &Path,
    rml_name: &str,
    rml_content: &str,
    config_dir: Option<&Path>,
) -> Result<usize, String> {
    use std::io::Write;
    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = zip_path.with_extension("zip.tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let safe_rml = check_name(rml_name).unwrap_or_else(|_| "Modpack".to_string());
    zip.start_file(format!("{safe_rml}.rml"), opts)
        .map_err(|e| e.to_string())?;
    zip.write_all(rml_content.as_bytes()).map_err(|e| e.to_string())?;

    let mut cfg_count = 0usize;
    if let Some(dir) = config_dir {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut files: Vec<(String, PathBuf)> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| {
                    let fname = e.file_name().to_string_lossy().to_string();
                    is_mod_config_name(&fname).then_some((fname, e.path()))
                })
                .collect();
            files.sort_by(|a, b| a.0.cmp(&b.0));
            for (fname, path) in files {
                if let Ok(bytes) = std::fs::read(&path) {
                    zip.start_file(format!("Config/{fname}"), opts)
                        .map_err(|e| e.to_string())?;
                    zip.write_all(&bytes).map_err(|e| e.to_string())?;
                    cfg_count += 1;
                }
            }
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, zip_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(cfg_count)
}

/// F5: reads a ZIP pack (`.rml` + `Config/Mod_*.xml`).
pub fn read_zip_pack(zip_path: &Path) -> Option<ZipPack> {
    use std::io::Read;
    const MAX_ENTRY_BYTES: u64 = 10 * 1024 * 1024;
    let file = std::fs::File::open(zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut list: Option<ModList> = None;
    let mut configs: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).ok()?;
        if !entry.is_file() || entry.size() > MAX_ENTRY_BYTES {
            continue;
        }
        let raw_name = entry.name().replace('\\', "/");
        let base_name = raw_name.rsplit('/').next().unwrap_or(&raw_name).to_string();
        if list.is_none() && base_name.to_lowercase().ends_with(".rml") {
            let mut text = String::new();
            if entry.read_to_string(&mut text).is_ok() {
                let stem = base_name
                    .strip_suffix(".rml")
                    .or_else(|| base_name.strip_suffix(".RML"))
                    .unwrap_or(&base_name)
                    .to_string();
                list = parse_rml_str(&text, stem, zip_path.to_path_buf(), None);
            }
        } else if is_mod_config_name(&base_name) {
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            if entry.read_to_end(&mut bytes).is_ok() {
                configs.push((base_name, bytes));
            }
        }
    }
    Some(ZipPack { list: list?, configs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_modlists_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(id: &str, name: &str, steam: Option<&str>) -> ModListEntry {
        ModListEntry {
            id: id.into(),
            name: name.into(),
            steam_id: steam.map(str::to_string),
        }
    }

    #[test]
    fn write_then_read_keeps_order_names_and_version() {
        let dir = temp_dir("roundtrip");
        let path = path_for(&dir, "Vampires & co");
        let entries = vec![
            entry("brrainz.harmony", "Harmony", Some("2009463077")),
            entry("ludeon.rimworld", "Core", None),
            entry("someone.mod", "A <weird> & \"quoted\" name", Some("123")),
        ];
        write_rml(&path, "1.7.4985 rev1269", &entries).unwrap();
        let read = read_rml(&path).unwrap();
        assert_eq!(read.name, "Vampires & co");
        assert_eq!(read.game_version.as_deref(), Some("1.7.4985 rev1269"));
        assert_eq!(read.ids, ["brrainz.harmony", "ludeon.rimworld", "someone.mod"]);
        assert_eq!(read.names[2], "A <weird> & \"quoted\" name");
        assert_eq!(read.steam_id("BrrAinz.Harmony"), Some("2009463077"));
        assert_eq!(read.steam_id("ludeon.rimworld"), None, "0 is not a Workshop ID");
        assert_eq!(read.steam_id("someone.mod"), Some("123"));
        assert!(!path.with_extension("rml.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_game_layout_and_old_meta_only_files() {
        let dir = temp_dir("game");
        // As the game writes it: meta lists only some mods, the order is in modList.
        let game = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<savedModList>\n\t<meta>\n\t\t<gameVersion>1.6.4871 rev591</gameVersion>\n\t\t<modIds>\n\t\t\t<li>ludeon.rimworld</li>\n\t\t</modIds>\n\t</meta>\n\t<modList>\n\t\t<ids>\n\t\t\t<li>zetrith.prepatcher</li>\n\t\t\t<li>brrainz.harmony</li>\n\t\t\t<li>ludeon.rimworld</li>\n\t\t</ids>\n\t\t<names>\n\t\t\t<li>Prepatcher</li>\n\t\t\t<li>Harmony</li>\n\t\t\t<li>Core</li>\n\t\t</names>\n\t</modList>\n</savedModList>";
        std::fs::write(dir.join("1.rml"), game).unwrap();
        let meta_only = "<savedModList><meta><modIds><li>a.b</li><li>c.d</li></modIds></meta></savedModList>";
        std::fs::write(dir.join("old.rml"), meta_only).unwrap();
        std::fs::write(dir.join("broken.rml"), "not xml at all").unwrap();
        std::fs::write(dir.join("notes.txt"), "<savedModList/>").unwrap();

        let lists = list_modlists(&dir);
        let names: Vec<&str> = lists.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["1", "old"], "broken and non-.rml files are skipped");
        assert_eq!(
            lists[0].ids,
            ["zetrith.prepatcher", "brrainz.harmony", "ludeon.rimworld"]
        );
        assert_eq!(lists[0].game_version.as_deref(), Some("1.6.4871 rev591"));
        assert_eq!(lists[1].ids, ["a.b", "c.d"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_must_be_valid_file_names() {
        assert_eq!(check_name("  My set  "), Ok("My set".into()));
        assert_eq!(check_name("Вампиры 1.7"), Ok("Вампиры 1.7".into()));
        assert_eq!(check_name("   "), Err(NameProblem::Empty));
        assert_eq!(check_name("a/b"), Err(NameProblem::BadCharacters));
        assert_eq!(check_name("what?"), Err(NameProblem::BadCharacters));
        assert_eq!(check_name(&"x".repeat(81)), Err(NameProblem::TooLong));
        assert_eq!(check_name("ends with dots..."), Ok("ends with dots".into()));
    }

    #[test]
    fn copies_get_free_names_and_dates() {
        let dir = temp_dir("copy");
        write_rml(&path_for(&dir, "Main"), "1.6", &[entry("a.b", "A", None)]).unwrap();
        assert_eq!(copy_name(&dir, "Main", "copy"), "Main (copy)");
        write_rml(&path_for(&dir, "Main (copy)"), "1.6", &[]).unwrap();
        assert_eq!(copy_name(&dir, "Main", "copy"), "Main (copy 2)");
        assert!(read_rml(&path_for(&dir, "Main")).unwrap().modified.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_files_rml_or_mods_config() {
        let dir = temp_dir("shared");
        let config = dir.join("ModsConfig.xml");
        std::fs::write(
            &config,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  <version>1.6.4633 rev1261</version>\n  <activeMods>\n    <li>brrainz.harmony</li>\n    <li>ludeon.rimworld</li>\n  </activeMods>\n</ModsConfigData>",
        )
        .unwrap();
        let from_config = read_shared(&config).unwrap();
        assert_eq!(from_config.name, "ModsConfig");
        assert_eq!(from_config.ids, ["brrainz.harmony", "ludeon.rimworld"]);
        assert_eq!(from_config.game_version.as_deref(), Some("1.6.4633 rev1261"));

        let rml = path_for(&dir, "Friend");
        write_rml(&rml, "1.7", &[entry("a.b", "A", Some("42"))]).unwrap();
        let from_rml = read_shared(&rml).unwrap();
        assert_eq!(from_rml.steam_id("a.b"), Some("42"));

        std::fs::write(dir.join("junk.xml"), "<Defs/>").unwrap();
        assert!(read_shared(&dir.join("junk.xml")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_names_and_share_text() {
        let dir = temp_dir("unique");
        assert_eq!(unique_name(&dir, "Friend"), "Friend");
        write_rml(&path_for(&dir, "Friend"), "1.7", &[]).unwrap();
        assert_eq!(unique_name(&dir, "Friend"), "Friend (2)");
        let _ = std::fs::remove_dir_all(&dir);

        let text = share_text(
            "My pack",
            &[("Harmony".into(), Some("2009463077".into())), ("Core".into(), None)],
        );
        assert_eq!(
            text,
            "My pack (2)\n1. Harmony — https://steamcommunity.com/sharedfiles/filedetails/?id=2009463077\n2. Core\n"
        );
    }

    #[test]
    fn workshop_ids() {
        assert!(is_workshop_id("2009463077"));
        assert!(!is_workshop_id("0"));
        assert!(!is_workshop_id("000"));
        assert!(!is_workshop_id(""));
        assert!(!is_workshop_id("12a"));
    }

    #[test]
    fn modlists_folder_is_next_to_config() {
        let cfg =
            Path::new(r"C:\Users\u\AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Config\ModsConfig.xml");
        assert_eq!(
            modlists_dir(cfg).unwrap(),
            PathBuf::from(r"C:\Users\u\AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\ModLists")
        );
    }

    #[test]
    fn zip_pack_roundtrip_with_mod_configs() {
        let dir = temp_dir("zip_pack");
        let cfg_dir = dir.join("Config");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("Mod_2009463077_Harmony.xml"), b"<ModSettings/>").unwrap();
        std::fs::write(cfg_dir.join("ModsConfig.xml"), b"<ModsConfigData/>").unwrap();
        std::fs::write(cfg_dir.join("Prefs.xml"), b"<Prefs/>").unwrap();

        let entries = vec![
            entry("brrainz.harmony", "Harmony", Some("2009463077")),
            entry("ludeon.rimworld", "Core", None),
        ];
        let rml = rml_text("1.7", &entries);
        let zip_path = dir.join("MyPack.zip");
        let packed = write_zip_pack(&zip_path, "MyPack", &rml, Some(&cfg_dir)).unwrap();
        assert_eq!(packed, 1, "only Mod_*.xml files are packed");

        let unpacked = read_zip_pack(&zip_path).unwrap();
        assert_eq!(unpacked.list.name, "MyPack");
        assert_eq!(unpacked.list.ids, ["brrainz.harmony", "ludeon.rimworld"]);
        assert_eq!(
            unpacked.configs,
            vec![("Mod_2009463077_Harmony.xml".to_string(), b"<ModSettings/>".to_vec())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
