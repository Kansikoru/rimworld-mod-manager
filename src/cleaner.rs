use crate::i18n::tr;
use crate::models::{is_protected_package_id, ModInfo};
use crate::steam::{
    remove_workshop_item_from_acf, unsubscribe_workshop_item_native,
    unsubscribe_workshop_items_batch_native, SteamPaths,
};

/// Moves a mod folder to the Windows Recycle Bin so the user can restore it.
/// Never falls back to permanent deletion.
pub fn move_mod_dir_to_trash(dir: &std::path::Path) -> Result<(), String> {
    trash::delete(dir).map_err(|e| {
        tr!(
            "Не удалось переместить {} в Корзину: {}",
            "Could not move {} to the Recycle Bin: {}",
            dir.display(),
            e
        )
    })
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
            result.errors.push(tr!(
                "Защищённый мод '{}' нельзя удалить",
                "Protected mod '{}' can't be deleted",
                m.package_id
            ));
            result.failed_count += 1;
        } else if !m.is_translation_mod {
            result.errors.push(tr!(
                "Мод {} не перевод — не удалён",
                "Mod {} is not a translation — not deleted",
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
            Err(e) => result.errors.push(tr!("Ошибка отписки в Steam: {}", "Steam unsubscribe failed: {}", e)),
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
        return Err(tr!(
            "Защищённый мод '{}' нельзя удалить",
            "Protected mod '{}' can't be deleted",
            m.package_id
        ));
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
