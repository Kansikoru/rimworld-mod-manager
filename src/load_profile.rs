//! Loading time of each mod, from the game's own profiler. With "Verbose logging" on
//! (`<logVerbose>` in Prefs.xml) RimWorld times every loading step and writes the tree to
//! Player.log when loading ends:
//!
//! ```text
//!  72580.453ms (self: 3.928 ms) LoadAllPlayData
//!  - 59099.407ms (81%) (self: 810.792 ms) Load all active mods.
//!  - - 16050.840ms (27%) (self: 1.583 ms) LoadModContent()
//!  - - - 1399.883ms (8.7%) (self: 1399.883 ms) Loading OskarPotocki.VanillaFactionsExpanded.Core content
//!  - - 14988.453ms (25%) (self: 13.594 ms) CreateModClasses()
//!  - - - 3225.405ms (22%) (self: 3225.332 ms) Loading Kindred.KindredMod mod class
//!  - - 11811.996ms (20%) (self: 0.524 ms) LoadModXML()
//!  - - - 1913.753ms (16%) (self: 0.033 ms) Loading Ludeon.RimWorld
//!  - - 3398.588ms (5.8%) (self: 3140.696 ms) InitializeMods()
//!  - - - 22.904ms (0.67%) (self: 22.904 ms) Initializing [zal.wmbp|World Map Beautification Project (Continued)]
//! ```
//!
//! Four steps are split by mod: its files (textures, sounds, assemblies), its `Mod` classes,
//! its defs, its initialization. The rest (one pass over all defs, language data, shaders…)
//! is shared by all mods.

/// What the profiler says about one mod; times in milliseconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModLoad {
    /// Package ID as the game wrote it; for a class not matched to a mod, the class name.
    pub id: String,
    pub content_ms: f64,
    pub classes_ms: f64,
    pub defs_ms: f64,
    pub init_ms: f64,
    /// `Mod` classes counted in `classes_ms` ("Kindred.KindredMod").
    pub classes: Vec<String>,
    /// A `Mod` class no installed mod's assemblies could be matched to.
    pub unmatched_class: bool,
}

impl ModLoad {
    pub fn total_ms(&self) -> f64 {
        self.content_ms + self.classes_ms + self.defs_ms + self.init_ms
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadProfile {
    /// `LoadAllPlayData`: the whole loading the profiler saw.
    pub total_ms: f64,
    /// The slowest first.
    pub mods: Vec<ModLoad>,
}

impl LoadProfile {
    /// What no single mod is behind: one pass over all defs, language data, shaders…
    pub fn shared_ms(&self) -> f64 {
        (self.total_ms - self.mods.iter().map(ModLoad::total_ms).sum::<f64>()).max(0.0)
    }
}

/// One line of the tree: depth (number of "- "), total time, label.
fn tree_line(line: &str) -> Option<(usize, f64, &str)> {
    let rest = line.strip_prefix(' ')?;
    let mut depth = 0;
    let mut rest = rest;
    while let Some(r) = rest.strip_prefix("- ") {
        depth += 1;
        rest = r;
    }
    let ms_end = rest.find("ms ")?;
    let ms: f64 = rest[..ms_end].replace(',', ".").parse().ok()?;
    let after = &rest[ms_end + 3..];
    // "(81%) (self: 810.792 ms) Label" or "(self: 3.928 ms) Label".
    let self_at = after.find("(self: ")?;
    let label_at = after[self_at..].find(") ")? + self_at + 2;
    Some((depth, ms, &after[label_at..]))
}

/// Which step of "Load all active mods." a line belongs to.
#[derive(Clone, Copy, PartialEq)]
enum Step {
    Content,
    Classes,
    Defs,
    Init,
    Other,
}

/// The last loading tree in the log (a log holds one per game start), if verbose logging
/// was on. Mod classes are listed by class name: `assign_classes` puts them on their mods.
pub fn parse(log: &str) -> Option<LoadProfile> {
    let lines: Vec<&str> = log.lines().collect();
    let start = lines
        .iter()
        .rposition(|l| tree_line(l).is_some_and(|(depth, _, label)| depth == 0 && label == "LoadAllPlayData"))?;
    let (_, total_ms, _) = tree_line(lines[start])?;
    let mut mods: Vec<ModLoad> = Vec::new();
    let mut step = Step::Other;
    for line in &lines[start + 1..] {
        let Some((depth, ms, label)) = tree_line(line) else {
            break;
        };
        match depth {
            0 => break,
            2 => {
                step = match label {
                    "LoadModContent()" => Step::Content,
                    "CreateModClasses()" => Step::Classes,
                    "LoadModXML()" => Step::Defs,
                    "InitializeMods()" => Step::Init,
                    _ => Step::Other,
                }
            }
            3 => {
                let key = match step {
                    Step::Content => label.strip_prefix("Loading ").and_then(|s| s.strip_suffix(" content")),
                    Step::Classes => label
                        .strip_prefix("Loading ")
                        .and_then(|s| s.strip_suffix(" mod class")),
                    Step::Defs => label.strip_prefix("Loading "),
                    // "Initializing [zal.wmbp|World Map Beautification Project (Continued)]"
                    Step::Init => label.strip_prefix("Initializing [").and_then(|s| s.split('|').next()),
                    Step::Other => None,
                };
                let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) else {
                    continue;
                };
                if step == Step::Classes {
                    mods.push(ModLoad {
                        id: key.to_string(),
                        classes_ms: ms,
                        classes: vec![key.to_string()],
                        unmatched_class: true,
                        ..Default::default()
                    });
                    continue;
                }
                let entry = match mods
                    .iter_mut()
                    .position(|m| !m.unmatched_class && m.id.eq_ignore_ascii_case(key))
                {
                    Some(i) => &mut mods[i],
                    None => {
                        mods.push(ModLoad {
                            id: key.to_string(),
                            ..Default::default()
                        });
                        mods.last_mut().expect("just pushed")
                    }
                };
                match step {
                    Step::Content => entry.content_ms += ms,
                    Step::Defs => entry.defs_ms += ms,
                    Step::Init => entry.init_ms += ms,
                    Step::Classes | Step::Other => {}
                }
            }
            _ => {}
        }
    }
    let mut profile = LoadProfile { total_ms, mods };
    profile.sort();
    Some(profile)
}

impl LoadProfile {
    fn sort(&mut self) {
        self.mods.sort_by(|a, b| b.total_ms().total_cmp(&a.total_ms()));
    }

    /// Puts each `Mod` class on the mod whose assembly it comes from. The log names only the
    /// class ("Kindred.KindredMod"), so it is matched by namespace against the mods' assembly
    /// file names (`assemblies`: package ID -> dll names without ".dll"): the whole namespace,
    /// then its first part, then a dll name that starts with it. A class no mod matches stays
    /// on its own row.
    pub fn assign_classes(&mut self, assemblies: &[(String, Vec<String>)]) {
        let classes: Vec<ModLoad> = self.mods.iter().filter(|m| m.unmatched_class).cloned().collect();
        self.mods.retain(|m| !m.unmatched_class);
        for class in classes {
            let name = &class.id;
            let namespace = name.rsplit_once('.').map_or("", |(ns, _)| ns);
            let first = namespace.split('.').next().unwrap_or("");
            let owner = [
                |dll: &str, ns: &str, _: &str| !ns.is_empty() && dll.eq_ignore_ascii_case(ns),
                |dll: &str, _: &str, first: &str| !first.is_empty() && dll.eq_ignore_ascii_case(first),
                |dll: &str, _: &str, first: &str| {
                    first.len() >= 3 && dll.to_lowercase().starts_with(&first.to_lowercase())
                },
            ]
            .iter()
            .find_map(|rule| {
                assemblies
                    .iter()
                    .find(|(_, dlls)| dlls.iter().any(|d| rule(d, namespace, first)))
                    .map(|(id, _)| id.clone())
            });
            match owner {
                Some(id) => {
                    let entry = match self.mods.iter_mut().position(|m| m.id.eq_ignore_ascii_case(&id)) {
                        Some(i) => &mut self.mods[i],
                        None => {
                            self.mods.push(ModLoad {
                                id,
                                ..Default::default()
                            });
                            self.mods.last_mut().expect("just pushed")
                        }
                    };
                    entry.classes_ms += class.classes_ms;
                    entry.classes.extend(class.classes);
                }
                None => self.mods.push(class),
            }
        }
        self.sort();
    }
}

/// `<logVerbose>` set to `on` in a Prefs.xml text: `None` when it already is (or the file has
/// no `PrefsData`). A missing tag is added.
fn prefs_with_log_verbose(xml: &str, on: bool) -> Option<String> {
    const OPEN: &str = "<logVerbose>";
    const CLOSE: &str = "</logVerbose>";
    let value = if on { "True" } else { "False" };
    if let Some(start) = xml.find(OPEN) {
        let inner = start + OPEN.len();
        let len = xml[inner..].find(CLOSE)?;
        let current = xml[inner..inner + len].trim().eq_ignore_ascii_case("true");
        if current == on {
            return None;
        }
        return Some(format!("{}{value}{}", &xml[..inner], &xml[inner + len..]));
    }
    let end = xml.rfind("</PrefsData>")?;
    Some(format!("{}  {OPEN}{value}{CLOSE}\n{}", &xml[..end], &xml[end..]))
}

/// Sets the game's "Verbose logging" in `<config dir>/Prefs.xml`; true when it changed.
/// The rest of the file (and its byte order mark) stays as it was.
fn set_log_verbose(config_dir: &std::path::Path, on: bool) -> Result<bool, String> {
    let path = config_dir.join("Prefs.xml");
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let (bom, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (true, rest),
        None => (false, &bytes[..]),
    };
    let text = String::from_utf8_lossy(body);
    let Some(new_text) = prefs_with_log_verbose(&text, on) else {
        return Ok(false);
    };
    let mut out = Vec::with_capacity(new_text.len() + 3);
    if bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(new_text.as_bytes());
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Left by `start_measured_launch` when it turned verbose logging on: holds the config folder,
/// so `finish_measured_launch` turns it off again (even after the manager was restarted).
fn verbose_marker_path() -> std::path::PathBuf {
    crate::game_install::manager_data_dir().join("verbose_log_for_load_profile.txt")
}

/// Before a measured launch: verbose logging on for this one game start. If the player had it
/// on already, it is left alone and stays on afterwards.
pub fn start_measured_launch(config_dir: &std::path::Path) -> Result<(), String> {
    if set_log_verbose(config_dir, true)? {
        std::fs::write(verbose_marker_path(), config_dir.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// After the game closed (or on the manager's start, when the game isn't running): verbose
/// logging off again, if the manager turned it on. The game writes Prefs.xml on exit, so this
/// must not run while it is open.
pub fn finish_measured_launch() {
    let marker = verbose_marker_path();
    let Ok(dir) = std::fs::read_to_string(&marker) else {
        return;
    };
    if set_log_verbose(std::path::Path::new(dir.trim()), false).is_ok() {
        let _ = std::fs::remove_file(marker);
    }
}

/// Whether a measured launch is under way (verbose logging turned on by the manager).
pub fn measured_launch_pending() -> bool {
    verbose_marker_path().exists()
}

/// Whether a side is a power of two (1, 2, 4 … 4096).
pub fn is_power_of_two(side: u32) -> bool {
    side.is_power_of_two()
}

/// Assembly file names (without ".dll") of each mod, from the folders the game loads for
/// this version: what `LoadProfile::assign_classes` matches `Mod` classes against.
pub fn mod_assemblies(mods: &[crate::models::ModInfo], game_version: Option<&str>) -> Vec<(String, Vec<String>)> {
    mods.iter()
        .filter_map(|m| {
            let mut dlls: Vec<String> = Vec::new();
            for folder in crate::optimizer::mod_load_folders(&m.dir_path, game_version) {
                let Ok(entries) = std::fs::read_dir(&folder) else {
                    continue;
                };
                let Some(asm_dir) = entries
                    .flatten()
                    .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("assemblies"))
                else {
                    continue;
                };
                for f in std::fs::read_dir(asm_dir.path()).into_iter().flatten().flatten() {
                    let path = f.path();
                    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            dlls.push(stem.to_string());
                        }
                    }
                }
            }
            (!dlls.is_empty()).then(|| (m.package_id.clone(), dlls))
        })
        .collect()
}

/// "1.7" from the log's "RimWorld 1.7.4995 rev309" line (1.6: "RimWorld 1.6.4633 rev1261").
pub fn game_version_in_log(log: &str) -> Option<String> {
    let at = log.find("RimWorld 1.")? + "RimWorld ".len();
    let rest = &log[at..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    crate::optimizer::short_game_version(&rest[..end])
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn game_version_comes_from_the_log() {
        assert_eq!(
            super::game_version_in_log("2026-11-02 23:48:20.232  INFO [Verse] RimWorld 1.7.4995 rev309\n").as_deref(),
            Some("1.7")
        );
        assert_eq!(super::game_version_in_log("no version here"), None);
    }

    #[test]
    fn verbose_logging_is_switched_in_prefs() {
        let prefs = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<PrefsData>\n  <devMode>True</devMode>\n  <logVerbose>False</logVerbose>\n</PrefsData>\n";
        let on = super::prefs_with_log_verbose(prefs, true).unwrap();
        assert!(on.contains("<logVerbose>True</logVerbose>") && on.contains("<devMode>True</devMode>"));
        assert_eq!(on.len(), prefs.len() - 1);
        // Already as wanted: nothing to write.
        assert_eq!(super::prefs_with_log_verbose(&on, true), None);
        assert_eq!(super::prefs_with_log_verbose(&on, false).unwrap(), prefs);
        // An old Prefs.xml without the tag gets it.
        let added = super::prefs_with_log_verbose("<PrefsData>\n</PrefsData>", true).unwrap();
        assert!(added.contains("<logVerbose>True</logVerbose>\n</PrefsData>"));
        assert_eq!(super::prefs_with_log_verbose("not prefs", true), None);

        // On disk the byte order mark survives.
        let dir = std::env::temp_dir().join(format!("rwmm_verbose_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(prefs.as_bytes());
        std::fs::write(dir.join("Prefs.xml"), &bytes).unwrap();
        assert!(super::set_log_verbose(&dir, true).unwrap());
        assert!(!super::set_log_verbose(&dir, true).unwrap());
        let written = std::fs::read(dir.join("Prefs.xml")).unwrap();
        assert!(written.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert!(String::from_utf8_lossy(&written).contains("<logVerbose>True</logVerbose>"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn odd_sides_are_counted_and_cached() {
        let dir = std::env::temp_dir().join(format!("rwmm_odd_{}", std::process::id()));
        let tex = dir.join("Textures");
        std::fs::create_dir_all(&tex).unwrap();
        image::RgbaImage::new(24, 24).save(tex.join("icon.png")).unwrap();
        image::RgbaImage::new(64, 128).save(tex.join("wall.png")).unwrap();
        // Replaced by its DDS: the game doesn't load the PNG.
        image::RgbaImage::new(100, 100).save(tex.join("rug.png")).unwrap();
        std::fs::write(tex.join("rug.dds"), b"DDS ").unwrap();
        assert_eq!(crate::optimizer::non_power_of_two_textures(&dir, Some("1.7")), (1, 2));

        let mods = vec![("a.mod".to_string(), dir.clone())];
        let (counts, fresh) = crate::optimizer::non_power_of_two_by_mod(&mods, Some("1.7"), &Default::default());
        assert_eq!(counts["a.mod"], (1, 2));
        assert_eq!(fresh.len(), 1);
        let cache: std::collections::HashMap<_, _> =
            fresh.into_iter().map(|(d, fp, odd, all)| (d, (fp, odd, all))).collect();
        let file = dir.join("cache.tsv");
        crate::optimizer::write_texture_sides_cache(&file, &cache).unwrap();
        let read = crate::optimizer::read_texture_sides_cache(&file);
        assert_eq!(read, cache);
        // Unchanged files: taken from the cache, nothing new to store.
        let (counts, fresh) = crate::optimizer::non_power_of_two_by_mod(&mods, Some("1.7"), &read);
        assert_eq!((counts["a.mod"], fresh.len()), ((1, 2), 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The installed game's last Player.log, if it was written with verbose logging.
    #[test]
    #[ignore]
    fn the_real_log_is_read() {
        let log = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap())
            .join(r"AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Logs\Player.log");
        let text = String::from_utf8_lossy(&std::fs::read(log).unwrap()).to_string();
        let p = super::parse(&text).expect("no tree: was verbose logging on?");
        println!(
            "total {:.0} ms, shared {:.0} ms, {} rows",
            p.total_ms,
            p.shared_ms(),
            p.mods.len()
        );
        for m in p.mods.iter().take(8) {
            println!("{:8.0}  {}", m.total_ms(), m.id);
        }
    }

    /// The count on the installed mods, against the 3792 warnings of a verbose 1.7 log.
    #[test]
    #[ignore]
    fn non_power_of_two_on_installed_mods() {
        let dir = std::path::Path::new(r"F:\SteamLibrary\steamapps\workshop\content\294100");
        let mut total = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let (odd, all) = crate::optimizer::non_power_of_two_textures(&e.path(), Some("1.7"));
            if odd > 300 {
                println!("{odd:5} / {all:5}  {}", e.path().display());
            }
            total += odd;
        }
        println!("total {total}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top of a real tree (RimWorld 1.7.4995, 133 mods), cut to a few lines per step.
    const SAMPLE: &str = include_str!("../tests/data/load_profile_1.7.txt");

    #[test]
    fn the_games_tree_is_read() {
        let p = parse(SAMPLE).expect("a tree");
        assert!((p.total_ms - 72580.453).abs() < 0.01);
        let vfe = p
            .mods
            .iter()
            .find(|m| m.id == "OskarPotocki.VanillaFactionsExpanded.Core")
            .unwrap();
        assert!((vfe.content_ms - 1399.883).abs() < 0.01);
        assert!(vfe.defs_ms > 0.0);
        let wmbp = p.mods.iter().find(|m| m.id == "zal.wmbp").unwrap();
        assert!((wmbp.init_ms - 22.904).abs() < 0.01);
        // Mod classes are kept by class name until they are put on their mods.
        let kindred_class = p.mods.iter().find(|m| m.id == "Kindred.KindredMod").unwrap();
        assert!(kindred_class.unmatched_class && (kindred_class.classes_ms - 3225.405).abs() < 0.01);
        // Sorted, the slowest first; the shared steps are what is left.
        assert!(p.mods.windows(2).all(|w| w[0].total_ms() >= w[1].total_ms()));
        assert!(p.shared_ms() > 0.0 && p.shared_ms() < p.total_ms);
    }

    #[test]
    fn classes_go_to_the_mod_of_their_assembly() {
        let mut p = parse(SAMPLE).unwrap();
        let assemblies = vec![
            ("tanyavonhart.kindred".to_string(), vec!["Kindred".to_string()]),
            (
                "OskarPotocki.VanillaFactionsExpanded.Core".to_string(),
                vec!["VEF".to_string()],
            ),
            ("void.charactereditor".to_string(), vec!["CharacterEditor".to_string()]),
        ];
        p.assign_classes(&assemblies);
        let kindred = p.mods.iter().find(|m| m.id == "tanyavonhart.kindred").unwrap();
        assert_eq!(kindred.classes, ["Kindred.KindredMod"]);
        assert!(kindred.content_ms > 0.0 && kindred.classes_ms > 3000.0);
        // Kindred is the slowest once its code counts.
        assert_eq!(p.mods[0].id, "tanyavonhart.kindred");
        let vef = p
            .mods
            .iter()
            .find(|m| m.id == "OskarPotocki.VanillaFactionsExpanded.Core")
            .unwrap();
        assert_eq!(vef.classes, ["VEF.VEF_Mod"]);
        // A class without a matching assembly stays on its own row.
        assert!(p.mods.iter().any(|m| m.unmatched_class && m.id == "Rimesis.RimesisMod"));
        assert!(!p.mods.iter().any(|m| m.id == "Kindred.KindredMod"));
    }

    #[test]
    fn no_tree_without_verbose_logging() {
        assert_eq!(
            parse("2026-11-02 23:48:20.232  INFO [Verse] RimWorld 1.7.4995 rev309\n"),
            None
        );
        // A broken line ends the tree instead of failing it.
        let p = parse(" 100.000ms (self: 1.000 ms) LoadAllPlayData\n - 50.000ms (50%) (self: 1.000 ms) Load all active mods.\n - - 40.000ms (40%) (self: 0.000 ms) LoadModXML()\n - - - 30.000ms (30%) (self: 0.000 ms) Loading a.b\nnot a tree line\n - - - 9.000ms (9%) (self: 0.000 ms) Loading c.d\n").unwrap();
        assert_eq!(p.mods.len(), 1);
        assert_eq!((p.mods[0].defs_ms, p.shared_ms()), (30.0, 70.0));
        assert!(is_power_of_two(256) && !is_power_of_two(24) && !is_power_of_two(0));
    }
}
