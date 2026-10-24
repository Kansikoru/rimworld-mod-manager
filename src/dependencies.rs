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
}

/// Выполняет полную валидацию зависимостей для текущего списка активных модов.
pub fn validate_dependencies(
    active_package_ids: &[String],
    all_mods: &[ModInfo],
) -> DependencyReport {
    let mut report = DependencyReport::default();

    let installed_map: HashMap<String, &ModInfo> = all_mods
        .iter()
        .map(|m| (m.package_id.to_lowercase(), m))
        .collect();

    let active_indices: HashMap<String, usize> = active_package_ids
        .iter()
        .enumerate()
        .map(|(idx, id)| (id.to_lowercase(), idx))
        .collect();

    let mut seen_problems: HashSet<(String, String)> = HashSet::new();

    for (current_idx, active_id) in active_package_ids.iter().enumerate() {
        let active_lower = active_id.to_lowercase();
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
    }

    report
}
