//! The companion mod's report: what only the game knows, written by the mod after loading
//! (`companion/Source/CompanionReport.cs`): video memory of each mod's textures (1.6), defs one
//! mod replaced in another, and game methods several mods patch with Harmony.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The report format this manager reads (`CompanionReport.Format`).
const FORMAT: u32 = 1;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CompanionReport {
    pub format: u32,
    pub game: Option<String>,
    pub written_unix: i64,
    pub load_secs: Option<f64>,
    /// The running mods in load order; other parts refer to them by index. A part the mod
    /// failed to collect is null.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub mods: Vec<ReportMod>,
    /// `None` on 1.7: the game uploads and evicts textures itself.
    pub textures: Option<Textures>,
    pub def_overrides: Option<Vec<DefOverride>>,
    /// `None` without Harmony.
    pub harmony: Option<Harmony>,
    /// Parts of the report the mod could not collect.
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportMod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub official: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Textures {
    /// All textures in memory: the game's, the mods' and the atlases.
    pub all_bytes: u64,
    pub all_count: u64,
    /// What the engine itself counts (`Texture.totalTextureMemory`).
    pub engine_bytes: u64,
    pub vram_mb: u64,
    #[serde(default)]
    pub mods: Vec<ModTextures>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModTextures {
    #[serde(rename = "mod")]
    pub mod_index: usize,
    pub count: u64,
    pub bytes: u64,
    /// Textures the game left uncompressed (sides not a multiple of 4, or compression off).
    pub uncompressed: u64,
    pub uncompressed_bytes: u64,
    /// Textures that keep a copy in RAM as well.
    pub readable_bytes: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DefOverride {
    #[serde(rename = "type")]
    pub def_type: String,
    pub def: String,
    /// The mods that define it, in load order.
    pub mods: Vec<usize>,
    /// The mod whose def the game kept (-1: none of them).
    pub winner: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Harmony {
    pub version: Option<String>,
    pub patched_methods: u64,
    /// Methods patched by two or more mods.
    #[serde(default)]
    pub shared: Vec<SharedMethod>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SharedMethod {
    pub method: String,
    pub patches: Vec<PatchRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatchRef {
    /// -1: the patch comes from no mod's assemblies (then `owner` names it).
    #[serde(rename = "mod")]
    pub mod_index: i64,
    pub owner: Option<String>,
    /// prefix, postfix, transpiler or finalizer.
    pub kind: String,
    /// A prefix that can stop the original method and the patches after it.
    #[serde(default)]
    pub skips: bool,
}

/// Defs of one mod that another mod replaced.
#[derive(Debug, Clone, PartialEq)]
pub struct OverridePair {
    pub winner: usize,
    pub loser: usize,
    /// "ThingDef Steel".
    pub defs: Vec<String>,
}

/// One mod's patches on a shared method.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodPatcher {
    pub name: String,
    /// prefix, postfix… once each.
    pub kinds: Vec<String>,
    pub skips: bool,
}

/// How likely the patches on one method are to break each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PatchRisk {
    /// Prefixes and postfixes that only add to the method.
    Low,
    /// One mod rewrites the method's code (transpiler) or can cancel it (a prefix returning
    /// false).
    Some,
    /// Two or more mods rewrite the same code.
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SharedMethodSummary {
    pub method: String,
    pub patchers: Vec<MethodPatcher>,
    pub risk: PatchRisk,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

/// "RimWorld.Pawn_HealthTracker.PostApplyDamage(DamageInfo)" -> "Pawn_HealthTracker.PostApplyDamage(DamageInfo)".
pub fn short_method(method: &str) -> &str {
    let name_end = method.find('(').unwrap_or(method.len());
    let mut dots = method[..name_end].rmatch_indices('.');
    match (dots.next(), dots.next()) {
        (Some(_), Some((type_dot, _))) => &method[type_dot + 1..],
        _ => method,
    }
}

pub fn path() -> PathBuf {
    crate::game_install::manager_data_dir().join("companion_report.json")
}

/// The last report, if there is one this manager can read.
pub fn load() -> Option<CompanionReport> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Option<CompanionReport> {
    let text = std::fs::read_to_string(path).ok()?;
    let report: CompanionReport = serde_json::from_str(&text).ok()?;
    (report.format == FORMAT).then_some(report)
}

impl CompanionReport {
    pub fn mod_name(&self, index: usize) -> &str {
        self.mods.get(index).map_or("?", |m| m.name.as_str())
    }

    fn is_official(&self, index: usize) -> bool {
        self.mods.get(index).is_some_and(|m| m.official)
    }

    /// The mod list changed since the report: its numbers describe another set.
    pub fn mods_differ(&self, active_ids: &[String]) -> bool {
        let report: HashSet<String> = self.mods.iter().map(|m| m.id.to_lowercase()).collect();
        let active: HashSet<String> = active_ids.iter().map(|id| id.to_lowercase()).collect();
        report != active
    }

    /// Mods by texture memory, the heaviest first.
    pub fn heaviest_mods(&self) -> Vec<&ModTextures> {
        let mut mods: Vec<&ModTextures> = self.textures.iter().flat_map(|t| &t.mods).collect();
        mods.sort_by_key(|m| std::cmp::Reverse(m.bytes));
        mods
    }

    /// Who replaced whose defs, split into (between mods, over the game and DLCs); the pairs
    /// with the most defs first. Official content replacing official content is left out.
    pub fn override_pairs(&self) -> (Vec<OverridePair>, Vec<OverridePair>) {
        let mut pairs: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
        for o in self.def_overrides.iter().flatten() {
            let Ok(winner) = usize::try_from(o.winner) else {
                continue;
            };
            for &loser in o.mods.iter().filter(|&&m| m != winner) {
                if self.is_official(winner) && self.is_official(loser) {
                    continue;
                }
                pairs
                    .entry((winner, loser))
                    .or_default()
                    .push(format!("{} {}", o.def_type, o.def));
            }
        }
        let (mut over_game, mut between): (Vec<_>, Vec<_>) = pairs
            .into_iter()
            .map(|((winner, loser), defs)| OverridePair { winner, loser, defs })
            .partition(|p| self.is_official(p.loser));
        for list in [&mut between, &mut over_game] {
            list.sort_by_key(|p| std::cmp::Reverse(p.defs.len()));
        }
        (between, over_game)
    }

    /// The estimate before "Play" replaced by what the game really took (1.6, where the mod
    /// measures textures): when the report is of the same mods and no texture was rewritten
    /// since (by the optimizer, an update). All textures count, the game's own too: that is
    /// what fills the video memory. `None` keeps the estimate.
    pub fn measured_vram(
        &self,
        estimate: &crate::optimizer::VramReport,
        active_ids: &[String],
    ) -> Option<crate::optimizer::VramReport> {
        let textures = self.textures.as_ref()?;
        let written = u64::try_from(self.written_unix).ok()?;
        if self.mods_differ(active_ids) || estimate.newest_texture_unix > written || textures.all_bytes == 0 {
            return None;
        }
        let mut heavy_mods: Vec<(String, u64)> = self
            .heaviest_mods()
            .into_iter()
            .filter(|m| !self.is_official(m.mod_index) && m.bytes >= crate::optimizer::HEAVY_MOD_BYTES)
            .map(|m| (self.mod_name(m.mod_index).to_string(), m.bytes))
            .collect();
        heavy_mods.truncate(crate::optimizer::HEAVY_MODS_SHOWN);
        Some(crate::optimizer::VramReport {
            estimated_used_bytes: textures.all_bytes,
            heavy_mods,
            measured: true,
            ..estimate.clone()
        })
    }

    /// Package IDs of a pair's mods: (winner, loser).
    pub fn pair_ids(&self, pair: &OverridePair) -> (&str, &str) {
        let id = |i: usize| self.mods.get(i).map_or("", |m| m.id.as_str());
        (id(pair.winner), id(pair.loser))
    }

    /// Methods patched by several mods: the riskiest first, then by how many mods.
    pub fn shared_methods(&self) -> Vec<SharedMethodSummary> {
        let mut out: Vec<SharedMethodSummary> = self
            .harmony
            .iter()
            .flat_map(|h| &h.shared)
            .map(|m| {
                let mut patchers: Vec<MethodPatcher> = Vec::new();
                for p in &m.patches {
                    let name = match usize::try_from(p.mod_index) {
                        Ok(i) => self.mod_name(i).to_string(),
                        Err(_) => p.owner.clone().unwrap_or_else(|| "?".to_string()),
                    };
                    let entry = match patchers.iter_mut().position(|x| x.name == name) {
                        Some(i) => &mut patchers[i],
                        None => {
                            patchers.push(MethodPatcher {
                                name,
                                kinds: Vec::new(),
                                skips: false,
                            });
                            patchers.last_mut().expect("just pushed")
                        }
                    };
                    if !entry.kinds.contains(&p.kind) {
                        entry.kinds.push(p.kind.clone());
                    }
                    entry.skips |= p.skips;
                }
                let rewriters = patchers
                    .iter()
                    .filter(|p| p.kinds.iter().any(|k| k == "transpiler"))
                    .count();
                let risk = if rewriters >= 2 {
                    PatchRisk::High
                } else if rewriters == 1 || patchers.iter().any(|p| p.skips) {
                    PatchRisk::Some
                } else {
                    PatchRisk::Low
                };
                SharedMethodSummary {
                    method: m.method.clone(),
                    patchers,
                    risk,
                }
            })
            .collect();
        out.sort_by_key(|m| (std::cmp::Reverse(m.risk), std::cmp::Reverse(m.patchers.len())));
        out
    }
}

/// Whether one mod of a pair names the other in its About.xml (a dependency, load after or
/// load before it): an add-on or a patch that replaces those defs on purpose, like UNAGI
/// Facial Animation over [NL] Facial Animation. `a` / `b`: the mods as scanned, if installed.
pub fn pair_is_declared(
    a: Option<&crate::models::ModInfo>,
    a_id: &str,
    b: Option<&crate::models::ModInfo>,
    b_id: &str,
) -> bool {
    fn names(m: &crate::models::ModInfo, other: &str) -> bool {
        [
            &m.dependencies,
            &m.load_after,
            &m.force_load_after,
            &m.load_before,
            &m.force_load_before,
        ]
        .iter()
        .any(|list| list.iter().any(|id| id.eq_ignore_ascii_case(other)))
    }
    a.is_some_and(|m| names(m, b_id)) || b.is_some_and(|m| names(m, a_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written like the mod writes it (JsonWriter: no spaces).
    const SAMPLE: &str = r#"{"format":1,"game":"1.6.4871 rev590","written_unix":1793570000,"load_secs":133.34,
"mods":[{"id":"ludeon.rimworld","name":"Core","official":true},{"id":"ludeon.rimworld.royalty","name":"Royalty","official":true},
{"id":"a.retex","name":"Retex","official":false},{"id":"b.big","name":"Big Mod","official":false},{"id":"c.other","name":"Other","official":false}],
"textures":{"all_bytes":3000000000,"all_count":40000,"engine_bytes":3100000000,"vram_mb":16253,"mods":[
{"mod":0,"count":5000,"bytes":400000000,"uncompressed":10,"uncompressed_bytes":1000000,"readable_bytes":0},
{"mod":3,"count":9000,"bytes":900000000,"uncompressed":300,"uncompressed_bytes":200000000,"readable_bytes":5000000}]},
"def_overrides":[
{"type":"ThingDef","def":"Steel","mods":[0,2],"winner":2},
{"type":"ThingDef","def":"Gun_Revolver","mods":[0,3,4],"winner":4},
{"type":"ThoughtDef","def":"AteRaw","mods":[3,4],"winner":4},
{"type":"ThingDef","def":"Plasteel","mods":[0,1],"winner":1},
{"type":"ThingDef","def":"Gone","mods":[2,3],"winner":-1}],
"harmony":{"version":"2.4.3.0","shared":[
{"method":"RimWorld.Pawn.Tick()","patches":[{"mod":3,"owner":"b","kind":"postfix","skips":false},{"mod":4,"owner":"c","kind":"postfix","skips":false}]},
{"method":"Verse.Thing.Draw()","patches":[{"mod":3,"owner":"b","kind":"prefix","skips":true},{"mod":3,"owner":"b","kind":"postfix","skips":false},{"mod":-1,"owner":"some.id","kind":"transpiler","skips":false}]},
{"method":"Verse.Map.MapUpdate()","patches":[{"mod":3,"owner":"b","kind":"transpiler","skips":false},{"mod":4,"owner":"c","kind":"transpiler","skips":false}]}],
"patched_methods":1234},"errors":[],"report_ms":321}"#;

    fn sample() -> CompanionReport {
        serde_json::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn the_mods_report_is_read() {
        let r = sample();
        assert_eq!(r.game.as_deref(), Some("1.6.4871 rev590"));
        assert_eq!(r.mods.len(), 5);
        assert_eq!(r.mod_name(3), "Big Mod");
        assert_eq!(r.mod_name(99), "?");
        let heaviest = r.heaviest_mods();
        assert_eq!(heaviest[0].mod_index, 3);
        assert_eq!(heaviest[0].uncompressed_bytes, 200_000_000);

        // 1.7: no textures, failed parts as null.
        let r: CompanionReport = serde_json::from_str(
            r#"{"format":1,"game":null,"written_unix":1,"load_secs":null,"mods":[],"textures":null,
               "def_overrides":null,"harmony":null,"errors":["game: MissingMethodException: x"],"report_ms":5}"#,
        )
        .unwrap();
        assert!(r.textures.is_none() && r.heaviest_mods().is_empty());
        assert_eq!(r.override_pairs(), (vec![], vec![]));
        assert!(r.shared_methods().is_empty());
        assert_eq!(r.errors.len(), 1);
    }

    #[test]
    fn overrides_are_paired_and_split() {
        let (between, over_game) = sample().override_pairs();
        // Other replaced Big Mod's defs twice; DLC over Core and a def nobody kept are left out.
        assert_eq!(
            between,
            vec![OverridePair {
                winner: 4,
                loser: 3,
                defs: vec!["ThingDef Gun_Revolver".into(), "ThoughtDef AteRaw".into()],
            }]
        );
        assert_eq!(over_game.len(), 2);
        assert!(over_game.iter().all(|p| p.loser == 0));
        assert!(over_game.iter().any(|p| p.winner == 2 && p.defs == ["ThingDef Steel"]));
    }

    #[test]
    fn the_measured_vram_replaces_the_estimate_when_it_still_fits() {
        let r = sample();
        let ids: Vec<String> = r.mods.iter().map(|m| m.id.clone()).collect();
        let estimate = crate::optimizer::VramReport {
            total_vram_bytes: 8 * 1024 * 1024 * 1024,
            estimated_used_bytes: 1,
            heavy_mods: vec![("Guess".into(), 1)],
            newest_texture_unix: 1_793_560_000,
            measured: false,
        };
        let m = r.measured_vram(&estimate, &ids).expect("same mods, no newer texture");
        assert!(m.measured);
        assert_eq!(m.estimated_used_bytes, 3_000_000_000);
        assert_eq!(m.total_vram_bytes, estimate.total_vram_bytes);
        // Core is the game's own: not among the mods to turn off.
        assert_eq!(m.heavy_mods, vec![("Big Mod".to_string(), 900_000_000)]);

        // Another mod list, a texture written after the game run, or 1.7 (no textures): the estimate.
        assert!(r.measured_vram(&estimate, &ids[1..]).is_none());
        let newer = crate::optimizer::VramReport {
            newest_texture_unix: 1_793_570_001,
            ..estimate.clone()
        };
        assert!(r.measured_vram(&newer, &ids).is_none());
        let on_17 = CompanionReport {
            textures: None,
            ..r.clone()
        };
        assert!(on_17.measured_vram(&estimate, &ids).is_none());
    }

    #[test]
    fn add_ons_that_name_their_mod_replace_on_purpose() {
        let r = sample();
        let (between, _) = r.override_pairs();
        assert_eq!(r.pair_ids(&between[0]), ("c.other", "b.big"));

        let mod_with = |id: &str, deps: &[&str], after: &[&str], before: &[&str]| crate::models::ModInfo {
            package_id: id.into(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            load_after: after.iter().map(|s| s.to_string()).collect(),
            load_before: before.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        // UNAGI over NL: it requires NL (IDs in another case than the game writes them).
        let unagi = mod_with("unagi.facialanimation", &["Nals.FacialAnimation"], &[], &[]);
        let nl = mod_with("nals.facialanimation", &[], &[], &[]);
        assert!(pair_is_declared(
            Some(&unagi),
            "unagi.facialanimation",
            Some(&nl),
            "nals.facialanimation"
        ));
        // The other way round, or the original asking to load before it, counts as well.
        assert!(pair_is_declared(
            Some(&nl),
            "nals.facialanimation",
            Some(&unagi),
            "unagi.facialanimation"
        ));
        let patch_target = mod_with("x.base", &[], &[], &["Y.Patch"]);
        assert!(pair_is_declared(None, "y.patch", Some(&patch_target), "x.base"));
        // Two mods that don't know each other, or one that isn't installed.
        let other = mod_with("z.other", &["brrainz.harmony"], &["ludeon.rimworld"], &[]);
        assert!(!pair_is_declared(
            Some(&other),
            "z.other",
            Some(&nl),
            "nals.facialanimation"
        ));
        assert!(!pair_is_declared(None, "a", None, "b"));
    }

    #[test]
    fn shared_methods_put_the_risky_first() {
        let methods = sample().shared_methods();
        let order: Vec<(&str, PatchRisk)> = methods.iter().map(|m| (m.method.as_str(), m.risk)).collect();
        assert_eq!(
            order,
            [
                // Two mods rewrite it.
                ("Verse.Map.MapUpdate()", PatchRisk::High),
                // One transpiler and a prefix that can cancel.
                ("Verse.Thing.Draw()", PatchRisk::Some),
                ("RimWorld.Pawn.Tick()", PatchRisk::Low),
            ]
        );
        // One mod's prefix and postfix are one patcher; a patch from no mod goes by its owner.
        let big = &methods[1].patchers[0];
        assert_eq!((big.name.as_str(), big.kinds.len(), big.skips), ("Big Mod", 2, true));
        assert_eq!(methods[1].patchers[1].name, "some.id");
    }

    /// What the mod's own code wrote (CompanionReport.Build run on stand-ins for the game and
    /// Harmony): a def class of a mod's own, a def twice in one mod, a method only one mod
    /// patches, a patch from no mod's assemblies.
    #[test]
    fn the_mods_own_output_is_read() {
        let r: CompanionReport =
            serde_json::from_str(include_str!("../tests/data/companion_report_sample.json")).unwrap();
        assert_eq!(r.mod_name(2), "Mod A \"quoted\"");
        assert!(r.textures.is_none());
        let (between, over_game) = r.override_pairs();
        let pairs: Vec<(usize, usize, usize)> = between.iter().map(|p| (p.winner, p.loser, p.defs.len())).collect();
        assert_eq!(pairs, [(4, 2, 1), (4, 3, 1)]);
        assert_eq!(over_game.len(), 1);
        let methods = r.shared_methods();
        assert_eq!(methods.len(), 2);
        assert!(
            methods.iter().all(|m| m.risk == PatchRisk::Some),
            "a skipping prefix, one transpiler"
        );
        assert!(methods.iter().any(|m| m.patchers.iter().any(|p| p.name == "x.loose")));

        // A part the mod failed to collect is null, the rest is still read.
        let r: CompanionReport =
            serde_json::from_str(r#"{"format":1,"written_unix":1,"mods":null,"errors":["mods: X"]}"#).unwrap();
        assert!(r.mods.is_empty() && r.errors.len() == 1);
    }

    #[test]
    fn method_names_are_shortened() {
        assert_eq!(
            short_method("RimWorld.Pawn_HealthTracker.PostApplyDamage(DamageInfo, Single)"),
            "Pawn_HealthTracker.PostApplyDamage(DamageInfo, Single)"
        );
        assert_eq!(short_method("Verse.Thing.Draw()"), "Thing.Draw()");
        assert_eq!(short_method("Draw()"), "Draw()");
        // Dots inside the parameters do not count.
        assert_eq!(short_method("A.B.C(System.Int32)"), "B.C(System.Int32)");
    }

    #[test]
    fn report_on_disk_and_mod_list_changes() {
        let dir = std::env::temp_dir().join(format!("rwmm_report_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("companion_report.json");
        std::fs::write(&file, SAMPLE).unwrap();
        let r = load_from(&file).unwrap();
        let ids: Vec<String> = r.mods.iter().map(|m| m.id.to_uppercase()).collect();
        assert!(!r.mods_differ(&ids));
        assert!(r.mods_differ(&ids[1..]));

        // Another format, or a broken file: no report.
        std::fs::write(&file, SAMPLE.replace(r#""format":1"#, r#""format":2"#)).unwrap();
        assert!(load_from(&file).is_none());
        std::fs::write(&file, "{").unwrap();
        assert!(load_from(&file).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
