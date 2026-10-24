use crate::models::{ModInfo, ModOrigin};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;

pub fn scan_mods(
    workshop_dir: Option<&Path>,
    local_dir: Option<&Path>,
    game_data_dir: Option<&Path>,
) -> Vec<ModInfo> {
    let mut targets = Vec::new();

    for (dir_opt, origin) in [
        (workshop_dir, ModOrigin::Workshop),
        (local_dir, ModOrigin::Local),
        (game_data_dir, ModOrigin::Local),
    ] {
        if let Some(dir) = dir_opt {
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            targets.push((path, origin));
                        }
                    }
                }
            }
        }
    }

    let mods: Vec<ModInfo> = targets
        .into_par_iter()
        .filter_map(|(dir, origin)| parse_mod_folder(&dir, origin))
        .collect();

    mods
}

pub fn parse_mod_folder(dir: &Path, origin: ModOrigin) -> Option<ModInfo> {
    let about_file = dir.join("About").join("About.xml");
    if !about_file.exists() {
        return None;
    }

    let xml_content = std::fs::read_to_string(&about_file).ok()?;
    let meta = parse_about_xml(&xml_content)?;

    let languages = scan_languages(dir);

    let preview_path = [
        dir.join("About").join("Preview.png"),
        dir.join("About").join("Preview.jpg"),
        dir.join("About").join("preview.png"),
        dir.join("About").join("preview.jpg"),
        dir.join("Preview.png"),
        dir.join("preview.png"),
        dir.join("Preview.jpg"),
        dir.join("preview.jpg"),
    ]
    .into_iter()
    .find(|p| p.is_file());

    let steam_item_id = match origin {
        ModOrigin::Workshop => dir
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
            .map(|s| s.to_string()),
        ModOrigin::Local => None,
    };

    let (has_defs, has_assemblies, has_other_content) = scan_content_kinds(dir);

    let name_lower = meta.name.to_lowercase();
    Some(ModInfo {
        package_id: meta.package_id,
        name: meta.name,
        name_lower,
        author: meta.author,
        description: clean_description(&meta.description),
        dir_path: dir.to_path_buf(),
        preview_path,
        supported_versions: meta.supported_versions,
        load_after: meta.load_after,
        force_load_after: meta.force_load_after,
        load_before: meta.load_before,
        force_load_before: meta.force_load_before,
        dependencies: meta.dependencies,
        origin,
        languages,
        is_translation_mod: false,
        parent_package_id: None,
        steam_item_id,
        has_defs,
        has_assemblies,
        has_other_content,
    })
}

/// Folder names that mean "this mod has real content" (besides Defs and DLLs).
const CONTENT_DIR_NAMES: &[&str] = &[
    "patches",
    "textures",
    "sounds",
    "assetbundles",
    "shaders",
    "meshes",
    "materials",
];

/// Returns (has_defs, has_assemblies, has_other_content).
/// Does not look inside About/ and Languages/ folders.
pub fn scan_content_kinds(dir: &Path) -> (bool, bool, bool) {
    let mut has_defs = false;
    let mut has_assemblies = false;
    let mut has_other = false;

    let walker = walkdir::WalkDir::new(dir)
        .max_depth(4)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            name != "languages" && name != "about" && name != ".git" && name != "source"
        });

    for entry in walker.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if entry.file_type().is_file() {
            if name.ends_with(".dll") {
                has_assemblies = true;
            }
        } else if entry.file_type().is_dir() && entry.depth() > 0 {
            if name == "defs" {
                has_defs = true;
            } else if name == "assemblies" {
                // An Assemblies folder counts even if the DLL sits deeper than the walk depth.
                has_assemblies = true;
            } else if CONTENT_DIR_NAMES.contains(&name.as_str()) {
                has_other = true;
            }
        }
        if has_defs && has_assemblies && has_other {
            break;
        }
    }

    (has_defs, has_assemblies, has_other)
}

#[derive(Default, Debug)]
pub struct ParsedAboutMeta {
    pub package_id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub supported_versions: Vec<String>,
    pub load_after: Vec<String>,
    pub force_load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub force_load_before: Vec<String>,
    pub dependencies: Vec<String>,
}

pub fn parse_about_xml(xml: &str) -> Option<ParsedAboutMeta> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut meta = ParsedAboutMeta::default();
    let mut buf = Vec::new();
    let mut current_tag = String::new();
    let mut tag_stack: Vec<String> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                current_tag = tag.clone();
                tag_stack.push(tag);
            }
            Ok(Event::End(_)) => {
                tag_stack.pop();
                current_tag = tag_stack.last().cloned().unwrap_or_default();
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().trim().to_string();
                if !text.is_empty() {
                    let parent_tag = if tag_stack.len() >= 2 {
                        tag_stack[tag_stack.len() - 2].as_str()
                    } else {
                        ""
                    };

                    let is_direct_child =
                        tag_stack.len() == 2 && tag_stack[0].eq_ignore_ascii_case("modmetadata");

                    match current_tag.to_lowercase().as_str() {
                        "packageid" => {
                            if is_direct_child {
                                meta.package_id = text;
                            } else if parent_tag.eq_ignore_ascii_case("li")
                                && tag_stack
                                    .iter()
                                    .any(|t| {
                                        t.eq_ignore_ascii_case("moddependencies")
                                            || t.eq_ignore_ascii_case("moddependenciesbyversion")
                                    })
                            {
                                meta.dependencies.push(text);
                            }
                        }
                        "name" => {
                            if is_direct_child {
                                meta.name = text;
                            }
                        }
                        "author" => {
                            if is_direct_child {
                                meta.author = text;
                            }
                        }
                        "description" => {
                            if is_direct_child {
                                if !meta.description.is_empty() {
                                    meta.description.push(' ');
                                }
                                meta.description.push_str(&text);
                            }
                        }
                        "li" => match parent_tag.to_lowercase().as_str() {
                            "supportedversions" => meta.supported_versions.push(text),
                            "loadafter" => meta.load_after.push(text),
                            "forceloadafter" => meta.force_load_after.push(text),
                            "loadbefore" => meta.load_before.push(text),
                            "forceloadbefore" => meta.force_load_before.push(text),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            Ok(Event::CData(ref e)) => {
                if current_tag.eq_ignore_ascii_case("description") {
                    let is_direct_child =
                        tag_stack.len() == 2 && tag_stack[0].eq_ignore_ascii_case("modmetadata");
                    if is_direct_child {
                        let text = String::from_utf8_lossy(e.as_ref()).trim().to_string();
                        if !text.is_empty() {
                            if !meta.description.is_empty() {
                                meta.description.push(' ');
                            }
                            meta.description.push_str(&text);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }

    if meta.package_id.is_empty() {
        return None;
    }
    if meta.name.is_empty() {
        meta.name = match meta.package_id.to_lowercase().as_str() {
            "ludeon.rimworld" => "Core".to_string(),
            "ludeon.rimworld.royalty" => "Royalty".to_string(),
            "ludeon.rimworld.ideology" => "Ideology".to_string(),
            "ludeon.rimworld.biotech" => "Biotech".to_string(),
            "ludeon.rimworld.anomaly" => "Anomaly".to_string(),
            "ludeon.rimworld.odyssey" => "Odyssey".to_string(),
            _ => meta.package_id.clone(),
        };
    }

    Some(meta)
}

static BBCODE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\[/?(b|i|u|list|\*|h1|h2|h3|code|strike|spoiler|url.*?|table|tr|th|td)\]")
        .unwrap()
});

fn clean_description(desc: &str) -> String {
    if desc.is_empty() {
        return String::new();
    }
    let cleaned = BBCODE_RE.replace_all(desc, "");
    cleaned.trim().to_string()
}

pub fn scan_languages(mod_dir: &Path) -> Vec<String> {
    let mut langs = HashSet::new();

    let root_langs = mod_dir.join("Languages");
    if root_langs.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&root_langs) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        langs.insert(name.to_string());
                    }
                }
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(mod_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let ver_langs = path.join("Languages");
                if ver_langs.is_dir() {
                    if let Ok(l_entries) = std::fs::read_dir(&ver_langs) {
                        for l_entry in l_entries.flatten() {
                            if l_entry.path().is_dir() {
                                if let Some(name) = l_entry.file_name().to_str() {
                                    langs.insert(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut result: Vec<String> = langs.into_iter().collect();
    result.sort();
    result
}

/// Removes extra copies of the same packageId (case-insensitive), keeping scan order.
/// A local copy wins over a Workshop copy — RimWorld refers to the Workshop one as `<id>_steam`.
/// Returns the unique mods and the list of duplicates found (for a warning in the UI).
pub fn dedupe_mods(mods: Vec<ModInfo>) -> (Vec<ModInfo>, Vec<DuplicateModEntry>) {
    let mut unique: Vec<ModInfo> = Vec::with_capacity(mods.len());
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut dups: Vec<DuplicateModEntry> = Vec::new();
    let mut dup_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for m in mods {
        let key = m.package_id.to_lowercase();
        match index.get(&key) {
            None => {
                index.insert(key, unique.len());
                unique.push(m);
            }
            Some(&pos) => {
                let di = *dup_index.entry(key.clone()).or_insert_with(|| {
                    dups.push(DuplicateModEntry {
                        package_id: unique[pos].package_id.clone(),
                        name: unique[pos].name.clone(),
                        origins: vec![(unique[pos].origin, unique[pos].dir_path.clone())],
                    });
                    dups.len() - 1
                });
                dups[di].origins.push((m.origin, m.dir_path.clone()));
                if m.origin == ModOrigin::Local && unique[pos].origin == ModOrigin::Workshop {
                    unique[pos] = m;
                }
            }
        }
    }
    (unique, dups)
}

#[derive(Debug, Clone)]
pub struct DuplicateModEntry {
    pub package_id: String,
    pub name: String,
    pub origins: Vec<(ModOrigin, std::path::PathBuf)>,
}

#[derive(Debug, Default)]
pub struct ModScanReport {
    pub duplicates: Vec<DuplicateModEntry>,
    pub unique_mods: std::collections::HashMap<String, ModInfo>,
}

pub fn scan_and_deduplicate(discovered_mods: Vec<ModInfo>) -> ModScanReport {
    let mut report = ModScanReport::default();
    let mut grouped: std::collections::HashMap<String, Vec<ModInfo>> = std::collections::HashMap::new();

    for m in discovered_mods {
        let key = m.package_id.to_lowercase();
        grouped.entry(key).or_default().push(m);
    }

    for (pkg_id, instances) in grouped {
        if instances.len() > 1 {
            let mod_name = instances[0].name.clone();
            let origins = instances
                .iter()
                .map(|inst| (inst.origin, inst.dir_path.clone()))
                .collect();

            report.duplicates.push(DuplicateModEntry {
                package_id: pkg_id.clone(),
                name: mod_name,
                origins,
            });

            let chosen = instances
                .into_iter()
                .max_by_key(|inst| match inst.origin {
                    ModOrigin::Local => 2,
                    ModOrigin::Workshop => 1,
                })
                .unwrap();

            report.unique_mods.insert(pkg_id, chosen);
        } else if let Some(single) = instances.into_iter().next() {
            report.unique_mods.insert(pkg_id, single);
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, origin: ModOrigin, dir: &str) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            name: id.to_string(),
            name_lower: id.to_lowercase(),
            author: String::new(),
            description: String::new(),
            dir_path: std::path::PathBuf::from(dir),
            preview_path: None,
            supported_versions: vec![],
            load_after: vec![],
            force_load_after: vec![],
            load_before: vec![],
            force_load_before: vec![],
            dependencies: vec![],
            origin,
            languages: vec![],
            is_translation_mod: false,
            parent_package_id: None,
            steam_item_id: None,
            has_defs: true,
            has_assemblies: false,
            has_other_content: false,
        }
    }

    #[test]
    fn dedupe_keeps_order_prefers_local_and_reports() {
        let mods = vec![
            m("a.first", ModOrigin::Workshop, "ws/1"),
            m("brrainz.harmony", ModOrigin::Workshop, "ws/2009463077"),
            m("b.second", ModOrigin::Workshop, "ws/3"),
            m("Brrainz.Harmony", ModOrigin::Local, "Mods/Harmony"),
        ];
        let (unique, dups) = dedupe_mods(mods);
        let ids: Vec<&str> = unique.iter().map(|m| m.package_id.as_str()).collect();
        assert_eq!(ids, vec!["a.first", "Brrainz.Harmony", "b.second"]);
        assert_eq!(unique[1].origin, ModOrigin::Local);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].origins.len(), 2);
    }

    #[test]
    fn content_kinds_ignore_languages_folder() {
        let dir = std::env::temp_dir().join(format!("rwmm_scan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("About")).unwrap();
        std::fs::create_dir_all(dir.join("Languages").join("Russian").join("Textures")).unwrap();
        assert_eq!(scan_content_kinds(&dir), (false, false, false), "translated textures inside Languages don't count");
        std::fs::create_dir_all(dir.join("1.6").join("Patches")).unwrap();
        assert_eq!(scan_content_kinds(&dir), (false, false, true));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
