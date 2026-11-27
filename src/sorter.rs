use crate::categories::ModCategory;
use crate::community_rules::CommunityDatabase;
use crate::models::{is_protected_package_id, ModInfo};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

pub trait CommunityRulesExt {
    fn apply_community_rules(&mut self, db: &CommunityDatabase);
}

impl CommunityRulesExt for ModInfo {
    fn apply_community_rules(&mut self, db: &CommunityDatabase) {
        if let Some(rule) = db.rules.get(&self.package_id.to_lowercase()) {
            self.load_before.extend(rule.load_before.clone());
            self.load_after.extend(rule.load_after.clone());
            self.force_load_before.extend(rule.force_load_before.clone());
            self.force_load_after.extend(rule.force_load_after.clone());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ModTier {
    Tier0Prepatcher = 0,
    Tier1CoreOfficial = 1,
    Tier2Frameworks = 2,
    Tier3Overhauls = 3,
    Tier4FactionsRaces = 4,
    Tier5ContentItems = 5,
    Tier6BiomesAnimals = 6,
    Tier7UiCamera = 7,
    Tier8QolBehaviors = 8,
    Tier9Patches = 9,
    Tier10Translations = 10,
    Tier11Optimizers = 11,
}

impl ModTier {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Tier0Prepatcher),
            1 => Some(Self::Tier1CoreOfficial),
            2 => Some(Self::Tier2Frameworks),
            3 => Some(Self::Tier3Overhauls),
            4 => Some(Self::Tier4FactionsRaces),
            5 => Some(Self::Tier5ContentItems),
            6 => Some(Self::Tier6BiomesAnimals),
            7 => Some(Self::Tier7UiCamera),
            8 => Some(Self::Tier8QolBehaviors),
            9 => Some(Self::Tier9Patches),
            10 => Some(Self::Tier10Translations),
            11 => Some(Self::Tier11Optimizers),
            _ => None,
        }
    }
}

/// Tier from the content-based categories (strongest category first). None if there are none.
pub fn tier_from_categories(categories: &[ModCategory], name_lower: &str) -> Option<ModTier> {
    let first = categories.first()?;
    Some(match first {
        ModCategory::Translation => ModTier::Tier10Translations,
        ModCategory::Framework => ModTier::Tier2Frameworks,
        ModCategory::Race | ModCategory::Faction => ModTier::Tier4FactionsRaces,
        ModCategory::Animal | ModCategory::Biome => ModTier::Tier6BiomesAnimals,
        ModCategory::Patch | ModCategory::Retexture => ModTier::Tier9Patches,
        ModCategory::QoL => {
            // A code-only mod named "...patch" / "...compatibility" is a patch, not a QoL mod.
            let words: Vec<&str> = name_lower
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            if words
                .iter()
                .any(|w| matches!(*w, "patch" | "patches" | "compatibility" | "compat" | "fix" | "hotfix"))
            {
                ModTier::Tier9Patches
            } else {
                ModTier::Tier8QolBehaviors
            }
        }
        ModCategory::Building | ModCategory::Weapon | ModCategory::Apparel | ModCategory::Storyteller => {
            ModTier::Tier5ContentItems
        }
    })
}

pub fn classify_mod_tier_with_db(
    package_id: &str,
    name: &str,
    is_translation: bool,
    categories: &[ModCategory],
    db: Option<&CommunityDatabase>,
) -> ModTier {
    let id = package_id.to_lowercase();
    let n = name.to_lowercase();

    // 1. The community rules database, if loaded.
    if let Some(database) = db {
        if let Some(rule) = database.rules.get(&id) {
            if let Some(tier_num) = rule.tier {
                if let Some(tier) = ModTier::from_u8(tier_num) {
                    return tier;
                }
            }
        }
    }

    // 2. Core and the official DLCs.
    if is_protected_package_id(&id) || id.starts_with("ludeon.rimworld") || id == "brrainz.harmony" {
        return ModTier::Tier1CoreOfficial;
    }

    // 3. Prepatcher / Fishery.
    if id.contains("prepatcher") || n.contains("prepatcher") || id.contains("fishery") {
        return ModTier::Tier0Prepatcher;
    }

    // 4. Optimizers and profilers.
    if id.contains("rocketman")
        || n.contains("rocketman")
        || id.contains("performancefish")
        || n.contains("performance analyzer")
        || id.contains("performanceanalyzer")
        || id.contains("runtimegc")
        || n.contains("runtimegc")
    {
        return ModTier::Tier11Optimizers;
    }

    // 4b. Categories from the mod's content (Defs, DLL, Patches...) are more reliable than guessing by name.
    //     The substring heuristics below only cover mods without categories.
    if let Some(tier) = tier_from_categories(categories, &n) {
        return tier;
    }

    // 5. Translations.
    if crate::models::is_translation_indicator(&id, &n, is_translation) {
        return ModTier::Tier10Translations;
    }

    // 6. Patches and compatibility.
    if id.contains("patch")
        || n.contains("patch")
        || id.contains("compatibility")
        || n.contains("compatibility")
        || id.contains("rebalance")
        || n.contains("rebalance")
        || id.contains("hotfix")
        || n.contains("hotfix")
        || id.contains("fix")
        || n.contains("fix")
        || id.contains("tweak")
        || n.contains("tweak")
    {
        return ModTier::Tier9Patches;
    }

    // 7. Frameworks and libraries.
    if id.contains("hugslib")
        || id.contains("jecstools")
        || id.contains("humanoidalienraces")
        || n.contains("humanoid alien races")
        || id.contains("vehicleframework")
        || id.contains("xmlextensions")
        || id.contains("library")
        || n.contains("library")
        || id.ends_with(".framework")
        || n.contains("framework")
        || id.ends_with(".core")
        || n.ends_with(" core")
    {
        return ModTier::Tier2Frameworks;
    }

    // 8. Overhauls.
    if id.contains("combatextended")
        || n.contains("combat extended")
        || id.contains("saveourship")
        || n.contains("save our ship")
        || id.contains("rimwar")
        || n.contains("rimwar")
    {
        return ModTier::Tier3Overhauls;
    }

    // 9. Races and factions.
    if id.starts_with("vanillafactionsexpanded.")
        || id.starts_with("vanillaracesexpanded.")
        || id.contains("alienrace")
        || id.contains("xenotype")
        || id.contains("ratkin")
        || n.contains("ratkin")
        || id.contains("wolfein")
        || n.contains("wolfein")
        || id.contains("milira")
        || n.contains("milira")
        || id.contains("dragonian")
        || n.contains("dragonian")
        || id.contains("kiiro")
        || n.contains("kiiro")
        || id.contains("maru")
        || n.contains("maru")
        || id.contains("axolotl")
        || n.contains("moelotl")
        || id.contains("moyo")
        || n.contains("revia")
        || id.contains("kurin")
        || n.contains("faction")
        || id.contains("faction")
        // A whole word in the name: "Terraces", "Bracelets" and "Grace" aren't races.
        || crate::models::has_word(&n, &["race", "races"])
        || id.contains("race")
    {
        return ModTier::Tier4FactionsRaces;
    }

    // 10. Biomes, animals and plants.
    if id.starts_with("biomes.")
        || id.starts_with("vanillaanimalsexpanded.")
        || id.starts_with("vanillaplantssexpanded.")
        || id.contains("regrowth")
        || n.contains("animal")
        || id.contains("animal")
        || n.contains("plant")
        || id.contains("plant")
        || n.contains("biome")
        || id.contains("biome")
    {
        return ModTier::Tier6BiomesAnimals;
    }

    // 11. UI and camera.
    if id.contains("rimhud")
        || n.contains("rimhud")
        || id.contains("dubsmintmenus")
        || id.contains("dubsmintminimap")
        || id.contains("cameraplus")
        || id.contains("numbers")
        || id.contains("allowtool")
        || n.contains("allow tool")
        || id.contains("levelup")
        || n.contains("hud")
        || id.contains("hud")
        || n.contains("camera")
        || id.contains("camera")
        || id.ends_with(".ui")
        // A whole word: "Quick Build", "Guide" and "Power Suits" aren't UI mods.
        || crate::models::has_word(&n, &["ui"])
        || id.contains("worktab")
        || n.contains("work tab")
    {
        return ModTier::Tier7UiCamera;
    }

    // 12. QoL and colonist behaviour.
    if id.contains("pickupandhaul")
        || id.contains("commonsense")
        || id.contains("smartspeed")
        || id.contains("downforme")
        || id.contains("justputitoverthere")
        || id.contains("snapout")
        || id.contains("replacestuff")
        || id.contains("bubbles")
        || n.contains("haul")
        || n.contains("qol")
    {
        return ModTier::Tier8QolBehaviors;
    }

    ModTier::Tier5ContentItems
}

/// Where an edge of the load order graph comes from, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// A mod's About.xml: dependencies, loadAfter, loadBefore.
    AboutXml,
    /// The player's own rule (`user_order.rs`); dropped where it goes against About.xml.
    Own,
    /// Community rules, translations after their mod, Core and the DLCs first, frameworks
    /// early; dropped where it goes against an own rule.
    Soft,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SortCandidate {
    package_id: String,
    tier: ModTier,
    orig_idx: usize,
}

impl Ord for SortCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .tier
            .cmp(&self.tier)
            .then_with(|| other.orig_idx.cmp(&self.orig_idx))
    }
}

impl PartialOrd for SortCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub const CANON_OFFICIAL_ORDER: &[&str] = &[
    "zetrith.prepatcher",
    "fishery.world",
    "brrainz.harmony",
    "ludeon.rimworld",
    "ludeon.rimworld.royalty",
    "ludeon.rimworld.ideology",
    "ludeon.rimworld.biotech",
    "ludeon.rimworld.anomaly",
    "ludeon.rimworld.odyssey",
];

pub fn is_canon_official(id: &str) -> bool {
    let lower = id.to_lowercase();
    CANON_OFFICIAL_ORDER.iter().any(|&c| c == lower)
}

pub fn sort_load_order_with_db(
    active_package_ids: &[String],
    all_mods: &[ModInfo],
    db: Option<&CommunityDatabase>,
) -> Vec<String> {
    if active_package_ids.is_empty() {
        return Vec::new();
    }

    let db_ref: &CommunityDatabase = match db {
        Some(d) => d,
        None => crate::community_rules::get_default_community_db(),
    };

    let mut seen = HashSet::new();
    let mut unique_active: Vec<(String, String)> = Vec::new();
    for id in active_package_ids {
        let lower = id.to_lowercase();
        if seen.insert(lower.clone()) {
            unique_active.push((lower, id.clone()));
        }
    }

    let total_mods = unique_active.len();
    if total_mods == 0 {
        return Vec::new();
    }

    let mod_map: HashMap<String, &ModInfo> = all_mods.iter().map(|m| (m.package_id.to_lowercase(), m)).collect();

    let active_set: HashSet<String> = unique_active.iter().map(|(l, _)| l.clone()).collect();
    let idx_map: HashMap<String, usize> = unique_active
        .iter()
        .enumerate()
        .map(|(i, (l, _))| (l.clone(), i))
        .collect();
    let casing_map: HashMap<String, String> = unique_active
        .iter()
        .map(|(l, orig)| (l.clone(), orig.clone()))
        .collect();

    let mut tier_map: HashMap<String, ModTier> = HashMap::new();
    let mut parent_of_trans: HashMap<String, String> = HashMap::new();

    for (id_lower, _) in &unique_active {
        let (name, is_trans, parent, categories) = if let Some(m) = mod_map.get(id_lower) {
            (
                m.name.clone(),
                m.is_translation_mod,
                m.parent_package_id.clone(),
                m.categories.clone(),
            )
        } else {
            (id_lower.clone(), false, None, Vec::new())
        };

        let tier = classify_mod_tier_with_db(id_lower, &name, is_trans, &categories, Some(db_ref));
        tier_map.insert(id_lower.clone(), tier);

        if let Some(p) = parent {
            let p_lower = p.to_lowercase();
            if active_set.contains(&p_lower) && p_lower != *id_lower {
                parent_of_trans.insert(id_lower.clone(), p_lower);
            }
        }
    }

    let mut adj: HashMap<String, HashSet<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    for (id_lower, _) in &unique_active {
        in_degree.insert(id_lower.clone(), 0);
        adj.insert(id_lower.clone(), HashSet::new());
    }

    // The player's own rules turn round any edge but a mod's About.xml (which the game
    // checks): the opposite edge of each own rule is not added.
    let turned_round: HashSet<(String, String)> = db_ref
        .user_order
        .iter()
        .map(|r| (r.after.to_lowercase(), r.before.to_lowercase()))
        .collect();

    // A mod named in a rule may be enabled as its Steam copy, `<id>_steam` (the game's name
    // for the second copy of a mod), or the other way round: it's the same mod, so the rule
    // holds for it. Without this, a dependency enabled as `_steam` got no edge, the sort left
    // it below the mod that needs it and the dependency check kept saying "wrong order".
    let resolve = |id: &str| -> Option<String> {
        let lower = id.trim().to_lowercase();
        if active_set.contains(&lower) {
            return Some(lower);
        }
        let steam = format!("{lower}_steam");
        if active_set.contains(&steam) {
            return Some(steam);
        }
        lower
            .strip_suffix("_steam")
            .filter(|base| active_set.contains(*base))
            .map(str::to_string)
    };

    let mut add_edge = |from: &str, to: &str, kind: Edge| {
        let (Some(from_l), Some(to_l)) = (resolve(from), resolve(to)) else {
            return;
        };
        if from_l != to_l {
            if is_canon_official(&to_l) && !is_canon_official(&from_l) {
                return;
            }
            match kind {
                Edge::AboutXml => {}
                // About.xml edges are all in by now.
                Edge::Own if adj.get(&to_l).is_some_and(|next| next.contains(&from_l)) => return,
                Edge::Own => {}
                Edge::Soft if turned_round.contains(&(from_l.clone(), to_l.clone())) => return,
                Edge::Soft => {}
            }

            if adj.entry(from_l.clone()).or_default().insert(to_l.clone()) {
                *in_degree.entry(to_l).or_insert(0) += 1;
            }
        }
    };

    for (id_lower, _) in &unique_active {
        if let Some(m) = mod_map.get(id_lower) {
            for dep in m
                .load_after
                .iter()
                .chain(m.force_load_after.iter())
                .chain(m.dependencies.iter())
            {
                add_edge(dep, id_lower, Edge::AboutXml);
            }
            for target in m.load_before.iter().chain(m.force_load_before.iter()) {
                add_edge(id_lower, target, Edge::AboutXml);
            }
        }
    }

    for rule in &db_ref.user_order {
        add_edge(&rule.before, &rule.after, Edge::Own);
    }

    // Load order rules from the community database.
    for (rule_id, rule) in &db_ref.rules {
        for target in &rule.load_before {
            add_edge(rule_id, target, Edge::Soft);
        }
        for target in &rule.force_load_before {
            add_edge(rule_id, target, Edge::Soft);
        }
        for dep in &rule.load_after {
            add_edge(dep, rule_id, Edge::Soft);
        }
        for dep in &rule.force_load_after {
            add_edge(dep, rule_id, Edge::Soft);
        }
    }

    for (trans_id, parent_id) in &parent_of_trans {
        add_edge(parent_id, trans_id, Edge::Soft);
    }

    for window in CANON_OFFICIAL_ORDER.windows(2) {
        add_edge(window[0], window[1], Edge::Soft);
    }

    let last_official = CANON_OFFICIAL_ORDER
        .iter()
        .rev()
        .find(|&&id| active_set.contains(id))
        .copied();

    if let Some(last_off) = last_official {
        for (id_lower, _) in &unique_active {
            if !is_canon_official(id_lower) {
                add_edge(last_off, id_lower, Edge::Soft);
            }
        }
    }

    // Special hardcoded priority for Core Frameworks (HugsLib, VEF)
    // to ensure they rise to the top of Tier2Frameworks regardless of alphabet/original index
    let special_early = [
        "unlimitedhugs.hugslib",
        "oskarpotocki.vanillafactionsexpanded.core",
        "bustedbunny.adaptivestorageframework",
    ];

    for special in &special_early {
        if active_set.contains(*special) {
            for (id_lower, _) in &unique_active {
                if !is_canon_official(id_lower) && !special_early.contains(&id_lower.as_str()) {
                    add_edge(special, id_lower, Edge::Soft);
                }
            }
        }
    }

    // Explicitly order special_early among themselves
    for w in special_early.windows(2) {
        if active_set.contains(w[0]) && active_set.contains(w[1]) {
            add_edge(w[0], w[1], Edge::Soft);
        }
    }

    let mut ready_heap: BinaryHeap<SortCandidate> = BinaryHeap::new();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut result_lower: Vec<String> = Vec::with_capacity(total_mods);

    let make_candidate = |pkg_id: &str| -> SortCandidate {
        let eff_tier = if let Some(parent) = parent_of_trans.get(pkg_id) {
            tier_map.get(parent).copied().unwrap_or(ModTier::Tier10Translations)
        } else {
            tier_map.get(pkg_id).copied().unwrap_or(ModTier::Tier5ContentItems)
        };
        SortCandidate {
            package_id: pkg_id.to_string(),
            tier: eff_tier,
            orig_idx: idx_map.get(pkg_id).copied().unwrap_or(0),
        }
    };

    for (id_lower, _) in &unique_active {
        if in_degree.get(id_lower).copied().unwrap_or(0) == 0 {
            ready_heap.push(make_candidate(id_lower));
        }
    }

    while result_lower.len() < total_mods {
        if let Some(candidate) = ready_heap.pop() {
            let u = candidate.package_id;
            if emitted.contains(&u) {
                continue;
            }
            emitted.insert(u.clone());
            result_lower.push(u.clone());

            if let Some(neighbors) = adj.get(&u) {
                for v in neighbors {
                    if let Some(deg) = in_degree.get_mut(v) {
                        if *deg > 0 {
                            *deg -= 1;
                            if *deg == 0 {
                                ready_heap.push(make_candidate(v));
                            }
                        }
                    }
                }
            }
        } else {
            let mut best_unblock: Option<String> = None;
            let mut best_key = (ModTier::Tier11Optimizers, usize::MAX, usize::MAX);

            for (id_lower, _) in &unique_active {
                if !emitted.contains(id_lower) {
                    let t = tier_map.get(id_lower).copied().unwrap_or(ModTier::Tier5ContentItems);
                    let deg = in_degree.get(id_lower).copied().unwrap_or(1);
                    let idx = idx_map.get(id_lower).copied().unwrap_or(0);
                    let key = (t, deg, idx);
                    if best_unblock.is_none() || key < best_key {
                        best_key = key;
                        best_unblock = Some(id_lower.clone());
                    }
                }
            }

            if let Some(unblock_id) = best_unblock {
                in_degree.insert(unblock_id.clone(), 0);
                ready_heap.push(make_candidate(&unblock_id));
            } else {
                break;
            }
        }
    }

    let mut canon_found: Vec<String> = Vec::new();
    for &official_id in CANON_OFFICIAL_ORDER {
        if let Some(pos) = result_lower.iter().position(|x| x == official_id) {
            canon_found.push(result_lower.remove(pos));
        }
    }
    for (i, off_id) in canon_found.into_iter().enumerate() {
        result_lower.insert(i, off_id);
    }

    if let Some(pos) = result_lower.iter().position(|x| x == "krkr.rocketman") {
        let rocketman = result_lower.remove(pos);
        let mut trans_items = Vec::new();
        let mut i = 0;
        while i < result_lower.len() {
            if parent_of_trans.get(&result_lower[i]).map(String::as_str) == Some("krkr.rocketman") {
                trans_items.push(result_lower.remove(i));
            } else {
                i += 1;
            }
        }
        result_lower.push(rocketman);
        for t in trans_items {
            result_lower.push(t);
        }
    }

    result_lower
        .into_iter()
        .map(|id_lower| casing_map.get(&id_lower).cloned().unwrap_or(id_lower))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dependency_enabled_as_its_steam_copy_still_goes_first() {
        // A library has two copies, the Steam one is enabled as `someone.zetalib_steam`; the
        // mod that needs it is above it and would stay there by name and tier alone.
        let library = ModInfo {
            package_id: "someone.zetalib_steam".into(),
            name: "Zeta".into(),
            ..Default::default()
        };
        let needs = ModInfo {
            package_id: "someone.alpha".into(),
            name: "Alpha".into(),
            dependencies: vec!["someone.zetalib".into()],
            ..Default::default()
        };
        let mods = vec![library, needs];
        let active = vec!["someone.alpha".to_string(), "someone.zetalib_steam".to_string()];
        let sorted = sort_load_order_with_db(&active, &mods, None);
        let pos = |id: &str| sorted.iter().position(|s| s.eq_ignore_ascii_case(id)).unwrap();
        assert!(pos("someone.zetalib_steam") < pos("someone.alpha"), "{sorted:?}");
        let report = crate::dependencies::validate_dependencies(&sorted, &mods);
        assert_eq!(report.order_count(), 0, "{:?}", report.problems);
    }

    #[test]
    fn categories_decide_tier_instead_of_name_guessing() {
        // "Quality" contains "ui", "Terraces" contains "race": names match whole words only.
        assert_ne!(
            classify_mod_tier_with_db("a.b", "Quality Builder", false, &[], None),
            ModTier::Tier7UiCamera
        );
        assert_ne!(
            classify_mod_tier_with_db("a.b", "Terraces", false, &[], None),
            ModTier::Tier4FactionsRaces
        );
        assert_eq!(
            classify_mod_tier_with_db("a.b", "Better UI", false, &[], None),
            ModTier::Tier7UiCamera
        );
        assert_eq!(
            classify_mod_tier_with_db("a.b", "Snake Race", false, &[], None),
            ModTier::Tier4FactionsRaces
        );
        assert_eq!(
            classify_mod_tier_with_db("a.b", "Quality Builder", false, &[ModCategory::QoL], None),
            ModTier::Tier8QolBehaviors
        );
        assert_eq!(
            classify_mod_tier_with_db("a.b", "Ratkin", false, &[ModCategory::Race, ModCategory::Apparel], None),
            ModTier::Tier4FactionsRaces
        );
        assert_eq!(
            classify_mod_tier_with_db("a.b", "CE Compatibility Patch", false, &[ModCategory::QoL], None),
            ModTier::Tier9Patches
        );
        // Official mods keep their tier regardless of categories.
        assert_eq!(
            classify_mod_tier_with_db("ludeon.rimworld", "Core", false, &[ModCategory::Weapon], None),
            ModTier::Tier1CoreOfficial
        );
    }
}
