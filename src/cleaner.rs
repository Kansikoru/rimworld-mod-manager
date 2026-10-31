use crate::i18n::tr;
use crate::models::{is_protected_package_id, ModInfo};
use crate::steam::{unsubscribe_workshop_items_batch_native, SteamPaths};

/// Moves a mod folder to the Windows Recycle Bin so the user can restore it.
/// Never falls back to permanent deletion.
pub fn move_mod_dir_to_trash(dir: &std::path::Path) -> Result<(), String> {
    // A link in Mods (SteamCMD puts one per mod): the link is removed; the files go to the
    // Recycle Bin only when they are SteamCMD's download, never someone else's folder.
    if let Ok(target) = std::fs::read_link(dir) {
        std::fs::remove_dir(dir).map_err(|e| {
            crate::i18n::fmt(
                crate::i18n::text(crate::i18n::TranslationKey::SteamCmdLinkRemoveError),
                &[&dir.display().to_string(), &e.to_string()],
            )
        })?;
        let target = crate::steamcmd::plain_path(&target);
        if crate::steamcmd::is_steamcmd_item(&target) {
            return move_mod_dir_to_trash(&target);
        }
        return Ok(());
    }
    trash::delete(dir).map_err(|e| {
        tr!(
            "Не удалось переместить {} в Корзину: {}",
            "Could not move {} to the Recycle Bin: {}",
            "无法将 {} 移至回收站：{}",
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

fn is_protected(m: &ModInfo) -> bool {
    is_protected_package_id(&m.package_id) || m.package_id.to_lowercase().starts_with("ludeon.rimworld")
}

/// Unsubscribes Workshop items in one Steam API session. A Workshop mod is only deleted
/// after this succeeds: otherwise Steam would simply download it again.
fn unsubscribe(item_ids: &[&str], steam_paths: &SteamPaths) -> Result<usize, String> {
    if item_ids.is_empty() {
        return Ok(0);
    }
    let dll = steam_paths.steam_dll_path.as_ref().ok_or_else(|| {
        tr!(
            "steam_api64.dll не найден, отписаться нельзя",
            "steam_api64.dll not found, can't unsubscribe",
            "找不到 steam_api64.dll，无法取消订阅"
        )
        .to_string()
    })?;
    unsubscribe_workshop_items_batch_native(dll, item_ids)
}

fn not_deleted_because(err: &str) -> String {
    tr!(
        "Отписка в Steam не удалась ({}). Моды из Workshop не удалены: иначе Steam скачает их обратно. Запустите Steam и повторите.",
        "Steam unsubscribe failed ({}). Workshop mods were not deleted: Steam would download them again. Start Steam and try again.", "Steam 取消订阅失败（{}）。创意工坊模组没有删除：否则 Steam 会重新下载它们。请启动 Steam 后重试。",
        err
    )
}

pub fn delete_translation_mods(mods_to_delete: &[&ModInfo], steam_paths: &SteamPaths) -> CleanResult {
    let mut result = CleanResult::default();

    let mut allowed: Vec<&ModInfo> = Vec::new();
    for m in mods_to_delete {
        if is_protected(m) {
            result.errors.push(tr!(
                "Защищённый мод '{}' нельзя удалить",
                "Protected mod '{}' can't be deleted",
                "受保护的模组「{}」不能删除",
                m.package_id
            ));
            result.failed_count += 1;
        } else if !m.is_translation_mod {
            result.errors.push(tr!(
                "Мод {} не перевод — не удалён",
                "Mod {} is not a translation — not deleted",
                "模组 {} 不是翻译 — 未删除",
                m.name
            ));
            result.failed_count += 1;
        } else {
            allowed.push(m);
        }
    }

    let steam_ids: Vec<&str> = allowed.iter().filter_map(|m| m.steam_item_id.as_deref()).collect();
    let workshop_ok = match unsubscribe(&steam_ids, steam_paths) {
        Ok(n) => {
            result.unsubscribed_count = n;
            true
        }
        Err(e) => {
            result.errors.push(not_deleted_because(&e));
            false
        }
    };

    for m in allowed {
        if m.steam_item_id.is_some() && !workshop_ok {
            result.failed_count += 1;
            continue;
        }
        if m.dir_path.is_dir() {
            match move_mod_dir_to_trash(&m.dir_path) {
                Ok(()) => result.deleted_count += 1,
                Err(e) => {
                    result.errors.push(e);
                    result.failed_count += 1;
                }
            }
        }
    }

    result
}

/// Unsubscribes from one mod (if it comes from the Workshop) and moves its folder to the
/// Recycle Bin. Official content is never deleted.
pub fn unsubscribe_and_delete_single(m: &ModInfo, steam_paths: &SteamPaths) -> Result<(), String> {
    if is_protected(m) {
        return Err(tr!(
            "Защищённый мод '{}' нельзя удалить",
            "Protected mod '{}' can't be deleted",
            "受保护的模组「{}」不能删除",
            m.package_id
        ));
    }

    if let Some(ref item_id) = m.steam_item_id {
        unsubscribe(&[item_id.as_str()], steam_paths).map_err(|e| not_deleted_because(&e))?;
    }

    if m.dir_path.is_dir() {
        move_mod_dir_to_trash(&m.dir_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workshop_mod_is_kept_when_unsubscribe_is_impossible() {
        let dir = std::env::temp_dir().join(format!("rwmm_clean_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = ModInfo {
            package_id: "someone.translation".into(),
            name: "Some Translation".into(),
            dir_path: dir.clone(),
            steam_item_id: Some("1234567890".into()),
            is_translation_mod: true,
            ..Default::default()
        };
        let no_steam = SteamPaths::default();

        assert!(unsubscribe_and_delete_single(&m, &no_steam).is_err());
        let res = delete_translation_mods(&[&m], &no_steam);
        assert_eq!((res.deleted_count, res.failed_count), (0, 1));
        assert!(dir.is_dir(), "folder must stay when Steam can't unsubscribe");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
