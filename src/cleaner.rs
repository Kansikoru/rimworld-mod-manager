use crate::models::{is_protected_package_id, ModInfo};
use crate::steam::{
    remove_workshop_item_from_acf, unsubscribe_workshop_item_native,
    unsubscribe_workshop_items_batch_native, SteamPaths,
};

/// Moves a mod folder to the Windows Recycle Bin so the user can restore it.
/// Never falls back to permanent deletion.
pub fn move_mod_dir_to_trash(dir: &std::path::Path) -> Result<(), String> {
    trash::delete(dir).map_err(|e| format!("Не удалось переместить {:?} в Корзину: {}", dir, e))
}

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

    let mut allowed: Vec<&ModInfo> = Vec::new();
    for m in mods_to_delete {
        if is_protected_package_id(&m.package_id) || m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
            result.errors.push(format!(
                "Официальный модуль или защищённый мод {} заблокирован от удаления!",
                m.package_id
            ));
            result.failed_count += 1;
        } else if !m.is_translation_mod {
            result.errors.push(format!(
                "Мод {} не является модом-переводом и заблокирован от удаления!",
                m.name
            ));
            result.failed_count += 1;
        } else {
            allowed.push(m);
        }
    }

    // 1. Unsubscribe all of them in ONE Steam API session (S9), not one session per mod.
    let steam_ids: Vec<&str> = allowed.iter().filter_map(|m| m.steam_item_id.as_deref()).collect();
    if let Some(ref dll_path) = steam_paths.steam_dll_path {
        match unsubscribe_workshop_items_batch_native(dll_path, &steam_ids) {
            Ok(n) => result.unsubscribed_count = n,
            Err(e) => result.errors.push(format!("Ошибка отписки в Steam: {}", e)),
        }
    }

    for m in allowed {
        // 2. Remove from ACF manifest
        if let (Some(item_id), Some(acf_path)) = (&m.steam_item_id, &steam_paths.acf_path) {
            let _ = remove_workshop_item_from_acf(acf_path, item_id);
        }

        // 3. Move directory to the Recycle Bin
        if m.dir_path.is_dir() {
            match move_mod_dir_to_trash(&m.dir_path) {
                Ok(_) => {
                    result.deleted_count += 1;
                }
                Err(e) => {
                    result.errors.push(e);
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
    if is_protected_package_id(&m.package_id) || m.package_id.to_lowercase().starts_with("ludeon.rimworld") {
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
        move_mod_dir_to_trash(&m.dir_path)?;
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
            match move_mod_dir_to_trash(&m.dir_path) {
                Ok(_) => result.deleted_count += 1,
                Err(e) => {
                    result.errors.push(e);
                    result.failed_count += 1;
                }
            }
        }
    }
    result
}
