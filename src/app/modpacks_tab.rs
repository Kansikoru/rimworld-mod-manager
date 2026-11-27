//! Modpacks tab (F4): mod sets of the game's ModLists folder, their pictures, tags, mod
//! settings, comparing, sharing and importing them.

use super::*;

/// State of the Modpacks tab.
pub struct ModpacksState {
    /// Mod sets from the game's ModLists folder (F4), by name.
    pub lists: Vec<crate::modlists::ModList>,
    /// Card pictures of the sets by name: the texture and whether it is the user's own
    /// (else a generated placeholder). Filled by the Modpacks tab.
    pub pictures: HashMap<String, (egui::TextureHandle, bool)>,
    /// Tags of the sets by name (`modlist_tags.json`).
    pub tags: crate::modlist_tags::Tags,
    /// Modpacks tab: show only sets with any of these tags (lower case).
    pub tag_filter: std::collections::HashSet<String>,
    /// The set last loaded or saved here.
    pub current: Option<String>,
    /// Set selected on the Sets tab, and its search query.
    pub selected: Option<String>,
    pub search: String,
    pub dialog: Option<ModListDialog>,
    pub(super) collection_rx: Option<Receiver<Result<crate::mod_updates::RemoteCollection, String>>>,
    /// Mod settings kept with the sets, by set name (`modlist_settings.rs`).
    pub settings: HashMap<String, crate::modlist_settings::Saved>,
    /// Modpacks tab: the "Compare" window.
    pub compare: Option<SetCompare>,
}

impl App {
    /// The game's ModLists folder (next to the Config folder with ModsConfig.xml).
    pub fn modlists_folder(&self) -> Option<PathBuf> {
        self.mods_config_path.as_deref().and_then(crate::modlists::modlists_dir)
    }

    pub fn refresh_modlists(&mut self) {
        self.modpacks.lists = self
            .modlists_folder()
            .map(|dir| crate::modlists::list_modlists(&dir))
            .unwrap_or_default();
        if let Some(cur) = &self.modpacks.current {
            if !self.modpacks.lists.iter().any(|l| &l.name == cur) {
                self.modpacks.current = None;
            }
        }
        if let Some(sel) = &self.modpacks.selected {
            if !self.modpacks.lists.iter().any(|l| &l.name == sel) {
                self.modpacks.selected = None;
            }
        }
        let kept = crate::modlist_settings::root();
        self.modpacks.settings = self
            .modpacks
            .lists
            .iter()
            .filter_map(|l| crate::modlist_settings::saved(&kept, &l.name).map(|s| (l.name.clone(), s)))
            .collect();
    }

    /// "Enable": writes the set to ModsConfig.xml (a backup of the old order is made),
    /// and starts the game too if asked.
    pub fn apply_modlist(&mut self, name: &str, launch: bool) {
        // The modpacks the game loads until now: the mods' settings in use are theirs.
        let live: Vec<String> = self
            .modpacks
            .lists
            .iter()
            .filter(|l| self.modlist_diff(l).is_same())
            .map(|l| l.name.clone())
            .collect();
        self.load_modlist(name);
        if self.save_config() {
            let mut message = fmt(self.i18n.get(TranslationKey::SetsApplied), &[name]);
            // Enabling the modpack that is on already changes nothing, its settings neither.
            if !live.iter().any(|l| l == name) {
                if let Some(settings) = self.switch_modlist_settings(&live, name) {
                    message.push_str(" | ");
                    message.push_str(&settings);
                }
            }
            self.status_message = message;
            if launch {
                self.check_vram_and_launch();
            }
        }
    }

    /// Makes the textures of set pictures that are not loaded yet: the user's own picture,
    /// else a placeholder generated from the name. Small files, read once per set.
    pub fn load_modlist_pictures(&mut self, ctx: &egui::Context) {
        let dir = crate::modlist_images::images_dir();
        for list in &self.modpacks.lists {
            if self.modpacks.pictures.contains_key(&list.name) {
                continue;
            }
            let own = crate::modlist_images::load_image(&dir, &list.name);
            let custom = own.is_some();
            let img = own.unwrap_or_else(|| crate::modlist_images::placeholder(&list.name));
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            let tex = ctx.load_texture(
                format!("modlist_picture/{}", list.name),
                color,
                egui::TextureOptions::LINEAR,
            );
            self.modpacks.pictures.insert(list.name.clone(), (tex, custom));
        }
    }

    /// Makes a picture file the card picture of a set.
    pub fn set_modlist_picture(&mut self, name: &str, source: &Path) {
        match crate::modlist_images::set_image(&crate::modlist_images::images_dir(), name, source) {
            Ok(()) => {
                self.modpacks.pictures.remove(name);
            }
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsPictureError), &[&e]),
        }
    }

    /// Back to the generated placeholder.
    pub fn remove_modlist_picture(&mut self, name: &str) {
        crate::modlist_images::remove_image(&crate::modlist_images::images_dir(), name);
        self.modpacks.pictures.remove(name);
    }

    /// Turns a tag on or off for a set and saves the tags.
    pub fn toggle_modlist_tag(&mut self, set: &str, tag: &str) {
        crate::modlist_tags::toggle(&mut self.modpacks.tags, set, tag);
        self.save_modlist_tags();
    }

    pub(super) fn save_modlist_tags(&mut self) {
        if let Err(e) = crate::modlist_tags::save(&crate::modlist_tags::tags_path(), &self.modpacks.tags) {
            self.status_message = fmt(self.i18n.get(TranslationKey::SetsTagsSaveError), &[&e.to_string()]);
        }
    }

    /// Copies a set to a free "Name (copy)" and selects the copy.
    pub fn duplicate_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        let copy = crate::modlists::copy_name(&dir, name, self.i18n.get(TranslationKey::SetsCopyWord));
        match std::fs::copy(
            crate::modlists::path_for(&dir, name),
            crate::modlists::path_for(&dir, &copy),
        ) {
            Ok(_) => {
                crate::modlist_images::copy_image(&crate::modlist_images::images_dir(), name, &copy);
                crate::modlist_tags::copy(&mut self.modpacks.tags, name, &copy);
                crate::modlist_settings::copy(&crate::modlist_settings::root(), name, &copy);
                self.save_modlist_tags();
                self.refresh_modlists();
                self.modpacks.selected = Some(copy.clone());
                self.status_message = fmt(self.i18n.get(TranslationKey::SetsDuplicated), &[name, &copy]);
            }
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// "Merge with…" (F9 of the plan): a new set "A + B" with the mods of both in load
    /// order; the two sets stay as they are.
    pub fn merge_modlists(&mut self, first: &str, second: &str) {
        let t = self.i18n;
        let find = |name: &str| self.modpacks.lists.iter().find(|l| l.name == name).cloned();
        let (Some(dir), Some(a), Some(b)) = (self.modlists_folder(), find(first), find(second)) else {
            return;
        };
        let mut entries = crate::modlists::merge(&a, &b);
        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let place: HashMap<String, usize> = sort_load_order_with_db(&ids, &self.all_mods, Some(&self.community_db))
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id.to_lowercase(), i))
            .collect();
        entries.sort_by_key(|e| place.get(&e.id).copied().unwrap_or(usize::MAX));
        let name = crate::modlists::unique_name(&dir, &format!("{first} + {second}"));
        let path = crate::modlists::path_for(&dir, &name);
        match crate::modlists::write_rml(&path, &self.game_version_for_sets(), &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.modpacks.selected = Some(name.clone());
                self.status_message = fmt(
                    t.get(TranslationKey::SetsMerged),
                    &[first, second, &name, &entries.len().to_string()],
                );
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// The current load order with names and Workshop IDs, as the game writes it.
    pub(super) fn modlist_entries(&self) -> Vec<crate::modlists::ModListEntry> {
        self.active_mods
            .iter()
            .map(|id| {
                let m = self.find_mod(id);
                // A mod not on disk keeps the Workshop ID a set remembers for it,
                // so "subscribe to missing" still works for the new file.
                let steam_id = m.and_then(|m| m.steam_item_id.clone()).or_else(|| {
                    self.modpacks
                        .lists
                        .iter()
                        .find_map(|l| l.steam_id(id).map(str::to_string))
                });
                crate::modlists::ModListEntry {
                    id: id.to_lowercase(),
                    name: m.map(|m| m.name.clone()).unwrap_or_else(|| id.clone()),
                    steam_id,
                }
            })
            .collect()
    }

    pub(super) fn game_version_for_sets(&self) -> String {
        self.steam_paths.game_version.clone().unwrap_or_else(|| "1.6".into())
    }

    pub(super) fn name_problem_text(&self, problem: crate::modlists::NameProblem) -> String {
        use crate::modlists::NameProblem;
        self.i18n
            .get(match problem {
                NameProblem::Empty => TranslationKey::SetsNameEmpty,
                NameProblem::BadCharacters => TranslationKey::SetsNameBadChars,
                NameProblem::TooLong => TranslationKey::SetsNameTooLong,
            })
            .to_string()
    }

    /// Loads a set into the list as unsaved changes: ModsConfig.xml changes only on "Save Order".
    pub fn load_modlist(&mut self, name: &str) {
        let Some(list) = self.modpacks.lists.iter().find(|l| l.name == name).cloned() else {
            return;
        };
        let mut ids = Vec::with_capacity(list.ids.len());
        for id in &list.ids {
            // A collection imported before the mod was on disk lists it as "workshop.<ID>":
            // once it's downloaded, the set enables it under its real ID (the game knows no other).
            let real = id
                .starts_with("workshop.")
                .then(|| self.find_mod(id).map(|m| m.package_id.clone()))
                .flatten();
            add_active_id(&mut ids, real.as_deref().unwrap_or(id));
        }
        self.skip_save_check = true;
        self.active_mods = ids;
        self.validate_deps();
        self.modpacks.current = Some(list.name.clone());
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsLoaded), &[&list.name]);
    }

    /// Saves the current load order as a new set. The error text is shown in the dialog.
    pub fn save_modlist_as(&mut self, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self
            .modlists_folder()
            .ok_or_else(|| self.i18n.get(TranslationKey::SetsNoFolder).to_string())?;
        let path = crate::modlists::path_for(&dir, &name);
        if path.exists() {
            return Err(self.i18n.get(TranslationKey::SetsNameExists).to_string());
        }
        self.write_modlist(&name, &path)
    }

    /// Overwrites an existing set with the current load order.
    pub fn overwrite_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        let path = crate::modlists::path_for(&dir, name);
        if let Err(e) = self.write_modlist(name, &path) {
            self.status_message = e;
        }
    }

    pub(super) fn write_modlist(&mut self, name: &str, path: &std::path::Path) -> Result<(), String> {
        let entries = self.modlist_entries();
        crate::modlists::write_rml(path, &self.game_version_for_sets(), &entries)
            .map_err(|e| fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]))?;
        self.refresh_modlists();
        self.modpacks.current = Some(name.to_string());
        self.status_message = fmt(
            self.i18n.get(TranslationKey::SetsSaved),
            &[name, &entries.len().to_string()],
        );
        Ok(())
    }

    pub fn rename_modlist(&mut self, from: &str, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self
            .modlists_folder()
            .ok_or_else(|| self.i18n.get(TranslationKey::SetsNoFolder).to_string())?;
        let target = crate::modlists::path_for(&dir, &name);
        // Changing only the letter case is fine; another existing set is not.
        if target.exists() && !name.eq_ignore_ascii_case(from) {
            return Err(self.i18n.get(TranslationKey::SetsNameExists).to_string());
        }
        std::fs::rename(crate::modlists::path_for(&dir, from), &target)
            .map_err(|e| fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]))?;
        if self.modpacks.current.as_deref() == Some(from) {
            self.modpacks.current = Some(name.clone());
        }
        crate::modlist_images::rename_image(&crate::modlist_images::images_dir(), from, &name);
        self.modpacks.pictures.remove(from);
        self.modpacks.pictures.remove(&name);
        crate::modlist_tags::rename(&mut self.modpacks.tags, from, &name);
        crate::modlist_settings::rename(&crate::modlist_settings::root(), from, &name);
        self.save_modlist_tags();
        self.refresh_modlists();
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsRenamed), &[from, &name]);
        Ok(())
    }

    /// Moves a set file to the Recycle Bin. Mods and ModsConfig.xml are not touched.
    pub fn delete_modlist(&mut self, name: &str) {
        let Some(dir) = self.modlists_folder() else { return };
        match trash::delete(crate::modlists::path_for(&dir, name)) {
            Ok(()) => {
                crate::modlist_images::remove_image(&crate::modlist_images::images_dir(), name);
                crate::modlist_settings::remove(&crate::modlist_settings::root(), name);
                self.modpacks.pictures.remove(name);
                if self.modpacks.tags.remove(name).is_some() {
                    self.save_modlist_tags();
                }
                self.status_message = fmt(self.i18n.get(TranslationKey::SetsDeleted), &[name]);
            }
            Err(e) => self.status_message = fmt(self.i18n.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
        self.refresh_modlists();
    }

    /// How the set differs from what the game loads now (ModsConfig.xml on disk).
    pub fn modlist_diff(&self, list: &crate::modlists::ModList) -> crate::mods_config::ListDiff {
        crate::mods_config::diff_lists(&self.saved_active_mods, &list.ids)
    }

    /// Mods of the set that are not on disk.
    pub fn modlist_missing(&self, list: &crate::modlists::ModList) -> usize {
        list.ids.iter().filter(|id| self.find_mod(id).is_none()).count()
    }

    /// Workshop IDs of the set's mods that are not on disk (local mods and DLC have none).
    pub fn modlist_missing_workshop_ids(&self, list: &crate::modlists::ModList) -> Vec<String> {
        let mut ids: Vec<String> = list
            .ids
            .iter()
            .enumerate()
            .filter(|(_, id)| self.find_mod(id).is_none())
            .filter_map(|(i, id)| self.workshop_id_for(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
            .filter(|ws| !self.workshop_item_gone(ws))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// F6: subscribes to every missing mod of the set that has a Workshop ID, in one
    /// Steam session in the background. Steam downloads them; then the user rescans.
    pub fn subscribe_modlist_missing(&mut self, name: &str) {
        if self.steam_rx.is_some() {
            return;
        }
        let Some(list) = self.modpacks.lists.iter().find(|l| l.name == name) else {
            return;
        };
        let ids = self.modlist_missing_workshop_ids(list);
        self.subscribe_workshop_ids(ids);
    }

    /// The set's missing mods that have no Workshop link (`only`: just this one).
    pub fn modlist_wanted(
        &self,
        list: &crate::modlists::ModList,
        only: Option<&str>,
    ) -> Vec<crate::name_search::Wanted> {
        list.ids
            .iter()
            .enumerate()
            .filter(|(_, id)| only.is_none_or(|o| o.eq_ignore_ascii_case(id)))
            .filter_map(|(i, id)| self.wanted(id, list.steam_id(id), list.names.get(i).map(String::as_str)))
            .collect()
    }

    // ----- Mod settings kept with a modpack (`modlist_settings.rs`) -----

    /// The game's Config folder: ModsConfig.xml and the mods' `Mod_*.xml` settings.
    pub(super) fn game_config_dir(&self) -> Option<PathBuf> {
        self.mods_config_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// Takes a copy of the mods' settings for the modpack (a new one replaces the old).
    pub fn save_modlist_settings(&mut self, name: &str) {
        let t = self.i18n;
        let Some(config) = self.game_config_dir() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        self.status_message = match crate::modlist_settings::save(&crate::modlist_settings::root(), name, &config) {
            Ok(n) => fmt(t.get(TranslationKey::SetsSettingsSavedMsg), &[name, &n.to_string()]),
            Err(e) => fmt(t.get(TranslationKey::SetsSettingsError), &[&e]),
        };
        self.refresh_modlists();
    }

    /// Puts the modpack's copy of the mods' settings back; says what happened.
    pub(super) fn restore_modlist_settings_text(&self, name: &str) -> Option<String> {
        let t = self.i18n;
        let config = self.game_config_dir()?;
        let backup =
            crate::modlist_settings::backup_root().join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
        Some(
            match crate::modlist_settings::restore(&crate::modlist_settings::root(), name, &config, &backup) {
                Ok(done) if done.written == 0 => t.get(TranslationKey::SetsSettingsSameMsg).to_string(),
                Ok(done) => {
                    let mut msg = fmt(
                        t.get(TranslationKey::SetsSettingsRestoredMsg),
                        &[&done.written.to_string()],
                    );
                    if done.backed_up > 0 {
                        msg.push_str(" | ");
                        msg.push_str(&fmt(
                            t.get(TranslationKey::SetsZipBackup),
                            &[&done.backed_up.to_string(), &backup.display().to_string()],
                        ));
                    }
                    msg
                }
                Err(e) => fmt(t.get(TranslationKey::SetsSettingsError), &[&e]),
            },
        )
    }

    pub fn restore_modlist_settings(&mut self, name: &str) {
        if let Some(msg) = self.restore_modlist_settings_text(name) {
            self.status_message = msg;
        }
    }

    pub fn forget_modlist_settings(&mut self, name: &str) {
        crate::modlist_settings::remove(&crate::modlist_settings::root(), name);
        self.status_message = fmt(self.i18n.get(TranslationKey::SetsSettingsForgotMsg), &[name]);
        self.refresh_modlists();
    }

    /// Enabling a modpack with kept settings: the modpacks the game ran until now (`live`)
    /// get a fresh copy of the settings in use, so nothing set while playing them is lost,
    /// and the new modpack's copy is put back. Returns what to tell the player.
    pub(super) fn switch_modlist_settings(&mut self, live: &[String], name: &str) -> Option<String> {
        self.modpacks.settings.get(name)?;
        if let Some(config) = self.game_config_dir() {
            let kept = crate::modlist_settings::root();
            for other in live.iter().filter(|l| self.modpacks.settings.contains_key(*l)) {
                let _ = crate::modlist_settings::save(&kept, other, &config);
            }
        }
        let msg = self.restore_modlist_settings_text(name);
        self.refresh_modlists();
        msg
    }

    // ----- Comparing a modpack with another list -----

    /// IDs and names of the list a modpack is compared with.
    pub(super) fn compare_side(&self, with: &CompareWith) -> Option<(Vec<String>, Vec<String>)> {
        match with {
            CompareWith::Current => Some((self.active_mods.clone(), Vec::new())),
            CompareWith::Modpack(name) => self
                .modpacks
                .lists
                .iter()
                .find(|l| &l.name == name)
                .map(|l| (l.ids.clone(), l.names.clone())),
            CompareWith::Save(name) => self
                .saves
                .list
                .iter()
                .find(|s| &s.name == name)
                .map(|s| (s.ids.clone(), s.names.clone())),
        }
    }

    /// Opens the "Compare" window (or changes what the modpack is compared with).
    pub fn open_set_compare(&mut self, name: &str, with: CompareWith) {
        let Some(list) = self.modpacks.lists.iter().find(|l| l.name == name) else {
            return;
        };
        let Some((ids, their_names)) = self.compare_side(&with) else {
            return;
        };
        let mut names = HashMap::new();
        for (ids, known) in [(&list.ids, &list.names), (&ids, &their_names)] {
            for (i, id) in ids.iter().enumerate() {
                let name = self
                    .find_mod(id)
                    .map(|m| m.name.clone())
                    .or_else(|| known.get(i).filter(|n| !n.trim().is_empty()).cloned())
                    .unwrap_or_else(|| id.clone());
                names.entry(id.to_lowercase()).or_insert(name);
            }
        }
        self.modpacks.compare = Some(SetCompare {
            first: name.to_string(),
            result: crate::modlists::compare(&list.ids, &ids),
            with,
            names,
        });
    }

    // ----- Sharing modpacks -----

    /// F5: adds a shared `.rml`, `ModsConfig.xml` or `.zip` pack as a new set (a free name is picked)
    /// and selects it. Names and Workshop IDs come from the file, else from installed mods.
    pub fn import_modlist(&mut self, file: &Path) {
        let t = self.i18n;
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let file_name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_zip = file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let (list, zip_configs) = if is_zip {
            let Some(pack) = crate::modlists::read_zip_pack(file) else {
                self.status_message = fmt(t.get(TranslationKey::SetsImportBad), &[&file_name]);
                return;
            };
            (pack.list, Some(pack.configs))
        } else {
            let Some(list) = crate::modlists::read_shared(file) else {
                self.status_message = fmt(t.get(TranslationKey::SetsImportBad), &[&file_name]);
                return;
            };
            (list, None)
        };
        let base = crate::modlists::check_name(&list.name)
            .unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let name = crate::modlists::unique_name(&dir, &base);
        let entries: Vec<crate::modlists::ModListEntry> = list
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let m = self.find_mod(id);
                crate::modlists::ModListEntry {
                    id: id.to_lowercase(),
                    name: m
                        .map(|m| m.name.clone())
                        .or_else(|| list.names.get(i).cloned())
                        .unwrap_or_else(|| id.clone()),
                    steam_id: list
                        .steam_id(id)
                        .map(str::to_string)
                        .or_else(|| m.and_then(|m| m.steam_item_id.clone())),
                }
            })
            .collect();
        let version = list
            .game_version
            .clone()
            .unwrap_or_else(|| self.game_version_for_sets());
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &name), &version, &entries) {
            Ok(()) => {
                let mut written_configs = 0usize;
                // The player's own settings of these mods are copied away before the pack's
                // replace them (a different file only), so nothing is lost.
                let backup_dir = crate::game_install::manager_data_dir()
                    .join("mod_settings_backup")
                    .join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
                let mut backed_up = 0usize;
                if let (Some(cfgs), Some(cfg_dir)) = (
                    zip_configs.as_ref(),
                    self.mods_config_path.as_deref().and_then(Path::parent),
                ) {
                    let _ = std::fs::create_dir_all(cfg_dir);
                    for (cfg_name, bytes) in cfgs {
                        if !crate::modlists::is_mod_config_name(cfg_name) {
                            continue;
                        }
                        let target = cfg_dir.join(cfg_name);
                        if let Ok(old) = std::fs::read(&target) {
                            if &old == bytes {
                                continue;
                            }
                            let saved = std::fs::create_dir_all(&backup_dir)
                                .and_then(|_| std::fs::write(backup_dir.join(cfg_name), &old))
                                .is_ok();
                            if !saved {
                                continue;
                            }
                            backed_up += 1;
                        }
                        if std::fs::write(&target, bytes).is_ok() {
                            written_configs += 1;
                        }
                    }
                }
                self.refresh_modlists();
                self.modpacks.selected = Some(name.clone());
                self.status_message = if zip_configs.is_some() {
                    let mut msg = fmt(
                        t.get(TranslationKey::SetsImportedZip),
                        &[&name, &entries.len().to_string(), &written_configs.to_string()],
                    );
                    if backed_up > 0 {
                        msg.push_str(" | ");
                        msg.push_str(&fmt(
                            t.get(TranslationKey::SetsZipBackup),
                            &[&backed_up.to_string(), &backup_dir.display().to_string()],
                        ));
                    }
                    msg
                } else {
                    fmt(
                        t.get(TranslationKey::SetsImported),
                        &[&name, &entries.len().to_string()],
                    )
                };
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// F5: copies the set's `.rml` file (or exports a `.zip` pack with `Config/Mod_*.xml`) to where the user chose.
    pub fn export_modlist(&mut self, name: &str, target: &Path) {
        let t = self.i18n;
        let Some(list) = self.modpacks.lists.iter().find(|l| l.name == name) else {
            return;
        };
        let is_zip = target
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if is_zip {
            let rml = match std::fs::read_to_string(&list.path) {
                Ok(s) => s,
                Err(e) => {
                    self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]);
                    return;
                }
            };
            let cfg_dir = self.mods_config_path.as_deref().and_then(Path::parent);
            self.status_message = match crate::modlists::write_zip_pack(target, name, &rml, cfg_dir) {
                Ok(n) => fmt(
                    t.get(TranslationKey::SetsExportedZip),
                    &[name, &n.to_string(), &target.display().to_string()],
                ),
                Err(e) => fmt(t.get(TranslationKey::SetsWriteError), &[&e]),
            };
            return;
        }
        self.status_message = match std::fs::copy(&list.path, target) {
            Ok(_) => fmt(
                t.get(TranslationKey::SetsExported),
                &[name, &target.display().to_string()],
            ),
            Err(e) => fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        };
    }

    /// F5: starts importing a Steam Workshop collection (or a list of Workshop links) into a new set.
    pub fn import_steam_collection(&mut self, raw: &str) -> Result<(), String> {
        let t = self.i18n;
        if self.modlists_folder().is_none() {
            return Err(t.get(TranslationKey::SetsNoFolder).to_string());
        }
        if crate::mod_updates::extract_workshop_ids(raw).is_empty() {
            return Err(t.get(TranslationKey::SetsCollectionBadUrl).to_string());
        }
        if self.modpacks.collection_rx.is_some() {
            return Ok(());
        }
        let input = raw.to_string();
        self.status_message = t.get(TranslationKey::SetsCollectionLoading).to_string();
        self.modpacks.collection_rx =
            Some(self.spawn_task(move || crate::mod_updates::fetch_collection_or_items(&input)));
        Ok(())
    }

    pub(super) fn finish_collection_import(&mut self, res: Result<crate::mod_updates::RemoteCollection, String>) {
        let t = self.i18n;
        let col = match res {
            Ok(c) => c,
            Err(e) => {
                self.status_message = fmt(t.get(TranslationKey::SetsCollectionError), &[&e]);
                return;
            }
        };
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let base = crate::modlists::check_name(&col.title)
            .unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let name = crate::modlists::unique_name(&dir, &base);
        let entries: Vec<crate::modlists::ModListEntry> = col
            .items
            .iter()
            .map(|(ws_id, ws_title)| {
                let installed = self.all_mods.iter().find(|m| {
                    m.steam_item_id.as_deref() == Some(ws_id.as_str())
                        || m.dir_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n == ws_id)
                });
                let idx = self.workshop_index.as_ref();
                let id = installed
                    .map(|m| m.package_id.to_lowercase())
                    .or_else(|| idx.and_then(|i| i.package_for_id(ws_id)).map(str::to_string))
                    .unwrap_or_else(|| format!("workshop.{ws_id}"));
                let item_name = installed
                    .map(|m| m.name.clone())
                    .or_else(|| (!ws_title.trim().is_empty()).then(|| ws_title.clone()))
                    .or_else(|| idx.and_then(|i| i.name_for_id(ws_id)).map(str::to_string))
                    .unwrap_or_else(|| ws_id.clone());
                crate::modlists::ModListEntry {
                    id,
                    name: item_name,
                    steam_id: Some(ws_id.clone()),
                }
            })
            .collect();
        let entries = modpack_from_collection(entries, &self.all_mods, Some(&self.community_db));
        let version = self.game_version_for_sets();
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &name), &version, &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.modpacks.selected = Some(name.clone());
                self.status_message = fmt(
                    t.get(TranslationKey::SetsImported),
                    &[&name, &entries.len().to_string()],
                );
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    pub fn collection_busy(&self) -> bool {
        self.modpacks.collection_rx.is_some()
    }

    /// F5: the set as text with Workshop links, for a chat or forum.
    pub fn modlist_share_text(&self, list: &crate::modlists::ModList) -> String {
        let mods: Vec<(String, Option<String>)> = list
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let m = self.find_mod(id);
                let name = m
                    .map(|m| m.name.clone())
                    .or_else(|| list.names.get(i).cloned())
                    .unwrap_or_else(|| id.clone());
                let steam = list
                    .steam_id(id)
                    .map(str::to_string)
                    .or_else(|| m.and_then(|m| m.steam_item_id.clone()));
                (name, steam)
            })
            .collect();
        crate::modlists::share_text(&list.name, &mods)
    }

    // ----- Dialogs -----

    /// Window for a mod set action: name field (save as new, rename) or delete confirmation.
    pub(super) fn render_modlist_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.modpacks.dialog.clone() else {
            return;
        };
        let t = self.i18n;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut submit = false;
        let title = match &dialog {
            ModListDialog::SaveAs { .. } => t.get(TranslationKey::SetsDialogSaveTitle),
            ModListDialog::Rename { .. } => t.get(TranslationKey::SetsDialogRenameTitle),
            ModListDialog::Delete { .. } => t.get(TranslationKey::SetsDialogDeleteTitle),
            ModListDialog::Collection { .. } => t.get(TranslationKey::SetsDialogCollectionTitle),
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(400.0);
                match &mut dialog {
                    ModListDialog::SaveAs { name, error } | ModListDialog::Rename { name, error, .. } => {
                        ui.label(t.get(TranslationKey::SetsNameLabel));
                        let field = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                        if ui.memory(|m| m.focused().is_none()) {
                            field.request_focus();
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                        if let Some(e) = error {
                            ui.label(egui::RichText::new(e.as_str()).color(THEME.accent_rose).size(12.0));
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(t.get(TranslationKey::SetsHint))
                                    .color(THEME.text_muted)
                                    .size(12.0),
                            )
                            .wrap(),
                        );
                    }
                    ModListDialog::Delete { name } => {
                        ui.add(egui::Label::new(fmt(t.get(TranslationKey::SetsDialogDeleteBody), &[name])).wrap());
                    }
                    ModListDialog::Collection { url, error } => {
                        ui.label(t.get(TranslationKey::SetsCollectionLabel));
                        let field = ui.add(egui::TextEdit::singleline(url).desired_width(f32::INFINITY));
                        if ui.memory(|m| m.focused().is_none()) {
                            field.request_focus();
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                        if let Some(e) = error {
                            ui.label(egui::RichText::new(e.as_str()).color(THEME.accent_rose).size(12.0));
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(t.get(TranslationKey::SetsCollectionHint))
                                    .color(THEME.text_muted)
                                    .size(12.0),
                            )
                            .wrap(),
                        );
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (text, style) = match &dialog {
                        ModListDialog::SaveAs { .. } => (
                            t.get(TranslationKey::SetsBtnSave),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                        ModListDialog::Rename { .. } => (
                            t.get(TranslationKey::SetsBtnRename),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                        ModListDialog::Delete { .. } => (
                            t.get(TranslationKey::SetsBtnDelete),
                            crate::ui::components::ButtonStyle::Danger,
                        ),
                        ModListDialog::Collection { .. } => (
                            t.get(TranslationKey::SetsBtnImport),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                    };
                    if crate::ui::components::styled_button(ui, None, text, style).clicked() {
                        submit = true;
                    }
                    if crate::ui::components::secondary_button(ui, t.get(TranslationKey::BtnCancel)).clicked() {
                        close = true;
                    }
                });
            });

        if submit {
            let result = match &dialog {
                ModListDialog::SaveAs { name, .. } => self.save_modlist_as(name),
                ModListDialog::Rename { from, name, .. } => self.rename_modlist(from, name),
                ModListDialog::Delete { name } => {
                    self.delete_modlist(name);
                    Ok(())
                }
                ModListDialog::Collection { url, .. } => self.import_steam_collection(url),
            };
            self.modpacks.dialog = match (result, dialog) {
                (Ok(()), _) => None,
                (Err(e), ModListDialog::SaveAs { name, .. }) => Some(ModListDialog::SaveAs { name, error: Some(e) }),
                (Err(e), ModListDialog::Rename { from, name, .. }) => Some(ModListDialog::Rename {
                    from,
                    name,
                    error: Some(e),
                }),
                (Err(e), ModListDialog::Collection { url, .. }) => {
                    Some(ModListDialog::Collection { url, error: Some(e) })
                }
                (Err(_), other) => Some(other),
            };
        } else if close {
            self.modpacks.dialog = None;
        } else {
            self.modpacks.dialog = Some(dialog);
        }
    }
}
