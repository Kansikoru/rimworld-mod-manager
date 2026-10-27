use crate::models::{ModInfo, ModOrigin};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;

/// A folder's identity for comparing paths: the resolved path if it exists, else the
/// path itself; in lower case, since Windows paths ignore case.
fn dir_key(dir: &Path) -> String {
    std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// The folders to read mods from, each once. The same folder can be set both as the
/// Workshop folder and as the game's Mods folder (a game without Steam, where Workshop
/// mods are copied into Mods): it is then read once, as the game's own Mods folder.
fn source_dirs<'a>(
    workshop_dir: Option<&'a Path>,
    local_dir: Option<&'a Path>,
    game_data_dir: Option<&'a Path>,
) -> Vec<(&'a Path, ModOrigin)> {
    let sources = [
        (workshop_dir, ModOrigin::Workshop),
        (local_dir, ModOrigin::Local),
        (game_data_dir, ModOrigin::Local),
    ];
    let mut out: Vec<(&Path, ModOrigin, String)> = Vec::new();
    for (dir, origin) in sources.into_iter().filter_map(|(d, o)| d.map(|d| (d, o))) {
        let key = dir_key(dir);
        match out.iter_mut().find(|(_, _, k)| *k == key) {
            // Seen already: the game's Mods folder wins over "Workshop".
            Some(existing) => {
                if origin == ModOrigin::Local {
                    existing.1 = ModOrigin::Local;
                }
            }
            None => out.push((dir, origin, key)),
        }
    }
    out.into_iter().map(|(d, o, _)| (d, o)).collect()
}

pub fn scan_mods(workshop_dir: Option<&Path>, local_dir: Option<&Path>, game_data_dir: Option<&Path>) -> Vec<ModInfo> {
    let mut targets = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (dir, origin) in source_dirs(workshop_dir, local_dir, game_data_dir) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // A mod folder reached twice (a link to another folder) is read once.
                if path.is_dir() && seen.insert(dir_key(&path)) {
                    targets.push((path, origin));
                }
            }
        }
    }

    // Def counts are cached by a fingerprint of each mod's Defs files: a rescan only
    // re-reads XML of mods that changed.
    let cache_path = crate::categories::cache_path();
    let cache = crate::categories::load_cache(&cache_path);

    let results: Vec<(ModInfo, crate::categories::DefStatsCacheEntry)> = targets
        .into_par_iter()
        .filter_map(|(dir, origin)| parse_mod_folder(&dir, origin, &cache))
        .collect();

    let new_cache: crate::categories::DefStatsCache = results
        .iter()
        .map(|(m, entry)| (m.dir_path.to_string_lossy().to_string(), entry.clone()))
        .collect();
    crate::categories::save_cache(&cache_path, &new_cache);

    results.into_iter().map(|(m, _)| m).collect()
}

pub fn parse_mod_folder(
    dir: &Path,
    origin: ModOrigin,
    cache: &crate::categories::DefStatsCache,
) -> Option<(ModInfo, crate::categories::DefStatsCacheEntry)> {
    let about_file = dir.join("About").join("About.xml");
    if !about_file.exists() {
        return None;
    }

    // Lossy: a stray non-UTF-8 byte must not make the whole mod disappear.
    let xml_bytes = std::fs::read(&about_file).ok()?;
    let meta = parse_about_xml(&String::from_utf8_lossy(&xml_bytes))?;

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

    let content = scan_content(dir);
    let (def_stats, cache_entry) = crate::categories::def_stats_for(dir, &content.def_dirs, cache);

    let name_lower = meta.name.to_lowercase();
    let info = ModInfo {
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
        has_defs: content.has_defs,
        has_assemblies: content.has_assemblies,
        has_other_content: content.has_other,
        has_patches: content.has_patches,
        has_textures: content.has_textures,
        def_stats,
        categories: Vec::new(),
        incompatible_with: meta.incompatible_with,
        dependency_alternatives: meta.dependency_alternatives,
        by_version: meta.by_version,
    };
    Some((info, cache_entry))
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

/// What kinds of content a mod folder has. About/ and Languages/ are not looked into.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContentKinds {
    pub has_defs: bool,
    pub has_assemblies: bool,
    /// Patches, Textures, Sounds, AssetBundles, Shaders...
    pub has_other: bool,
    pub has_patches: bool,
    pub has_textures: bool,
    /// Every Defs folder (root and version folders like 1.6/Defs).
    pub def_dirs: Vec<std::path::PathBuf>,
}

pub fn scan_content(dir: &Path) -> ContentKinds {
    let mut kinds = ContentKinds::default();
    let mut dll_found = false;

    {
        let walker = walkdir::WalkDir::new(dir).max_depth(4).into_iter().filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            match name.as_str() {
                "languages" | "about" | ".git" | "source" => false,
                "defs" => {
                    kinds.has_defs = true;
                    kinds.def_dirs.push(e.path().to_path_buf());
                    false // read later, file by file, by the category scanner
                }
                "assemblies" => {
                    // Counts even if the DLL sits deeper than the walk depth.
                    kinds.has_assemblies = true;
                    true
                }
                other if CONTENT_DIR_NAMES.contains(&other) => {
                    kinds.has_other = true;
                    match other {
                        "patches" => kinds.has_patches = true,
                        "textures" => kinds.has_textures = true,
                        _ => {}
                    }
                    false // thousands of files inside; the folder itself is enough
                }
                _ => true,
            }
        });

        for entry in walker.flatten() {
            if entry.file_type().is_file() && entry.file_name().to_string_lossy().to_lowercase().ends_with(".dll") {
                dll_found = true;
            }
        }
    }

    kinds.has_assemblies |= dll_found;
    kinds
}

/// Returns (has_defs, has_assemblies, has_other_content).
pub fn scan_content_kinds(dir: &Path) -> (bool, bool, bool) {
    let k = scan_content(dir);
    (k.has_defs, k.has_assemblies, k.has_other)
}

/// A dependency from `modDependencies`: the required packageId and the IDs that can
/// stand in for it (`alternativePackageIds`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AboutDependency {
    pub package_id: String,
    pub alternatives: Vec<String>,
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
    /// Alternatives per dependency (lowercase dependency id -> alternative ids).
    pub dependency_alternatives: std::collections::HashMap<String, Vec<String>>,
    pub incompatible_with: Vec<String>,
    /// `*ByVersion` blocks, keyed by game version ("1.6").
    pub by_version: crate::models::VersionedLists,
}

/// Minimal XML tree: About.xml files are small, and reading fields from a tree is far
/// more reliable than tracking a tag stack.
#[derive(Debug, Default)]
struct XmlNode {
    /// Lowercase tag name.
    name: String,
    text: String,
    children: Vec<XmlNode>,
}

impl XmlNode {
    fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| c.name == name)
    }
    fn text_of(&self, name: &str) -> String {
        self.child(name).map(|c| c.text.trim().to_string()).unwrap_or_default()
    }
    /// Texts of `<li>` children of `<name>`.
    fn list(&self, name: &str) -> Vec<String> {
        self.child(name).map(li_texts).unwrap_or_default()
    }
}

fn li_texts(node: &XmlNode) -> Vec<String> {
    node.children
        .iter()
        .filter(|c| c.name == "li")
        .map(|c| c.text.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Builds the tree. A syntax error stops reading but keeps everything parsed so far,
/// so a mod with a slightly broken About.xml still shows up instead of vanishing.
fn parse_xml_tree(xml: &str) -> Option<XmlNode> {
    let mut reader = Reader::from_str(xml.trim_start_matches('\u{feff}'));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack: Vec<XmlNode> = vec![XmlNode::default()]; // synthetic document node

    let push_text = |stack: &mut Vec<XmlNode>, text: &str| {
        if let Some(top) = stack.last_mut() {
            if !top.text.is_empty() {
                top.text.push(' ');
            }
            top.text.push_str(text.trim());
        }
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => stack.push(XmlNode {
                name: String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase(),
                ..Default::default()
            }),
            Ok(Event::Empty(ref e)) => {
                let node = XmlNode {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase(),
                    ..Default::default()
                };
                if let Some(top) = stack.last_mut() {
                    top.children.push(node);
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let node = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(node);
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e
                    .unescape()
                    .map(|t| t.to_string())
                    .unwrap_or_else(|_| String::from_utf8_lossy(e.as_ref()).to_string());
                push_text(&mut stack, &text);
            }
            Ok(Event::CData(ref e)) => push_text(&mut stack, &String::from_utf8_lossy(e.as_ref())),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    // Close nodes left open by a syntax error.
    while stack.len() > 1 {
        let node = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(node);
    }
    let doc = stack.pop()?;
    doc.children.into_iter().find(|c| c.name == "modmetadata")
}

fn parse_dependencies(node: &XmlNode) -> Vec<AboutDependency> {
    node.children
        .iter()
        .filter(|c| c.name == "li")
        .filter_map(|li| {
            let package_id = li.text_of("packageid");
            (!package_id.is_empty()).then(|| AboutDependency {
                package_id,
                alternatives: li.list("alternativepackageids"),
            })
        })
        .collect()
}

/// `<xByVersion><v1.6><li>..</li></v1.6></xByVersion>` -> {"1.6": [..]}
fn by_version<T>(
    root: &XmlNode,
    name: &str,
    read: impl Fn(&XmlNode) -> Vec<T>,
) -> std::collections::HashMap<String, Vec<T>> {
    root.child(name)
        .map(|block| {
            block
                .children
                .iter()
                .map(|v| (v.name.trim_start_matches('v').to_string(), read(v)))
                .filter(|(_, items)| !items.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_about_xml(xml: &str) -> Option<ParsedAboutMeta> {
    let root = parse_xml_tree(xml)?;
    let mut meta = ParsedAboutMeta {
        package_id: root.text_of("packageid"),
        name: root.text_of("name"),
        author: root.text_of("author"),
        description: root.text_of("description"),
        supported_versions: root.list("supportedversions"),
        load_after: root.list("loadafter"),
        force_load_after: root.list("forceloadafter"),
        load_before: root.list("loadbefore"),
        force_load_before: root.list("forceloadbefore"),
        incompatible_with: root.list("incompatiblewith"),
        ..Default::default()
    };
    if meta.author.is_empty() {
        meta.author = root.list("authors").join(", ");
    }

    let mut add_deps = |deps: Vec<AboutDependency>, into_versioned: Option<&str>| {
        for d in deps {
            if !d.alternatives.is_empty() {
                meta.dependency_alternatives
                    .insert(d.package_id.to_lowercase(), d.alternatives.clone());
            }
            match into_versioned {
                Some(v) => meta
                    .by_version
                    .dependencies
                    .entry(v.to_string())
                    .or_default()
                    .push(d.package_id),
                None => meta.dependencies.push(d.package_id),
            }
        }
    };
    if let Some(deps) = root.child("moddependencies") {
        add_deps(parse_dependencies(deps), None);
    }
    for (version, deps) in by_version(&root, "moddependenciesbyversion", parse_dependencies) {
        add_deps(deps, Some(&version));
    }
    meta.by_version.load_after = by_version(&root, "loadafterbyversion", li_texts);
    meta.by_version.load_before = by_version(&root, "loadbeforebyversion", li_texts);
    meta.by_version.incompatible_with = by_version(&root, "incompatiblewithbyversion", li_texts);

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

/// Adds the `*ByVersion` lists for the installed game version to the plain lists
/// (all versions when the game version is unknown). Call once after scanning.
pub fn resolve_versioned_fields(mods: &mut [ModInfo], game_mm: Option<&str>) {
    fn add(into: &mut Vec<String>, map: &std::collections::HashMap<String, Vec<String>>, game_mm: Option<&str>) {
        let lists: Vec<&Vec<String>> = match game_mm {
            Some(v) => map.get(v).into_iter().collect(),
            None => map.values().collect(),
        };
        for id in lists.into_iter().flatten() {
            if !into.iter().any(|x| x.eq_ignore_ascii_case(id)) {
                into.push(id.clone());
            }
        }
    }
    for m in mods.iter_mut() {
        let v = m.by_version.clone();
        add(&mut m.dependencies, &v.dependencies, game_mm);
        add(&mut m.load_after, &v.load_after, game_mm);
        add(&mut m.load_before, &v.load_before, game_mm);
        add(&mut m.incompatible_with, &v.incompatible_with, game_mm);
    }
}
static BBCODE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\[/?(b|i|u|list|\*|h1|h2|h3|code|strike|spoiler|url.*?|table|tr|th|td)\]").unwrap()
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
                // The same folder reached twice is not a second copy.
                if dir_key(&m.dir_path) == dir_key(&unique[pos].dir_path) {
                    continue;
                }
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
            ..Default::default()
        }
    }

    #[test]
    fn one_folder_set_as_workshop_and_mods_is_read_once() {
        let dir = std::env::temp_dir().join(format!("rwmm_scan_same_{}", std::process::id()));
        let data = dir.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let mods = dir.join("Mods");
        std::fs::create_dir_all(&mods).unwrap();
        // Same folder, written differently (case, trailing slash), as on a game without Steam.
        let mods_other_spelling = std::path::PathBuf::from(format!("{}\\", mods.display().to_string().to_uppercase()));

        let sources = source_dirs(Some(&mods_other_spelling), Some(&mods), Some(&data));
        assert_eq!(sources.len(), 2, "Mods once, Data once");
        assert_eq!(
            sources[0].1,
            ModOrigin::Local,
            "the game's Mods folder wins over Workshop"
        );
        assert_eq!(sources[1].0, data.as_path());

        // A real Steam setup keeps both folders.
        let workshop = dir.join("workshop");
        std::fs::create_dir_all(&workshop).unwrap();
        let sources = source_dirs(Some(&workshop), Some(&mods), None);
        assert_eq!(
            sources.iter().map(|s| s.1).collect::<Vec<_>>(),
            [ModOrigin::Workshop, ModOrigin::Local]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_folder_is_not_reported_as_a_copy() {
        let dir = std::env::temp_dir().join(format!("rwmm_scan_dup_{}", std::process::id()));
        let folder = dir.join("Mods").join("1195427067");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.display().to_string();
        let (unique, dups) = dedupe_mods(vec![
            m("com.bymarcin.ArchitectIcons", ModOrigin::Workshop, &path),
            m("com.bymarcin.ArchitectIcons", ModOrigin::Local, &path.to_uppercase()),
        ]);
        assert_eq!(unique.len(), 1);
        assert!(dups.is_empty(), "one folder seen twice is not two copies");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const ABOUT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ModMetaData>
  <name>Test &amp; Mod</name>
  <author>Someone</author>
  <packageId>Someone.TestMod</packageId>
  <supportedVersions><li>1.5</li><li>1.6</li></supportedVersions>
  <description><![CDATA[Line one]]></description>
  <modDependencies>
    <li>
      <alternativePackageIds><li>zetrith.prepatcher</li></alternativePackageIds>
      <packageId>brrainz.harmony</packageId>
      <displayName>Harmony</displayName>
    </li>
  </modDependencies>
  <modDependenciesByVersion>
    <v1.6><li><packageId>OskarPotocki.VanillaFactionsExpanded.Core</packageId></li></v1.6>
    <v1.5><li><packageId>old.framework</packageId></li></v1.5>
  </modDependenciesByVersion>
  <loadAfter><li>ludeon.rimworld</li></loadAfter>
  <loadAfterByVersion><v1.6><li>a.after16</li></v1.6></loadAfterByVersion>
  <loadBeforeByVersion><v1.5><li>b.before15</li></v1.5></loadBeforeByVersion>
  <incompatibleWith><li>bad.mod</li></incompatibleWith>
</ModMetaData>"#;

    #[test]
    fn parses_about_with_versions_alternatives_incompatible() {
        let meta = parse_about_xml(ABOUT).unwrap();
        assert_eq!(meta.package_id, "Someone.TestMod");
        assert_eq!(meta.name, "Test & Mod");
        assert_eq!(meta.description, "Line one");
        assert_eq!(meta.supported_versions, vec!["1.5", "1.6"]);
        assert_eq!(meta.dependencies, vec!["brrainz.harmony"]);
        assert_eq!(
            meta.dependency_alternatives["brrainz.harmony"],
            vec!["zetrith.prepatcher"]
        );
        assert_eq!(meta.incompatible_with, vec!["bad.mod"]);
        assert_eq!(meta.load_after, vec!["ludeon.rimworld"]);
        assert_eq!(
            meta.by_version.dependencies["1.6"],
            vec!["OskarPotocki.VanillaFactionsExpanded.Core"]
        );
        assert_eq!(meta.by_version.load_after["1.6"], vec!["a.after16"]);
        assert_eq!(meta.by_version.load_before["1.5"], vec!["b.before15"]);

        // Resolve for 1.6: only the 1.6 blocks are added
        let mut mods = vec![ModInfo {
            package_id: meta.package_id.clone(),
            dependencies: meta.dependencies.clone(),
            load_after: meta.load_after.clone(),
            by_version: meta.by_version.clone(),
            ..Default::default()
        }];
        resolve_versioned_fields(&mut mods, Some("1.6"));
        assert_eq!(
            mods[0].dependencies,
            vec!["brrainz.harmony", "OskarPotocki.VanillaFactionsExpanded.Core"]
        );
        assert_eq!(mods[0].load_after, vec!["ludeon.rimworld", "a.after16"]);
        assert!(mods[0].load_before.is_empty(), "1.5-only loadBefore is not used on 1.6");
    }

    #[test]
    fn broken_about_xml_still_gives_the_mod() {
        // Unescaped '&' and an unclosed tag: the mod must not vanish from the list.
        let broken = "<ModMetaData><packageId>a.b</packageId><name>Rock & Roll</name><description>oops";
        let meta = parse_about_xml(broken).expect("partial parse");
        assert_eq!(meta.package_id, "a.b");
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
        assert_eq!(
            scan_content_kinds(&dir),
            (false, false, false),
            "translated textures inside Languages don't count"
        );
        std::fs::create_dir_all(dir.join("1.6").join("Patches")).unwrap();
        assert_eq!(scan_content_kinds(&dir), (false, false, true));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
