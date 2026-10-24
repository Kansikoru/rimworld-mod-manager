use crate::models::{is_protected_package_id, ModInfo};
use crate::steam::{remove_workshop_item_from_acf, unsubscribe_workshop_item_native, SteamPaths};

#[derive(Debug, Default)]
pub struct CleanResult {
    pub deleted_count: usize,
    pub unsubscribed_count: usize,
    pub failed_count: usize,
    pub errors: Vec<String>,
}

pub fn delete_translation_mods(
    mods_to_delete: &[&ModInfo],
    steam_paths: &SteamPaths,
) -> CleanResult {
    let mut result = CleanResult::default();

    for m in mods_to_delete {
        if is_protected_package_id(&m.package_id) || m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
            result.errors.push(format!(
                "Официальный модуль или защищённый мод {} заблокирован от удаления!",
                m.package_id
            ));
            result.failed_count += 1;
            continue;
        }

        if !m.is_translation_mod {
            result.errors.push(format!(
                "Мод {} не является модом-переводом и заблокирован от удаления!",
                m.name
            ));
            result.failed_count += 1;
            continue;
        }

        // 1. Steam Workshop unsubscribe
        if let Some(ref item_id) = m.steam_item_id {
            if let Some(ref dll_path) = steam_paths.steam_dll_path {
                match unsubscribe_workshop_item_native(dll_path, item_id) {
                    Ok(unsub) => {
                        if unsub {
                            result.unsubscribed_count += 1;
                        }
                    }
                    Err(e) => {
                        result
                            .errors
                            .push(format!("Ошибка отписки в Steam для {}: {}", m.name, e));
                    }
                }
            }

            // 2. Remove from ACF manifest
            if let Some(ref acf_path) = steam_paths.acf_path {
                let _ = remove_workshop_item_from_acf(acf_path, item_id);
            }
        }

        // 3. Delete directory from disk
        if m.dir_path.is_dir() {
            match std::fs::remove_dir_all(&m.dir_path) {
                Ok(_) => {
                    result.deleted_count += 1;
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Не удалось удалить папку {:?}: {}", m.dir_path, e));
                    result.failed_count += 1;
                }
            }
        }
    }

    result
}

/// Unsubscribes from a single mod in Steam Workshop and deletes its local folder.
/// Protects system/DLC mods from deletion.
pub fn unsubscribe_and_delete_single(m: &ModInfo, steam_paths: &SteamPaths) -> Result<(), String> {
    if is_protected_package_id(&m.package_id) {
        return Err(format!("Защищённый мод '{}' нельзя удалить!", m.package_id));
    }

    if let Some(ref item_id) = m.steam_item_id {
        if let Some(ref dll_path) = steam_paths.steam_dll_path {
            let _ = unsubscribe_workshop_item_native(dll_path, item_id);
        }
        if let Some(ref acf_path) = steam_paths.acf_path {
            let _ = remove_workshop_item_from_acf(acf_path, item_id);
        }
    }

    if m.dir_path.is_dir() {
        std::fs::remove_dir_all(&m.dir_path)
            .map_err(|e| format!("Не удалось удалить папку {:?}: {}", m.dir_path, e))?;
    }

    Ok(())
}

/// Unsubscribes from and deletes multiple selected mods in batch.
pub fn unsubscribe_and_delete_batch(mods: &[&ModInfo], steam_paths: &SteamPaths) -> CleanResult {
    let mut result = CleanResult::default();
    for m in mods {
        if is_protected_package_id(&m.package_id) || m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
            result.errors.push(format!(
                "Официальный модуль или защищённый мод {} заблокирован от удаления!",
                m.package_id
            ));
            result.failed_count += 1;
            continue;
        }

        let mut unsub_ok = false;
        if let Some(ref item_id) = m.steam_item_id {
            if let Some(ref dll_path) = steam_paths.steam_dll_path {
                if let Ok(unsub) = unsubscribe_workshop_item_native(dll_path, item_id) {
                    if unsub {
                        unsub_ok = true;
                    }
                }
            }
            if let Some(ref acf_path) = steam_paths.acf_path {
                let _ = remove_workshop_item_from_acf(acf_path, item_id);
            }
        }
        if unsub_ok {
            result.unsubscribed_count += 1;
        }

        if m.dir_path.is_dir() {
            match std::fs::remove_dir_all(&m.dir_path) {
                Ok(_) => result.deleted_count += 1,
                Err(e) => {
                    result
                        .errors
                        .push(format!("Ошибка удаления {:?}: {}", m.dir_path, e));
                    result.failed_count += 1;
                }
            }
        }
    }
    result
}
