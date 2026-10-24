use crate::models::{is_protected_package_id, ModInfo};
use crate::community_rules::CommunityDatabase;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct ConflictReport {
    pub incompatibilities: Vec<(String, String)>,
}

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

pub fn check_incompatibilities(
    active_package_ids: &[String],
    db: &CommunityDatabase,
) -> ConflictReport {
    let mut report = ConflictReport::default();
    let active_set: HashSet<String> = active_package_ids.iter().map(|id| id.to_lowercase()).collect();

    for (id, rule) in &db.rules {
        if active_set.contains(id) {
            for incomp in &rule.incompatible {
                let incomp_lower = incomp.to_lowercase();
                if active_set.contains(&incomp_lower) {
                    report.incompatibilities.push((id.clone(), incomp_lower));
                }
            }
        }
    }
    report
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

    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

pub fn classify_mod_tier(package_id: &str, name: &str, is_translation: bool) -> ModTier {
    classify_mod_tier_with_db(
        package_id,
        name,
        is_translation,
        Some(crate::community_rules::get_default_community_db()),
    )
}

pub fn classify_mod_tier_with_db(
    package_id: &str,
    name: &str,
    is_translation: bool,
    db: Option<&CommunityDatabase>,
) -> ModTier {
    let id = package_id.to_lowercase();
    let n = name.to_lowercase();

    // 1. Проверяем CommunityDatabase (если задана)
    if let Some(database) = db {
        if let Some(rule) = database.rules.get(&id) {
            if let Some(tier_num) = rule.tier {
                if let Some(tier) = ModTier::from_u8(tier_num) {
                    return tier;
                }
            }
        }
    }

    // 2. Официальные моды и Core
    if is_protected_package_id(&id) || id.starts_with("ludeon.rimworld") || id == "brrainz.harmony" {
        return ModTier::Tier1CoreOfficial;
    }

    // 3. Prepatcher / Fishery эвристика
    if id.contains("prepatcher") || n.contains("prepatcher") || id.contains("fishery") {
        return ModTier::Tier0Prepatcher;
    }

    // 4. Оптимизаторы и профилировщики
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

    // 5. Переводы
    if crate::models::is_translation_indicator(&id, &n, is_translation) {
        return ModTier::Tier10Translations;
    }

    // 6. Патчи и совместимость
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

    // 7. Фреймворки и библиотеки
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

    // 8. Глобальные оверхолы
    if id.contains("combatextended")
        || n.contains("combat extended")
        || id.contains("saveourship")
        || n.contains("save our ship")
        || id.contains("rimwar")
        || n.contains("rimwar")
    {
        return ModTier::Tier3Overhauls;
    }

    // 9. Расы и фракции
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
        || n.contains("race")
        || id.contains("race")
    {
        return ModTier::Tier4FactionsRaces;
    }

    // 10. Биомы, животные и растения
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

    // 11. UI и камера
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
        || n.contains("ui")
        || id.contains("worktab")
        || n.contains("work tab")
    {
        return ModTier::Tier7UiCamera;
    }

    // 12. QoL и поведение колонистов
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

pub fn sort_load_order(active_package_ids: &[String], all_mods: &[ModInfo]) -> Vec<String> {
    sort_load_order_with_db(active_package_ids, all_mods, None)
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

    let mod_map: HashMap<String, &ModInfo> = all_mods
        .iter()
        .map(|m| (m.package_id.to_lowercase(), m))
        .collect();

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
        let (name, is_trans, parent) = if let Some(m) = mod_map.get(id_lower) {
            (
                m.name.clone(),
                m.is_translation_mod,
                m.parent_package_id.clone(),
            )
        } else {
            (id_lower.clone(), false, None)
        };

        let tier = classify_mod_tier_with_db(id_lower, &name, is_trans, Some(db_ref));
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

    let mut add_edge = |from: &str, to: &str| {
        let from_l = from.to_lowercase();
        let to_l = to.to_lowercase();
        if from_l != to_l && active_set.contains(&from_l) && active_set.contains(&to_l) {
            if is_canon_official(&to_l) && !is_canon_official(&from_l) {
                return;
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
                add_edge(dep, id_lower);
            }
            for target in m.load_before.iter().chain(m.force_load_before.iter()) {
                add_edge(id_lower, target);
            }
        }
    }

    // Правила порядка загрузки из базы сообщества
    for (rule_id, rule) in &db_ref.rules {
        for target in &rule.load_before {
            add_edge(rule_id, target);
        }
        for target in &rule.force_load_before {
            add_edge(rule_id, target);
        }
        for dep in &rule.load_after {
            add_edge(dep, rule_id);
        }
        for dep in &rule.force_load_after {
            add_edge(dep, rule_id);
        }
    }

    for (trans_id, parent_id) in &parent_of_trans {
        add_edge(parent_id, trans_id);
    }

    for window in CANON_OFFICIAL_ORDER.windows(2) {
        add_edge(window[0], window[1]);
    }

    let last_official = CANON_OFFICIAL_ORDER
        .iter()
        .rev()
        .find(|&&id| active_set.contains(id))
        .copied();

    if let Some(last_off) = last_official {
        for (id_lower, _) in &unique_active {
            if !is_canon_official(id_lower) {
                add_edge(last_off, id_lower);
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
                    add_edge(special, id_lower);
                }
            }
        }
    }
    
    // Explicitly order special_early among themselves
    for w in special_early.windows(2) {
        if active_set.contains(w[0]) && active_set.contains(w[1]) {
            add_edge(w[0], w[1]);
        }
    }

    let mut ready_heap: BinaryHeap<SortCandidate> = BinaryHeap::new();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut result_lower: Vec<String> = Vec::with_capacity(total_mods);

    let make_candidate = |pkg_id: &str| -> SortCandidate {
        let eff_tier = if let Some(parent) = parent_of_trans.get(pkg_id) {
            tier_map
                .get(parent)
                .copied()
                .unwrap_or(ModTier::Tier10Translations)
        } else {
            tier_map
                .get(pkg_id)
                .copied()
                .unwrap_or(ModTier::Tier5ContentItems)
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
                    let t = tier_map
                        .get(id_lower)
                        .copied()
                        .unwrap_or(ModTier::Tier5ContentItems);
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


