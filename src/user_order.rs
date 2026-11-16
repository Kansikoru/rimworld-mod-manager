//! The player's own load order rules ("this mod always loads after that one"). Made by
//! "Remember this place in the load order" in the menu of an active mod, kept in
//! `user_order_rules.json` in the program's data folder and applied by every sort on top of
//! the community rules (`sorter::sort_load_order_with_db`). A mod's own About.xml and the
//! fixed place of Core and the DLCs still win: the game itself checks those.

use crate::community_rules::CommunityDatabase;
use crate::models::ModInfo;
use crate::sorter::sort_load_order_with_db;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Most rules one "remember this place" may add. One or two are the usual case.
const MAX_RULES_PER_PIN: usize = 16;

/// `after` loads after `before`. IDs are in lower case; the names are what the mods were
/// called when the rule was made, for the list of rules once a mod isn't installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRule {
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub before_name: String,
    #[serde(default)]
    pub after_name: String,
}

impl OrderRule {
    pub fn involves(&self, id: &str) -> bool {
        self.before.eq_ignore_ascii_case(id) || self.after.eq_ignore_ascii_case(id)
    }

    /// Same two mods, in either direction.
    fn same_pair(&self, other: &OrderRule) -> bool {
        self.involves(&other.before) && self.involves(&other.after)
    }
}

pub fn rules_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("user_order_rules.json")
}

/// Rules in the file; none when it's missing or unreadable. IDs come back in lower case,
/// rules of a mod with itself are dropped.
pub fn load(path: &Path) -> Vec<OrderRule> {
    let rules: Vec<OrderRule> = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut out: Vec<OrderRule> = Vec::with_capacity(rules.len());
    for mut rule in rules {
        rule.before = rule.before.to_lowercase();
        rule.after = rule.after.to_lowercase();
        if rule.before != rule.after {
            add(&mut out, rule);
        }
    }
    out
}

pub fn save(path: &Path, rules: &[OrderRule]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(rules).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Adds `rule`, replacing a rule about the same two mods (in either direction): the newer
/// wish of the player wins.
pub fn add(rules: &mut Vec<OrderRule>, rule: OrderRule) {
    rules.retain(|r| !r.same_pair(&rule));
    rules.push(rule);
}

/// Removes every rule that names `id`; returns how many.
pub fn remove_for(rules: &mut Vec<OrderRule>, id: &str) -> usize {
    let before = rules.len();
    rules.retain(|r| !r.involves(id));
    before - rules.len()
}

/// What "remember this place" did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinOutcome {
    /// The sort already leaves the mod where it is: nothing to remember.
    AlreadyThere,
    /// These rules were added; the sort now leaves the mod in its place.
    Added(Vec<OrderRule>),
    /// The mod can't stay there: its About.xml (or that of `blocker`), or the fixed place
    /// of Core and the DLCs, puts it on the other side of `blocker` (lower-case ID).
    /// Nothing was added.
    Blocked { blocker: String },
    /// More rules than `MAX_RULES_PER_PIN` would be needed. Nothing was added.
    TooFar,
}

/// Makes the sort leave `mod_id` where it stands in `active` relative to every other mod:
/// sorts, takes the nearest mod (in the current list) that the sort puts on the other side,
/// adds a rule for that pair, and sorts again until nothing is on the wrong side. The rules
/// go into `db.user_order`; on `Blocked` and `TooFar` it is left as it was.
pub fn pin_place(active: &[String], all_mods: &[ModInfo], db: &mut CommunityDatabase, mod_id: &str) -> PinOutcome {
    let target = mod_id.to_lowercase();
    let mut current: Vec<String> = Vec::with_capacity(active.len());
    for id in active {
        let lower = id.to_lowercase();
        if !current.contains(&lower) {
            current.push(lower);
        }
    }
    let Some(here) = current.iter().position(|id| *id == target) else {
        return PinOutcome::AlreadyThere;
    };
    let name_of = |id: &str| {
        all_mods
            .iter()
            .find(|m| m.package_id.eq_ignore_ascii_case(id))
            .map_or_else(|| id.to_string(), |m| m.name.clone())
    };

    let kept = db.user_order.clone();
    let mut added: Vec<OrderRule> = Vec::new();
    loop {
        let place: HashMap<String, usize> = sort_load_order_with_db(&current, all_mods, Some(db))
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id.to_lowercase(), i))
            .collect();
        let Some(&sorted_here) = place.get(&target) else {
            break;
        };
        let wrong_side = current
            .iter()
            .enumerate()
            .filter(|&(i, id)| i != here && place.get(id).is_some_and(|&s| (i < here) != (s < sorted_here)))
            .min_by_key(|&(i, _)| i.abs_diff(here));
        let Some((i, other)) = wrong_side else {
            break;
        };
        let (before, after) = if i < here { (other, &target) } else { (&target, other) };
        let rule = OrderRule {
            before: before.clone(),
            after: after.clone(),
            before_name: name_of(before),
            after_name: name_of(after),
        };
        if added.iter().any(|r| r.same_pair(&rule)) {
            // The rule for this pair is in and the sort still disagrees: something stronger
            // than an own rule holds the two mods the other way round.
            db.user_order = kept;
            return PinOutcome::Blocked { blocker: other.clone() };
        }
        if added.len() >= MAX_RULES_PER_PIN {
            db.user_order = kept;
            return PinOutcome::TooFar;
        }
        add(&mut db.user_order, rule.clone());
        added.push(rule);
    }
    if added.is_empty() {
        PinOutcome::AlreadyThere
    } else {
        PinOutcome::Added(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::ModCategory;

    fn installed(id: &str, name: &str) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn sort(active: &[String], mods: &[ModInfo], db: &CommunityDatabase) -> Vec<String> {
        sort_load_order_with_db(active, mods, Some(db))
    }

    /// `id` has every other mod of `wanted` on the same side in `sorted`.
    fn keeps_place(sorted: &[String], wanted: &[String], id: &str) -> bool {
        let pos = |list: &[String], x: &str| list.iter().position(|m| m == x).unwrap();
        let (s, w) = (pos(sorted, id), pos(wanted, id));
        wanted
            .iter()
            .filter(|m| *m != id)
            .all(|m| (pos(sorted, m) < s) == (pos(wanted, m) < w))
    }

    #[test]
    fn a_remembered_place_survives_every_sort() {
        // A framework the sorter lifts to the top; the player wants it after the content mod.
        let mut framework = installed("a.framework", "Some Framework");
        framework.categories = vec![ModCategory::Framework];
        let mods = vec![
            installed("ludeon.rimworld", "Core"),
            framework,
            installed("b.content", "Content"),
            installed("c.other", "Other"),
        ];
        let mut db = CommunityDatabase::default();
        let wanted = ids(&["ludeon.rimworld", "b.content", "a.framework", "c.other"]);
        assert_eq!(
            sort(&wanted, &mods, &db),
            ids(&["ludeon.rimworld", "a.framework", "b.content", "c.other"]),
            "without a rule the framework goes first"
        );

        let outcome = pin_place(&wanted, &mods, &mut db, "A.Framework");
        let PinOutcome::Added(rules) = outcome else {
            panic!("expected rules, got {outcome:?}");
        };
        assert_eq!(rules.len(), 1, "one rule against the nearest mod is enough: {rules:?}");
        assert_eq!(
            (rules[0].before.as_str(), rules[0].after.as_str()),
            ("b.content", "a.framework")
        );
        assert_eq!(rules[0].after_name, "Some Framework");
        assert_eq!(sort(&wanted, &mods, &db), wanted);

        // From another starting order the framework still follows the content mod, and
        // remembering the same place again adds nothing.
        let shuffled = ids(&["c.other", "a.framework", "b.content", "ludeon.rimworld"]);
        let sorted = sort(&shuffled, &mods, &db);
        let pos = |id: &str| sorted.iter().position(|m| m == id).unwrap();
        assert!(pos("b.content") < pos("a.framework"), "{sorted:?}");
        assert_eq!(
            pin_place(&wanted, &mods, &mut db, "a.framework"),
            PinOutcome::AlreadyThere
        );
    }

    #[test]
    fn an_own_rule_beats_the_community_rules_but_not_about_xml() {
        let mut translation = installed("x.translation", "X Translation");
        translation.load_before = vec!["x.mod".to_string()];
        let mods = vec![
            installed("ludeon.rimworld", "Core"),
            installed("x.mod", "X"),
            translation,
            installed("y.patch", "Y Patch"),
            installed("z.mod", "Z"),
        ];
        // Community rule: the patch loads before Z. The player wants it after.
        let mut db = CommunityDatabase::default();
        db.rules.insert(
            "y.patch".to_string(),
            crate::models::CommunityRule {
                load_before: vec!["z.mod".to_string()],
                ..Default::default()
            },
        );
        let wanted = ids(&["ludeon.rimworld", "x.translation", "x.mod", "z.mod", "y.patch"]);
        assert!(!keeps_place(&sort(&wanted, &mods, &db), &wanted, "y.patch"));
        assert!(matches!(
            pin_place(&wanted, &mods, &mut db, "y.patch"),
            PinOutcome::Added(_)
        ));
        assert!(keeps_place(&sort(&wanted, &mods, &db), &wanted, "y.patch"));

        // About.xml of the translation says "before X": an own rule can't turn that round.
        let against_about = ids(&["ludeon.rimworld", "x.mod", "x.translation", "z.mod", "y.patch"]);
        let rules_before = db.user_order.clone();
        assert_eq!(
            pin_place(&against_about, &mods, &mut db, "x.translation"),
            PinOutcome::Blocked {
                blocker: "x.mod".to_string()
            }
        );
        assert_eq!(db.user_order, rules_before, "nothing added when blocked");

        // Nor can a mod go above Core.
        let above_core = ids(&["y.patch", "ludeon.rimworld", "x.translation", "x.mod", "z.mod"]);
        assert_eq!(
            pin_place(&above_core, &mods, &mut db, "y.patch"),
            PinOutcome::Blocked {
                blocker: "ludeon.rimworld".to_string()
            }
        );
    }

    #[test]
    fn rules_are_stored_replaced_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("rmm_user_order_{}", std::process::id()));
        let path = dir.join("user_order_rules.json");
        let rule = |before: &str, after: &str| OrderRule {
            before: before.to_string(),
            after: after.to_string(),
            before_name: String::new(),
            after_name: String::new(),
        };
        let mut rules = Vec::new();
        add(&mut rules, rule("a", "b"));
        add(&mut rules, rule("c", "b"));
        add(&mut rules, rule("b", "a"));
        assert_eq!(
            rules,
            vec![rule("c", "b"), rule("b", "a")],
            "the newer rule about a pair wins"
        );

        save(&path, &[rule("A.Mod", "B.Mod"), rule("x", "x")]).unwrap();
        assert_eq!(
            load(&path),
            vec![rule("a.mod", "b.mod")],
            "lower case, no rule of a mod with itself"
        );
        assert!(load(&dir.join("missing.json")).is_empty());

        assert_eq!(remove_for(&mut rules, "B"), 2);
        assert!(rules.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
