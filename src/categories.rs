//! Mod categories computed from what a mod actually contains (like RimPy's tags),
//! not guessed from its name. Works for any mod, including ones no database knows.
//!
//! 1. While scanning, every XML file in the mod's `Defs` folders is read once and the
//!    def types are counted (`DefStats`): races, factions, buildings, weapons, ...
//!    The result is cached by a fingerprint of the Defs files, so rescans are fast.
//! 2. `assign_categories` turns the counts (+ DLLs, Patches, Textures, how many other
//!    mods depend on it) into categories.
//! 3. The user can override categories of a mod; overrides are saved to disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::i18n::Language;
use crate::models::ModInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModCategory {
    Framework,
    QoL,
    Race,
    Faction,
    Building,
    Weapon,
    Apparel,
    Animal,
    Biome,
    Storyteller,
    Patch,
    Retexture,
    Translation,
}

impl ModCategory {
    pub const ALL: [ModCategory; 13] = [
        ModCategory::Framework,
        ModCategory::QoL,
        ModCategory::Race,
        ModCategory::Faction,
        ModCategory::Building,
        ModCategory::Weapon,
        ModCategory::Apparel,
        ModCategory::Animal,
        ModCategory::Biome,
        ModCategory::Storyteller,
        ModCategory::Patch,
        ModCategory::Retexture,
        ModCategory::Translation,
    ];

    pub fn label(&self, lang: Language) -> &'static str {
        let ru = lang == Language::Russian;
        match self {
            ModCategory::Framework => if ru { "Фреймворк" } else { "Framework" },
            ModCategory::QoL => if ru { "QoL / интерфейс" } else { "QoL / UI" },
            ModCategory::Race => if ru { "Расы" } else { "Races" },
            ModCategory::Faction => if ru { "Фракции" } else { "Factions" },
            ModCategory::Building => if ru { "Постройки" } else { "Buildings" },
            ModCategory::Weapon => if ru { "Оружие" } else { "Weapons" },
            ModCategory::Apparel => if ru { "Одежда" } else { "Apparel" },
            ModCategory::Animal => if ru { "Животные" } else { "Animals" },
            ModCategory::Biome => if ru { "Биомы" } else { "Biomes" },
            ModCategory::Storyteller => if ru { "Рассказчик" } else { "Storyteller" },
            ModCategory::Patch => if ru { "Патч" } else { "Patch" },
            ModCategory::Retexture => if ru { "Ретекстур" } else { "Retexture" },
            ModCategory::Translation => if ru { "Перевод" } else { "Translation" },
        }
    }

    /// (background, foreground) RGB for the badge.
    /// (background, text) of the category badge: a soft pastel text color on the same
    /// color at 18% over a list row (`#1B1530`), like the other badges.
    pub fn colors(&self) -> ([u8; 3], [u8; 3]) {
        let fg: [u8; 3] = match self {
            ModCategory::Framework => [0xB6, 0x9C, 0xFF],
            ModCategory::QoL => [0x8E, 0xC5, 0xFF],
            ModCategory::Race => [0xE0, 0xA3, 0xFF],
            ModCategory::Faction => [0xFB, 0xBF, 0x77],
            ModCategory::Building => [0xA5, 0xB4, 0xFC],
            ModCategory::Weapon => [0xF4, 0x7F, 0xA0],
            ModCategory::Apparel => [0xC4, 0xB5, 0xFD],
            ModCategory::Animal => [0xFD, 0xE6, 0x8A],
            ModCategory::Biome => [0x6E, 0xE7, 0xB7],
            ModCategory::Storyteller => [0xF0, 0xAB, 0xFC],
            ModCategory::Patch => [0x5E, 0xEA, 0xD4],
            ModCategory::Retexture => [0xF9, 0xA8, 0xD4],
            ModCategory::Translation => [0xA8, 0xB8, 0xFF],
        };
        const ROW: [u8; 3] = [0x1B, 0x15, 0x30];
        let bg = [0, 1, 2].map(|i| (ROW[i] as f32 + (fg[i] as f32 - ROW[i] as f32) * 0.18).round() as u8);
        (bg, fg)
    }
}

/// Counts of def types found in a mod's Defs folders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefStats {
    pub total_defs: u32,
    pub buildings: u32,
    pub weapons: u32,
    pub apparel: u32,
    pub animals: u32,
    pub humanlike: u32,
    pub alien_races: u32,
    pub xenotypes: u32,
    pub factions: u32,
    pub biomes: u32,
    pub storytellers: u32,
}

impl DefStats {
    fn add(&mut self, other: &DefStats) {
        self.total_defs += other.total_defs;
        self.buildings += other.buildings;
        self.weapons += other.weapons;
        self.apparel += other.apparel;
        self.animals += other.animals;
        self.humanlike += other.humanlike;
        self.alien_races += other.alien_races;
        self.xenotypes += other.xenotypes;
        self.factions += other.factions;
        self.biomes += other.biomes;
        self.storytellers += other.storytellers;
    }
}

// Defs parsing
fn attr(e: &BytesStart, name: &[u8]) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok().map(|v| v.to_string()))
        .unwrap_or_default()
}

/// What we learn about one `<ThingDef>` while reading it.
#[derive(Default)]
struct ThingInfo {
    parent: String,
    is_abstract: bool,
    category: String,
    thing_class: String,
    has_weapon_tags: bool,
    has_apparel: bool,
    has_race: bool,
    has_building: bool,
    intelligence: String,
}

impl ThingInfo {
    fn finish(self, stats: &mut DefStats) {
        if self.is_abstract {
            return;
        }
        let parent = self.parent.to_lowercase();
        if self.has_race || parent.contains("animal") {
            if self.intelligence.eq_ignore_ascii_case("humanlike") {
                stats.humanlike += 1;
            } else {
                stats.animals += 1;
            }
        } else if self.has_weapon_tags
            || parent.contains("weapon")
            || parent.contains("gun")
            || parent.contains("melee")
        {
            stats.weapons += 1;
        } else if self.has_apparel || parent.contains("apparel") {
            stats.apparel += 1;
        } else if self.category.eq_ignore_ascii_case("building")
            || self.has_building
            || self.thing_class.starts_with("Building")
            || ["building", "furniture", "bed", "wall", "bench", "table", "chair", "door", "lamp"]
                .iter()
                .any(|k| parent.contains(k))
        {
            stats.buildings += 1;
        }
    }
}

/// Counts def types in one Defs XML file. Only the tags that matter are looked at.
pub fn parse_defs_xml(xml: &str, stats: &mut DefStats) {
    let xml = xml.trim_start_matches('\u{feff}');
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut depth = 0usize;
    let mut thing: Option<ThingInfo> = None;
    // Which ThingDef child (depth 3) or race child (depth 4) text we are collecting.
    let mut capture: Option<&'static str> = None;
    let mut in_race = false;

    // Handles an opening tag at `level` (1 = <Defs>, 2 = a def, 3 = def field, 4 = sub-field).
    let on_open = |e: &BytesStart,
                       level: usize,
                       thing: &mut Option<ThingInfo>,
                       capture: &mut Option<&'static str>,
                       in_race: &mut bool,
                       stats: &mut DefStats,
                       is_empty: bool| {
        let name_bytes = e.local_name();
        let name = String::from_utf8_lossy(name_bytes.as_ref()).to_string();
        match level {
            2 => {
                stats.total_defs += 1;
                let full = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if full.ends_with("ThingDef_AlienRace") {
                    stats.alien_races += 1;
                } else if name.ends_with("ThingDef") {
                    if !is_empty {
                        *thing = Some(ThingInfo {
                            parent: attr(e, b"ParentName"),
                            is_abstract: attr(e, b"Abstract").eq_ignore_ascii_case("true"),
                            ..Default::default()
                        });
                    }
                } else {
                    match name.as_str() {
                        "FactionDef" => stats.factions += 1,
                        "BiomeDef" => stats.biomes += 1,
                        "StorytellerDef" => stats.storytellers += 1,
                        "XenotypeDef" => stats.xenotypes += 1,
                        _ => {}
                    }
                }
            }
            3 => {
                if let Some(t) = thing.as_mut() {
                    match name.as_str() {
                        "category" => *capture = Some("category"),
                        "thingClass" => *capture = Some("thingClass"),
                        "weaponTags" => t.has_weapon_tags = true,
                        "apparel" => t.has_apparel = true,
                        "building" => t.has_building = true,
                        "race" => {
                            t.has_race = true;
                            *in_race = !is_empty;
                        }
                        _ => {}
                    }
                }
            }
            4 if *in_race && name == "intelligence" => {
                *capture = Some("intelligence");
            }
            _ => {}
        }
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                on_open(e, depth, &mut thing, &mut capture, &mut in_race, stats, false);
            }
            Ok(Event::Empty(ref e)) => {
                on_open(e, depth + 1, &mut thing, &mut capture, &mut in_race, stats, true);
                capture = None;
            }
            Ok(Event::Text(ref e)) => {
                if let (Some(field), Some(t)) = (capture, thing.as_mut()) {
                    let text = e.unescape().map(|s| s.trim().to_string()).unwrap_or_default();
                    match field {
                        "category" => t.category = text,
                        "thingClass" => t.thing_class = text,
                        "intelligence" => t.intelligence = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(_)) => {
                capture = None;
                if depth == 3 {
                    in_race = false;
                }
                if depth == 2 {
                    if let Some(t) = thing.take() {
                        t.finish(stats);
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

fn xml_files_in(def_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in def_dirs {
        for entry in walkdir::WalkDir::new(dir).into_iter().flatten() {
            let p = entry.path();
            if entry.file_type().is_file()
                && p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("xml")).unwrap_or(false)
            {
                files.push(p.to_path_buf());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// "files:bytes:newest mtime" — changes whenever a Defs file is added, removed or edited.
fn fingerprint(files: &[PathBuf]) -> String {
    let mut size = 0u64;
    let mut newest = 0u64;
    for f in files {
        if let Ok(meta) = std::fs::metadata(f) {
            size += meta.len();
            if let Ok(m) = meta.modified() {
                if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                    newest = newest.max(d.as_secs());
                }
            }
        }
    }
    format!("{}:{}:{}", files.len(), size, newest)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefStatsCacheEntry {
    pub fingerprint: String,
    pub stats: DefStats,
}

pub type DefStatsCache = HashMap<String, DefStatsCacheEntry>;

pub fn cache_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("category_cache.json")
}

pub fn load_cache(path: &Path) -> DefStatsCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_cache(path: &Path, cache: &DefStatsCache) {
    if let Ok(text) = serde_json::to_string(cache) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Def counts for a mod, from the cache when its Defs files did not change.
/// Returns the stats and the cache entry to store.
pub fn def_stats_for(
    mod_dir: &Path,
    def_dirs: &[PathBuf],
    cache: &DefStatsCache,
) -> (DefStats, DefStatsCacheEntry) {
    let files = xml_files_in(def_dirs);
    let fp = fingerprint(&files);
    let key = mod_dir.to_string_lossy().to_string();
    if let Some(entry) = cache.get(&key) {
        if entry.fingerprint == fp {
            return (entry.stats.clone(), entry.clone());
        }
    }
    let mut stats = DefStats::default();
    for f in &files {
        if let Ok(bytes) = std::fs::read(f) {
            let text = String::from_utf8_lossy(&bytes);
            let mut file_stats = DefStats::default();
            parse_defs_xml(&text, &mut file_stats);
            stats.add(&file_stats);
        }
    }
    let entry = DefStatsCacheEntry { fingerprint: fp, stats: stats.clone() };
    (stats, entry)
}

// Categories
fn is_framework_name(name_lower: &str, id_lower: &str) -> bool {
    let words: Vec<&str> = name_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| matches!(*w, "framework" | "library" | "lib" | "core" | "api"))
        || id_lower.ends_with(".core")
        || id_lower.ends_with("lib")
        || id_lower.contains("framework")
        || id_lower.contains("library")
}

/// Categories of one mod. `dependents` = how many installed mods list it in modDependencies.
pub fn compute_categories(m: &ModInfo, dependents: usize) -> Vec<ModCategory> {
    let id_lower = m.package_id.to_lowercase();
    if id_lower.starts_with("ludeon.rimworld") {
        return Vec::new();
    }
    if m.is_translation_mod {
        return vec![ModCategory::Translation];
    }

    let s = &m.def_stats;
    let mut content: Vec<(ModCategory, u32)> = vec![
        (ModCategory::Race, s.alien_races + s.humanlike + s.xenotypes),
        (ModCategory::Faction, s.factions),
        (ModCategory::Building, s.buildings),
        (ModCategory::Weapon, s.weapons),
        (ModCategory::Apparel, s.apparel),
        (ModCategory::Animal, s.animals),
        (ModCategory::Biome, s.biomes),
        (ModCategory::Storyteller, s.storytellers),
    ];
    content.retain(|(_, n)| *n > 0);
    // Races, factions and biomes are the "headline" of a mod even with few defs
    // (a race mod also adds lots of apparel): they come first in this fixed order,
    // everything else follows by number of defs. A storyteller is usually a small extra
    // inside a faction/content mod (VFE Pirates, VPE), so it is ranked by count like the rest.
    let headline_rank = |cat: &ModCategory| match cat {
        ModCategory::Race => 0,
        ModCategory::Faction => 1,
        ModCategory::Biome => 2,
        _ => 3,
    };
    content.sort_by_key(|(cat, n)| (headline_rank(cat), std::cmp::Reverse(*n)));

    let mut cats: Vec<ModCategory> = Vec::new();
    let framework = m.has_assemblies
        && (dependents >= 2 || (is_framework_name(&m.name_lower, &id_lower) && s.total_defs < 40));
    if framework {
        cats.push(ModCategory::Framework);
    }
    cats.extend(content.iter().map(|(c, _)| *c));

    if cats.is_empty() {
        if m.has_assemblies {
            cats.push(ModCategory::QoL);
        } else if s.total_defs == 0 && m.has_patches {
            cats.push(ModCategory::Patch);
        } else if s.total_defs == 0 && m.has_textures {
            cats.push(ModCategory::Retexture);
        } else if m.has_patches {
            cats.push(ModCategory::Patch);
        }
    }
    cats
}

/// Fills `categories` for all mods; user overrides (by lowercase packageId) win.
pub fn assign_categories(mods: &mut [ModInfo], overrides: &HashMap<String, Vec<ModCategory>>) {
    let mut dependents: HashMap<String, usize> = HashMap::new();
    for m in mods.iter() {
        let mut seen = std::collections::HashSet::new();
        for d in &m.dependencies {
            let d = d.to_lowercase();
            if seen.insert(d.clone()) {
                *dependents.entry(d).or_insert(0) += 1;
            }
        }
    }
    for m in mods.iter_mut() {
        let key = m.package_id.to_lowercase();
        m.categories = match overrides.get(&key) {
            Some(user) => user.clone(),
            None => compute_categories(m, dependents.get(&key).copied().unwrap_or(0)),
        };
    }
}

// User overrides
pub fn overrides_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("category_overrides.json")
}

pub fn load_overrides(path: &Path) -> HashMap<String, Vec<ModCategory>> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_overrides(path: &Path, overrides: &HashMap<String, Vec<ModCategory>>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(overrides).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(xml: &str) -> DefStats {
        let mut s = DefStats::default();
        parse_defs_xml(xml, &mut s);
        s
    }

    #[test]
    fn counts_def_types() {
        let s = stats(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Defs>
  <AlienRace.ThingDef_AlienRace ParentName="BasePawn"><defName>Alien_Ratkin</defName></AlienRace.ThingDef_AlienRace>
  <ThingDef ParentName="BaseHumanMakeableGun"><defName>Gun_Laser</defName><weaponTags><li>Gun</li></weaponTags></ThingDef>
  <ThingDef ParentName="ApparelMakeableBase"><defName>Apparel_Hat</defName><apparel><layers><li>Overhead</li></layers></apparel></ThingDef>
  <ThingDef ParentName="FurnitureBase"><defName>Chair_Fancy</defName></ThingDef>
  <ThingDef><defName>Turret</defName><category>Building</category><thingClass>Building_TurretGun</thingClass></ThingDef>
  <ThingDef ParentName="AnimalThingBase"><defName>Dragon</defName><race><body>QuadrupedAnimal</body></race></ThingDef>
  <ThingDef><defName>Human2</defName><race><intelligence>Humanlike</intelligence></race></ThingDef>
  <ThingDef Name="MyAbstractBase" Abstract="True"><category>Building</category></ThingDef>
  <FactionDef><defName>Pirates2</defName></FactionDef>
  <BiomeDef><defName>Glacier</defName></BiomeDef>
  <StorytellerDef><defName>Teller</defName></StorytellerDef>
  <XenotypeDef><defName>Elf</defName></XenotypeDef>
  <RecipeDef><defName>Make_Thing</defName></RecipeDef>
</Defs>"#,
        );
        assert_eq!(s.total_defs, 13);
        assert_eq!(s.alien_races, 1);
        assert_eq!(s.weapons, 1);
        assert_eq!(s.apparel, 1);
        assert_eq!(s.buildings, 2, "furniture + turret, abstract base skipped");
        assert_eq!(s.animals, 1);
        assert_eq!(s.humanlike, 1);
        assert_eq!((s.factions, s.biomes, s.storytellers, s.xenotypes), (1, 1, 1, 1));
    }

    fn base_mod(id: &str, name: &str) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            ..Default::default()
        }
    }

    #[test]
    fn categories_from_content() {
        let mut race = base_mod("someone.ratkin", "Ratkin Race");
        race.has_assemblies = true;
        race.def_stats = DefStats { alien_races: 1, apparel: 20, weapons: 5, factions: 2, total_defs: 60, ..Default::default() };
        let cats = compute_categories(&race, 0);
        assert_eq!(cats[0], ModCategory::Race, "race is the headline even with fewer defs");
        assert!(cats.contains(&ModCategory::Faction) && cats.contains(&ModCategory::Apparel));

        let mut qol = base_mod("someone.pickup", "Pick Up And Haul");
        qol.has_assemblies = true;
        qol.def_stats.total_defs = 3;
        assert_eq!(compute_categories(&qol, 0), vec![ModCategory::QoL]);

        let mut lib = base_mod("unlimitedhugs.hugslib", "HugsLib");
        lib.has_assemblies = true;
        assert_eq!(compute_categories(&lib, 5), vec![ModCategory::Framework]);

        let mut patch = base_mod("someone.patch", "Some Patch");
        patch.has_patches = true;
        assert_eq!(compute_categories(&patch, 0), vec![ModCategory::Patch]);

        let mut retex = base_mod("someone.retex", "Pretty Walls");
        retex.has_textures = true;
        assert_eq!(compute_categories(&retex, 0), vec![ModCategory::Retexture]);

        let core = base_mod("Ludeon.RimWorld", "Core");
        assert!(compute_categories(&core, 50).is_empty());
    }

    #[test]
    fn overrides_win() {
        let mut mods = vec![base_mod("a.b", "Thing")];
        mods[0].has_assemblies = true;
        let mut ov = HashMap::new();
        ov.insert("a.b".to_string(), vec![ModCategory::Weapon]);
        assign_categories(&mut mods, &ov);
        assert_eq!(mods[0].categories, vec![ModCategory::Weapon]);
    }
}
