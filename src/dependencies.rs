use crate::models::{is_protected_package_id, ModInfo};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyProblem {
    /// Зависимость не найдена на диске (ни в Workshop, ни локально)
    Missing {
        dependent_name: String,
        dependent_id: String,
        required_id: String,
    },
    /// Зависимость установлена, но выключена в списке активных
    Disabled {
        dependent_name: String,
        dependent_id: String,
        required_id: String,
        required_name: String,
    },
    /// Зависимость включена, но стоит в списке загрузки НИЖЕ зависимого мода
    WrongOrder {
        dependent_name: String,
        dependent_id: String,
        required_name: String,
        required_id: String,
    },
    /// Мод объявил `incompatibleWith`, и этот мод тоже включён
    Incompatible {
        dependent_name: String,
        dependent_id: String,
        other_name: String,
        other_id: String,
    },
}

impl DependencyProblem {
    /// The mod the problem is shown on.
    pub fn dependent_id(&self) -> &str {
        match self {
            DependencyProblem::Missing { dependent_id, .. }
            | DependencyProblem::Disabled { dependent_id, .. }
            | DependencyProblem::WrongOrder { dependent_id, .. }
            | DependencyProblem::Incompatible { dependent_id, .. } => dependent_id,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DependencyReport {
    pub problems: Vec<DependencyProblem>,
}

impl DependencyReport {
    pub fn is_empty(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn missing_count(&self) -> usize {
        self.problems
            .iter()
            .filter(|p| matches!(p, DependencyProblem::Missing { .. }))
            .count()
    }

    pub fn disabled_count(&self) -> usize {
        self.problems
            .iter()
            .filter(|p| matches!(p, DependencyProblem::Disabled { .. }))
            .count()
    }

    pub fn order_count(&self) -> usize {
        self.problems
            .iter()
            .filter(|p| matches!(p, DependencyProblem::WrongOrder { .. }))
            .count()
    }

    pub fn incompatible_count(&self) -> usize {
        self.incompatible_pairs().len()
    }

    /// Incompatible pairs, each once even when both mods declare it:
    /// `((id, name), (id, name))`.
    pub fn incompatible_pairs(&self) -> Vec<((String, String), (String, String))> {
        let mut seen = std::collections::HashSet::new();
        self.problems
            .iter()
            .filter_map(|p| match p {
                DependencyProblem::Incompatible {
                    dependent_name,
                    dependent_id,
                    other_name,
                    other_id,
                } => {
                    let (a, b) = (dependent_id.to_lowercase(), other_id.to_lowercase());
                    let key = if a <= b { (a, b) } else { (b, a) };
                    seen.insert(key).then(|| {
                        (
                            (dependent_id.clone(), dependent_name.clone()),
                            (other_id.clone(), other_name.clone()),
                        )
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Something "Auto-fix" can do: enable or sort dependencies (not incompatibilities —
    /// which of two mods to turn off is the player's choice).
    pub fn auto_fixable(&self) -> bool {
        self.missing_count() + self.disabled_count() + self.order_count() > 0
    }
}

/// Mods that satisfy a dependency on another mod.
/// Prepatcher's author: "zetrith.prepatcher (this mod) satisfies dependencies on brrainz.harmony".
pub fn dependency_substitutes(dep_lower: &str) -> &'static [&'static str] {
    match dep_lower {
        "brrainz.harmony" => &["zetrith.prepatcher"],
        _ => &[],
    }
}

/// Выполняет полную валидацию зависимостей для текущего списка активных модов.
pub fn validate_dependencies(active_package_ids: &[String], all_mods: &[ModInfo]) -> DependencyReport {
    let mut report = DependencyReport::default();

    let installed_map: HashMap<String, &ModInfo> = all_mods.iter().map(|m| (m.package_id.to_lowercase(), m)).collect();

    // RimWorld calls the Workshop copy of a mod that also exists as a local copy
    // `<id>_steam`. For dependencies it is the same mod: `brrainz.harmony_steam` satisfies
    // "needs brrainz.harmony", and its own dependencies are checked too.
    let canonical = |id: &str| -> String {
        let lower = id.to_lowercase();
        if installed_map.contains_key(&lower) {
            return lower;
        }
        match lower.strip_suffix("_steam") {
            Some(base) if installed_map.contains_key(base) => base.to_string(),
            _ => lower,
        }
    };

    let active_indices: HashMap<String, usize> = active_package_ids
        .iter()
        .enumerate()
        .map(|(idx, id)| (canonical(id), idx))
        .collect();

    let mut seen_problems: HashSet<(String, String)> = HashSet::new();

    for (current_idx, active_id) in active_package_ids.iter().enumerate() {
        let active_lower = canonical(active_id);
        let Some(mod_info) = installed_map.get(&active_lower) else {
            continue;
        };

        for dep_raw in &mod_info.dependencies {
            let dep_lower = dep_raw.trim().to_lowercase();

            if dep_lower.is_empty() || dep_lower == active_lower {
                continue;
            }

            if !seen_problems.insert((active_lower.clone(), dep_lower.clone())) {
                continue;
            }

            // Another active mod can stand in for the dependency: Prepatcher "can be used in
            // place of the Harmony mod", and About.xml may list alternativePackageIds.
            let declared_alternatives = mod_info
                .dependency_alternatives
                .get(&dep_lower)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let substitute_active = dependency_substitutes(&dep_lower)
                .iter()
                .any(|alt| active_indices.contains_key(*alt))
                || declared_alternatives
                    .iter()
                    .any(|alt| active_indices.contains_key(&alt.to_lowercase()));
            if substitute_active && !active_indices.contains_key(&dep_lower) {
                continue;
            }

            // 1. Проверяем системные модули и официальные DLC
            if is_protected_package_id(&dep_lower) {
                if let Some(&dep_idx) = active_indices.get(&dep_lower) {
                    if dep_idx > current_idx {
                        report.problems.push(DependencyProblem::WrongOrder {
                            dependent_name: mod_info.name.clone(),
                            dependent_id: mod_info.package_id.clone(),
                            required_name: dep_raw.clone(),
                            required_id: dep_raw.clone(),
                        });
                    }
                } else {
                    report.problems.push(DependencyProblem::Disabled {
                        dependent_name: mod_info.name.clone(),
                        dependent_id: mod_info.package_id.clone(),
                        required_id: dep_raw.clone(),
                        required_name: dep_raw.clone(),
                    });
                }
                continue;
            }

            // 2. Проверяем пользовательские моды
            if let Some(dep_mod) = installed_map.get(&dep_lower) {
                if let Some(&dep_idx) = active_indices.get(&dep_lower) {
                    if dep_idx > current_idx {
                        report.problems.push(DependencyProblem::WrongOrder {
                            dependent_name: mod_info.name.clone(),
                            dependent_id: mod_info.package_id.clone(),
                            required_name: dep_mod.name.clone(),
                            required_id: dep_mod.package_id.clone(),
                        });
                    }
                } else {
                    report.problems.push(DependencyProblem::Disabled {
                        dependent_name: mod_info.name.clone(),
                        dependent_id: mod_info.package_id.clone(),
                        required_id: dep_mod.package_id.clone(),
                        required_name: dep_mod.name.clone(),
                    });
                }
            } else {
                report.problems.push(DependencyProblem::Missing {
                    dependent_name: mod_info.name.clone(),
                    dependent_id: mod_info.package_id.clone(),
                    required_id: dep_raw.clone(),
                });
            }
        }

        // incompatibleWith: both mods are active at the same time
        for other_raw in &mod_info.incompatible_with {
            let other_lower = other_raw.trim().to_lowercase();
            if other_lower.is_empty() || other_lower == active_lower || !active_indices.contains_key(&other_lower) {
                continue;
            }
            if !seen_problems.insert((active_lower.clone(), format!("!{other_lower}"))) {
                continue;
            }
            let other_name = installed_map
                .get(&other_lower)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| other_raw.clone());
            report.problems.push(DependencyProblem::Incompatible {
                dependent_name: mod_info.name.clone(),
                dependent_id: mod_info.package_id.clone(),
                other_name,
                other_id: other_raw.clone(),
            });
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModInfo, ModOrigin};

    fn m(id: &str, deps: &[&str]) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            name: id.to_string(),
            name_lower: id.to_lowercase(),
            author: String::new(),
            description: String::new(),
            dir_path: std::path::PathBuf::new(),
            preview_path: None,
            supported_versions: vec![],
            load_after: vec![],
            force_load_after: vec![],
            load_before: vec![],
            force_load_before: vec![],
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            origin: ModOrigin::Workshop,
            languages: vec![],
            is_translation_mod: false,
            parent_package_id: None,
            steam_item_id: None,
            has_defs: true,
            has_assemblies: true,
            has_other_content: true,
            ..Default::default()
        }
    }

    #[test]
    fn alternatives_and_incompatibilities() {
        let mut needs_lib = m("someone.mod", &["original.lib"]);
        needs_lib
            .dependency_alternatives
            .insert("original.lib".into(), vec!["fork.lib".into()]);
        needs_lib.incompatible_with = vec!["bad.mod".into()];
        let mods = vec![m("fork.lib", &[]), m("bad.mod", &[]), needs_lib];

        let active: Vec<String> = ["fork.lib", "someone.mod"].iter().map(|s| s.to_string()).collect();
        assert!(
            validate_dependencies(&active, &mods).is_empty(),
            "fork satisfies the dependency"
        );

        let with_bad: Vec<String> = ["fork.lib", "someone.mod", "bad.mod"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rep = validate_dependencies(&with_bad, &mods);
        assert_eq!(rep.incompatible_count(), 1);
        assert_eq!(rep.problems[0].dependent_id(), "someone.mod");
        assert!(!rep.auto_fixable(), "only the player can pick which mod to turn off");

        // Both mods declare it: still one pair.
        let mut bad = m("bad.mod", &[]);
        bad.incompatible_with = vec!["Someone.Mod".into()];
        let mut needs_lib = mods[2].clone();
        needs_lib.incompatible_with = vec!["bad.mod".into()];
        let both = vec![m("fork.lib", &[]), bad, needs_lib];
        let rep = validate_dependencies(&with_bad, &both);
        assert_eq!(rep.incompatible_pairs().len(), 1);
    }

    #[test]
    fn prepatcher_satisfies_harmony_dependency() {
        let mods = vec![
            m("zetrith.prepatcher", &[]),
            m("brrainz.harmony", &[]),
            m("ludeon.rimworld", &[]),
            m("someone.mod", &["brrainz.harmony"]),
        ];
        let active: Vec<String> = ["zetrith.prepatcher", "ludeon.rimworld", "someone.mod"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(validate_dependencies(&active, &mods).is_empty());

        let without_prepatcher: Vec<String> = ["ludeon.rimworld", "someone.mod"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(validate_dependencies(&without_prepatcher, &mods).disabled_count(), 1);
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dlc_that_isnt_installed_is_missing_not_disabled() {
        // Biotech is not owned: it isn't among the scanned mods at all.
        let mods = vec![
            m("ludeon.rimworld", &[]),
            m("someone.genes", &["ludeon.rimworld.biotech"]),
        ];
        let report = validate_dependencies(&ids(&["ludeon.rimworld", "someone.genes"]), &mods);
        assert_eq!(report.missing_count(), 1, "reported as not found");
        assert_eq!(
            report.disabled_count(),
            0,
            "so the auto-fix never adds a DLC the player doesn't have"
        );

        // Owned but turned off: that one is "disabled" and the auto-fix may turn it on.
        let owned = vec![
            m("ludeon.rimworld", &[]),
            m("ludeon.rimworld.biotech", &[]),
            m("someone.genes", &["ludeon.rimworld.biotech"]),
        ];
        let report = validate_dependencies(&ids(&["ludeon.rimworld", "someone.genes"]), &owned);
        assert_eq!((report.missing_count(), report.disabled_count()), (0, 1));
    }

    #[test]
    fn workshop_copy_with_steam_suffix_counts_as_the_mod() {
        let mods = vec![
            m("brrainz.harmony", &[]),
            m("someone.mod", &["brrainz.harmony"]),
            m("other.mod", &["needed.lib"]),
        ];
        // Harmony is enabled as its Workshop copy: the dependency is met, not "disabled".
        let report = validate_dependencies(&ids(&["brrainz.harmony_steam", "someone.mod"]), &mods);
        assert!(report.is_empty(), "{:?}", report.problems);

        // A mod enabled as `_steam` has its own dependencies checked.
        let report = validate_dependencies(&ids(&["Other.Mod_steam"]), &mods);
        assert_eq!(report.missing_count(), 1);
        assert_eq!(report.problems[0].dependent_id(), "other.mod");

        // Order still matters with the suffix.
        let report = validate_dependencies(&ids(&["someone.mod", "brrainz.harmony_steam"]), &mods);
        assert_eq!(report.order_count(), 1);
    }
}
