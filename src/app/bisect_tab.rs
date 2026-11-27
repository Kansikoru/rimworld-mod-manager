//! Broken-mod search (F8): the wizard's state and its steps.

use super::*;

/// State of the broken-mod search. The session is saved to bisect.json after every change,
/// so it survives closing the program in the middle.
pub struct BisectUiState {
    /// RWMM_TAB=bisect: open the broken-mod search once the mods are scanned (screenshots).
    pub(super) open_after_scan: bool,
    pub open: bool,
    pub session: Option<crate::bisect::Bisect>,
    /// Setup screen: mods ticked as "never turn off" (lowercase IDs).
    pub keep: Option<std::collections::HashSet<String>>,
    pub filter: String,
    /// Why the last wizard action didn't run (game running, unsaved order...).
    pub note: Option<String>,
    /// Watches for RimWorld to exit after a bisect launch and analyzes Player.log.
    pub(super) watch_rx: Option<Receiver<Option<LogReport>>>,
}

impl App {
    /// RimWorld closed after a bisect launch: answer the step from its log when the log is clear.
    pub(super) fn poll_bisect_watch(&mut self) {
        if let Some(rx) = &self.bisect.watch_rx {
            if let Ok(opt_report) = rx.try_recv() {
                self.bisect.watch_rx = None;
                if let Some(report) = opt_report {
                    // Only a clear log answers by itself: a crash = "still there", not a single
                    // error = "gone". Errors without a crash are in almost every RimWorld log,
                    // working ones too, so then the player answers (the search would otherwise
                    // end on an innocent mod).
                    let answer = if report.has_hard_crash {
                        Some(true)
                    } else if report.total_errors == 0 && report.culprits.is_empty() {
                        Some(false)
                    } else {
                        None
                    };
                    let errors = report.total_errors;
                    self.analyzer.report = Some(report);
                    self.analyzer.companion_report = crate::companion_report::load();
                    self.analyzer.tick_profile = crate::tick_profile::load();
                    if self
                        .bisect
                        .session
                        .as_ref()
                        .is_some_and(|s| s.applied && s.outcome().is_none())
                    {
                        let note = match answer {
                            Some(problem) => {
                                self.bisect_answer(problem);
                                let key = if problem {
                                    TranslationKey::BisectAutoYes
                                } else {
                                    TranslationKey::BisectAutoNo
                                };
                                self.i18n.get(key).to_string()
                            }
                            None => fmt(self.i18n.get(TranslationKey::BisectAutoUnsure), &[&errors.to_string()]),
                        };
                        self.bisect.note = Some(note);
                    }
                }
            }
        }
    }

    /// Opens the search window: the running search, or the setup for a new one.
    pub fn open_bisect(&mut self) {
        self.bisect.open = true;
        self.bisect.note = None;
        if self.bisect.session.is_none() && self.bisect.keep.is_none() {
            self.bisect.keep = Some(self.bisect_default_keep());
        }
    }

    /// Never turned off by default: Core and DLC, Harmony / Prepatcher, frameworks.
    pub(super) fn bisect_default_keep(&self) -> std::collections::HashSet<String> {
        self.saved_active_mods
            .iter()
            .filter(|id| {
                let lower = id.to_lowercase();
                lower == "ludeon.rimworld"
                    || lower.starts_with("ludeon.rimworld.")
                    || lower == "brrainz.harmony"
                    || lower == "zetrith.prepatcher"
                    || lower == crate::companion::PACKAGE_ID
                    || self
                        .find_mod(id)
                        .is_some_and(|m| self.is_official_mod(m) || m.categories.contains(&ModCategory::Framework))
            })
            .map(|id| id.to_lowercase())
            .collect()
    }

    /// What each mod of `ids` needs among `ids` (alternatives and Prepatcher-for-Harmony count).
    pub fn bisect_requirements(&self, ids: &[String]) -> std::collections::BTreeMap<String, Vec<String>> {
        let present: std::collections::HashSet<String> = ids.iter().map(|i| i.to_lowercase()).collect();
        let mut out = std::collections::BTreeMap::new();
        for id in ids {
            let Some(m) = self.find_mod(id) else { continue };
            let mut needs: Vec<String> = Vec::new();
            for dep in &m.dependencies {
                let dep = dep.trim().to_lowercase();
                let declared = m.dependency_alternatives.get(&dep).cloned().unwrap_or_default();
                let choice = std::iter::once(dep.clone())
                    .chain(declared.into_iter().map(|a| a.to_lowercase()))
                    .chain(
                        crate::dependencies::dependency_substitutes(&dep)
                            .iter()
                            .map(|s| s.to_string()),
                    )
                    .find(|candidate| present.contains(candidate));
                if let Some(c) = choice {
                    needs.push(c);
                }
            }
            if !needs.is_empty() {
                out.insert(id.to_lowercase(), needs);
            }
        }
        out
    }

    pub(super) fn persist_bisect(&self) {
        let path = bisect_session_path();
        match &self.bisect.session {
            Some(search) => {
                if let Ok(text) = serde_json::to_string(search) {
                    let _ = crate::fs_util::write_atomic(&path, text);
                }
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Why a step can't run now: the game is open, or the list has unsaved changes.
    pub(super) fn bisect_blocker(&self, check_unsaved: bool) -> Option<String> {
        let t = self.i18n;
        if self.mods_config_path.is_none() {
            return Some(t.get(TranslationKey::BisectNoConfig).to_string());
        }
        if check_unsaved && self.has_unsaved_changes() {
            return Some(t.get(TranslationKey::BisectUnsaved).to_string());
        }
        if crate::game_install::is_game_running() {
            return Some(t.get(TranslationKey::BisectCloseGame).to_string());
        }
        None
    }

    /// Starts a search from the load order the game uses now, keeping the ticked mods on.
    /// The current order is saved as a modpack first, so it can always be brought back.
    pub fn start_bisect(&mut self) {
        if let Some(why) = self.bisect_blocker(true) {
            self.bisect.note = Some(why);
            return;
        }
        let keep: Vec<String> = self.bisect.keep.clone().unwrap_or_default().into_iter().collect();
        let original = self.saved_active_mods.clone();
        let requires = self.bisect_requirements(&original);
        let search = crate::bisect::Bisect::new(&original, &keep, &requires);

        // "Before the search" modpack (overwritten each time).
        if let Some(dir) = self.modlists_folder() {
            let name = self.i18n.get(TranslationKey::BisectBackupSetName);
            let entries = self.modlist_entries();
            let version = self.game_version_for_sets();
            if crate::modlists::write_rml(&crate::modlists::path_for(&dir, name), &version, &entries).is_ok() {
                self.refresh_modlists();
            }
        }
        self.bisect.session = Some(search);
        self.bisect.keep = None;
        self.bisect.note = None;
        self.persist_bisect();
    }

    /// Writes the current test's load order to ModsConfig.xml, and starts the game if asked.
    pub fn bisect_apply(&mut self, launch: bool) {
        if let Some(why) = self.bisect_blocker(false) {
            self.bisect.note = Some(why);
            return;
        }
        let (Some(search), Some(config)) = (self.bisect.session.as_ref(), self.mods_config_path.clone()) else {
            return;
        };
        let enabled = search.enabled();
        // One backup at the first launch; later steps don't push real backups out.
        let first = search.history.is_empty() && !search.applied;
        let written = if first {
            save_active_mods(&config, &enabled)
        } else {
            crate::mods_config::write_active_mods(&config, &enabled)
        };
        if let Err(e) = written {
            self.bisect.note = Some(e);
            return;
        }
        self.skip_save_check = true;
        self.active_mods = enabled.clone();
        self.remember_saved_order(&enabled);
        self.validate_deps();
        if let Some(search) = self.bisect.session.as_mut() {
            search.applied = true;
        }
        self.bisect.note = None;
        self.persist_bisect();
        if launch {
            self.launch_game_now();
            self.start_bisect_game_watch();
        }
    }

    /// F8: watches for the game process to exit after a bisect launch, then analyzes Player.log.
    pub(super) fn start_bisect_game_watch(&mut self) {
        self.bisect.watch_rx = self.spawn_game_log_watch();
    }

    pub fn bisect_watching_game(&self) -> bool {
        self.bisect.watch_rx.is_some()
    }

    pub fn bisect_answer(&mut self, problem: bool) {
        self.bisect.watch_rx = None;
        if let Some(search) = self.bisect.session.as_mut() {
            search.answer(problem);
            self.bisect.note = None;
            self.persist_bisect();
        }
    }

    pub fn bisect_step_back(&mut self) {
        self.bisect.watch_rx = None;
        if let Some(search) = self.bisect.session.as_mut() {
            search.step_back();
            self.bisect.note = None;
            self.persist_bisect();
        }
    }

    /// Ends the search: brings the original load order back, without the found mods and
    /// what needs them if `turn_off_found`.
    pub fn bisect_finish(&mut self, turn_off_found: bool) {
        if let Some(why) = self.bisect_blocker(false) {
            self.bisect.note = Some(why);
            return;
        }
        let Some(search) = self.bisect.session.as_ref() else {
            return;
        };
        let Some(config) = self.mods_config_path.clone() else {
            return;
        };
        let t = self.i18n;
        let mut order = search.original.clone();
        let mut message = t.get(TranslationKey::BisectRestored).to_string();
        if turn_off_found {
            if let Some(crate::bisect::Outcome::Found(found)) = search.outcome() {
                let mut off: Vec<String> = found.clone();
                off.extend(search.dependents_of(found));
                order.retain(|id| !off.contains(id));
                message = fmt(t.get(TranslationKey::BisectTurnedOff), &[&off.len().to_string()]);
            }
        }
        if let Err(e) = save_active_mods(&config, &order) {
            self.bisect.note = Some(e);
            return;
        }
        self.skip_save_check = true;
        self.active_mods = order.clone();
        self.remember_saved_order(&order);
        self.validate_deps();
        self.bisect.session = None;
        self.bisect.open = false;
        self.bisect.note = None;
        self.persist_bisect();
        self.status_message = message;
    }

    /// The setup screen is closed without starting.
    pub fn close_bisect_window(&mut self) {
        self.bisect.open = false;
        self.bisect.keep = None;
        self.bisect.note = None;
    }

    /// Launches a bisect search over the mods updated or added since the last good run (F9);
    /// the rest stay on as the base.
    pub fn start_bisect_on_updated(&mut self) {
        let Some(diff) = &self.analyzer.last_good_diff else {
            return;
        };
        if diff.updated.is_empty() && diff.added.is_empty() {
            return;
        };
        let suspects: std::collections::HashSet<String> = diff
            .updated
            .iter()
            .chain(&diff.added)
            .map(|id| id.to_lowercase())
            .collect();

        let mut keep = std::collections::HashSet::new();
        for id in &self.active_mods {
            let key = id.to_lowercase();
            if !suspects.contains(&key) {
                keep.insert(key);
            }
        }
        self.bisect.keep = Some(keep);
        self.start_bisect();
        self.bisect.open = true;
    }
}

fn bisect_session_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("bisect.json")
}

/// An unfinished broken-mod search from an earlier run, if any.
pub(super) fn load_bisect_session() -> Option<crate::bisect::Bisect> {
    let text = std::fs::read_to_string(bisect_session_path()).ok()?;
    serde_json::from_str(&text).ok()
}
