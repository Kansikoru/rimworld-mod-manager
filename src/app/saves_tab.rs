//! Saves tab (F7): the game's saves, their mods, renaming and deleting them.

use super::*;

/// State of the Saves tab: headers of the game's saves, read in the background.
pub struct SavesState {
    pub list: Vec<crate::saves::SaveInfo>,
    pub(super) rx: Option<Receiver<Vec<crate::saves::SaveInfo>>>,
    pub(super) loaded_at: Option<std::time::Instant>,
    pub selected: Option<String>,
    pub search: String,
    pub dialog: Option<SaveDialog>,
}

impl App {
    /// The saves' headers were read.
    pub(super) fn poll_saves(&mut self) {
        if let Some(rx) = &self.saves.rx {
            if let Ok(saves) = rx.try_recv() {
                self.saves.rx = None;
                self.saves.list = saves;
                if let Some(sel) = &self.saves.selected {
                    if !self.saves.list.iter().any(|s| &s.name == sel) {
                        self.saves.selected = None;
                    }
                }
                self.name_search_when_ready();
            }
        }
    }

    /// The game's Saves folder (next to the Config folder with ModsConfig.xml).
    pub fn saves_folder(&self) -> Option<PathBuf> {
        self.mods_config_path.as_deref().and_then(crate::saves::saves_dir)
    }

    /// Reads the save headers in the background (the first read of big files can be slow).
    pub fn refresh_saves(&mut self) {
        if self.saves.rx.is_some() {
            return;
        }
        let Some(dir) = self.saves_folder() else { return };
        self.saves.loaded_at = Some(std::time::Instant::now());
        self.saves.rx = Some(self.spawn_task(move || crate::saves::list_saves(&dir)));
    }

    /// Reads the saves when the Saves tab is shown, and again every 30 s while it is
    /// open, so new saves and autosaves appear by themselves.
    pub fn ensure_saves_loaded(&mut self) {
        const REREAD: std::time::Duration = std::time::Duration::from_secs(30);
        if self.saves.loaded_at.is_none_or(|t| t.elapsed() > REREAD) {
            self.refresh_saves();
        }
    }

    pub fn saves_loading(&self) -> bool {
        self.saves.rx.is_some()
    }

    /// The save's mods compared with the current load order.
    pub fn save_compare(&self, save: &crate::saves::SaveInfo) -> crate::saves::SaveCompare {
        crate::saves::compare(&save.ids, &self.active_mods, |id| self.find_mod(id).is_some())
    }

    /// The save was made on another major game version than the installed one.
    pub fn save_other_version(&self, save: &crate::saves::SaveInfo) -> bool {
        let installed = self
            .steam_paths
            .game_version
            .as_deref()
            .and_then(crate::game_install::major_minor);
        let saved = save.game_version.as_deref().and_then(crate::game_install::major_minor);
        matches!((installed, saved), (Some(a), Some(b)) if a != b)
    }

    /// "Enable as in the save": the save's mods in its order, as unsaved changes
    /// (ModsConfig.xml changes only on "Save Order"; mods not on disk are kept).
    pub fn load_save_mods(&mut self, name: &str) {
        let Some(save) = self.saves.list.iter().find(|s| s.name == name) else {
            return;
        };
        let mut ids = Vec::with_capacity(save.ids.len());
        for id in &save.ids {
            add_active_id(&mut ids, id);
        }
        self.skip_save_check = true;
        self.active_mods = ids;
        self.modpacks.current = None;
        self.validate_deps();
        self.status_message = fmt(self.i18n.get(TranslationKey::SavesLoaded), &[name]);
    }

    /// Keeps the save's mod list as a modpack named after the save.
    pub fn save_to_modlist(&mut self, name: &str) {
        let t = self.i18n;
        let Some(dir) = self.modlists_folder() else {
            self.status_message = t.get(TranslationKey::SetsNoFolder).to_string();
            return;
        };
        let Some(save) = self.saves.list.iter().find(|s| s.name == name) else {
            return;
        };
        let entries: Vec<crate::modlists::ModListEntry> = save
            .ids
            .iter()
            .map(|id| {
                let m = self.find_mod(id);
                crate::modlists::ModListEntry {
                    id: id.to_lowercase(),
                    name: m
                        .map(|m| m.name.clone())
                        .or_else(|| save.name_of(id).map(str::to_string))
                        .unwrap_or_else(|| id.clone()),
                    steam_id: save
                        .steam_id(id)
                        .map(str::to_string)
                        .or_else(|| m.and_then(|m| m.steam_item_id.clone())),
                }
            })
            .collect();
        let base =
            crate::modlists::check_name(name).unwrap_or_else(|_| t.get(TranslationKey::SetsImportedName).to_string());
        let set_name = crate::modlists::unique_name(&dir, &base);
        let version = save
            .game_version
            .clone()
            .unwrap_or_else(|| self.game_version_for_sets());
        match crate::modlists::write_rml(&crate::modlists::path_for(&dir, &set_name), &version, &entries) {
            Ok(()) => {
                self.refresh_modlists();
                self.modpacks.selected = Some(set_name.clone());
                self.status_message = fmt(t.get(TranslationKey::SavesToSetDone), &[&set_name]);
            }
            Err(e) => self.status_message = fmt(t.get(TranslationKey::SetsWriteError), &[&e.to_string()]),
        }
    }

    /// F7: renames a save file on disk.
    pub fn rename_save(&mut self, from: &str, raw_name: &str) -> Result<(), String> {
        let name = crate::modlists::check_name(raw_name).map_err(|p| self.name_problem_text(p))?;
        let dir = self
            .saves_folder()
            .ok_or_else(|| self.i18n.get(TranslationKey::SavesNoFolder).to_string())?;
        let source = self
            .saves
            .list
            .iter()
            .find(|s| s.name == from)
            .map(|s| s.path.clone())
            .unwrap_or_else(|| crate::saves::path_for(&dir, from));
        let target = crate::saves::path_for(&dir, &name);
        if target.exists() && !name.eq_ignore_ascii_case(from) {
            return Err(self.i18n.get(TranslationKey::SavesNameExists).to_string());
        }
        std::fs::rename(&source, &target)
            .map_err(|e| fmt(self.i18n.get(TranslationKey::SavesWriteError), &[&e.to_string()]))?;
        if let Some(save) = self.saves.list.iter_mut().find(|s| s.name == from) {
            save.name = name.clone();
            save.path = target;
        }
        if self.saves.selected.as_deref() == Some(from) {
            self.saves.selected = Some(name.clone());
        }
        self.status_message = fmt(self.i18n.get(TranslationKey::SavesRenamed), &[from, &name]);
        Ok(())
    }

    /// F7: moves a save file to the Recycle Bin.
    pub fn delete_save(&mut self, name: &str) {
        let Some(path) = self
            .saves
            .list
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.path.clone())
            .or_else(|| self.saves_folder().map(|d| crate::saves::path_for(&d, name)))
        else {
            return;
        };
        match crate::cleaner::move_mod_dir_to_trash(&path) {
            Ok(()) => {
                self.saves.list.retain(|s| s.name != name);
                if self.saves.selected.as_deref() == Some(name) {
                    self.saves.selected = self.saves.list.first().map(|s| s.name.clone());
                }
                self.status_message = fmt(self.i18n.get(TranslationKey::SavesDeleted), &[name]);
            }
            Err(e) => {
                self.status_message = fmt(self.i18n.get(TranslationKey::SavesWriteError), &[&e]);
            }
        }
    }

    /// Subscribes to the save's mods that are not installed and have a Workshop ID.
    pub fn subscribe_save_missing(&mut self, name: &str) {
        let Some(save) = self.saves.list.iter().find(|s| s.name == name) else {
            return;
        };
        let ids = self.save_missing_workshop_ids(save);
        self.subscribe_workshop_ids(ids);
    }

    pub fn save_missing_workshop_ids(&self, save: &crate::saves::SaveInfo) -> Vec<String> {
        let mut ids: Vec<String> = save
            .ids
            .iter()
            .filter(|id| self.find_mod(id).is_none())
            .filter_map(|id| self.workshop_id_for(id, save.steam_id(id), save.name_of(id)))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    // ----- Mods the newest save needs (F9 of the plan, item 24) -----

    /// The save written last, once the saves are read (read in the background at start).
    pub(super) fn newest_save(&self) -> Option<&crate::saves::SaveInfo> {
        self.saves.list.iter().max_by_key(|s| s.modified.unwrap_or(0))
    }

    /// Mods of `before` that are off now and that the newest save is made with
    /// (Core stays on anyway): asks whether to turn them back on.
    pub(super) fn warn_removed_from_save(&mut self, before: &[String]) {
        if self.pending_question.is_some() {
            return;
        }
        let Some(save) = self.newest_save() else { return };
        let names: Vec<String> = turned_off_from_save(before, &self.active_mods, &save.ids)
            .into_iter()
            .map(|id| {
                self.find_mod(&id)
                    .map(|m| m.name.clone())
                    .or_else(|| save.name_of(&id).map(str::to_string))
                    .unwrap_or(id)
            })
            .collect();
        if !names.is_empty() {
            self.pending_question = Some(PendingQuestion::RemovedFromSave {
                save: save.name.clone(),
                names,
            });
        }
    }

    /// F7: window for renaming or deleting a save file.
    pub(super) fn render_save_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.saves.dialog.clone() else {
            return;
        };
        let t = self.i18n;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut submit = false;
        let title = match &dialog {
            SaveDialog::Rename { .. } => t.get(TranslationKey::SavesDialogRenameTitle),
            SaveDialog::Delete { .. } => t.get(TranslationKey::SavesDialogDeleteTitle),
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(400.0);
                match &mut dialog {
                    SaveDialog::Rename { name, error, .. } => {
                        ui.label(t.get(TranslationKey::SavesNameLabel));
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
                    }
                    SaveDialog::Delete { name } => {
                        ui.add(egui::Label::new(fmt(t.get(TranslationKey::SavesDialogDeleteBody), &[name])).wrap());
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (text, style) = match &dialog {
                        SaveDialog::Rename { .. } => (
                            t.get(TranslationKey::SetsBtnRename),
                            crate::ui::components::ButtonStyle::Primary,
                        ),
                        SaveDialog::Delete { .. } => (
                            t.get(TranslationKey::SetsBtnDelete),
                            crate::ui::components::ButtonStyle::Danger,
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
                SaveDialog::Rename { from, name, .. } => self.rename_save(from, name),
                SaveDialog::Delete { name } => {
                    self.delete_save(name);
                    Ok(())
                }
            };
            self.saves.dialog = match (result, dialog) {
                (Ok(()), _) => None,
                (Err(e), SaveDialog::Rename { from, name, .. }) => Some(SaveDialog::Rename {
                    from,
                    name,
                    error: Some(e),
                }),
                (Err(_), other) => Some(other),
            };
        } else if close {
            self.saves.dialog = None;
        } else {
            self.saves.dialog = Some(dialog);
        }
    }
}
