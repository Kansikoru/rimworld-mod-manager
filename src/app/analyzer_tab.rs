//! Log Analyzer: the log report, the companion mod and its reports, the last working run.

use super::*;

/// State of the Log Analyzer tab.
pub struct AnalyzerState {
    pub report: Option<LogReport>,
    /// What the companion mod wrote after the last loading with it (textures, conflicts).
    pub companion_report: Option<crate::companion_report::CompanionReport>,
    /// The mods' load per game tick, as the companion last measured it (`tick_profile.rs`).
    pub tick_profile: Option<crate::tick_profile::TickProfile>,
    /// The next launch from the manager runs with the game's verbose logging, so the log
    /// holds how long each mod took to load (`load_profile`).
    pub measure_next_launch: bool,
    /// Lowercase package ID -> (textures with a side not a power of two, all PNG/JPG) of the
    /// mods in the log's load profile (1.7 loads those twice).
    pub odd_textures: HashMap<String, (u32, u32)>,
    pub(super) odd_textures_rx: Option<Receiver<OddTexturesResult>>,
    /// The log (path, size) `odd_textures` were counted for.
    pub(super) odd_textures_for: Option<(PathBuf, u64)>,
    pub(super) texture_sides_cache: Option<HashMap<PathBuf, (u64, u32, u32)>>,
    pub custom_log_path: Option<PathBuf>,
    /// F9: Snapshot of the last known working run and diff with current configuration.
    pub last_good: Option<crate::last_good::LastGoodSnapshot>,
    pub last_good_diff: Option<crate::last_good::LastGoodDiff>,
    pub(super) last_good_rx: Option<Receiver<crate::last_good::LastGoodDiff>>,
    /// A new "last working run" snapshot being written in the background.
    pub(super) last_good_save_rx: Option<Receiver<crate::last_good::LastGoodSnapshot>>,
    /// The active list changed while the diff was being computed: compute it again.
    pub(super) last_good_rerun: bool,
    /// F13: Log viewer mode (Report / FullLog) and filters.
    pub filter_text: String,
    pub level_filter: crate::analyzer::LogLevelFilter,
    /// RWMM_TAB=analyzer-companion: scroll the Log Analyzer to the companion's report once the
    /// log is analyzed (screenshots).
    pub scroll_to_companion_report: bool,
    pub rx: Option<Receiver<LogReport>>,
    pub(super) silent: bool,
    pub(super) initial_log_checked: bool,
}

impl App {
    // ----- Companion mod (F11) -----

    /// The companion mod in the game's Mods folder (`None`: the Mods folder is unknown).
    pub fn companion_state(&self) -> Option<crate::companion::State> {
        self.game_mods_dir().map(|d| crate::companion::state(&d))
    }

    /// Asks the companion to measure the mods' load on the next launch of the game. The
    /// companion in the game must be the one this version carries: an older one doesn't
    /// know the request.
    pub fn request_tick_profile(&mut self) {
        let t = self.i18n;
        if self.companion_state() != Some(crate::companion::State::Current) {
            self.install_companion();
            if self.companion_state() != Some(crate::companion::State::Current) {
                // `install_companion` said why (no Mods folder, or the game is running).
                return;
            }
        }
        self.status_message =
            match crate::tick_profile::request_at(&crate::tick_profile::request_path(), crate::tick_profile::TICKS) {
                Ok(()) => t.get(TranslationKey::TickProfileRequested).to_string(),
                Err(e) => fmt(t.get(TranslationKey::TickProfileError), &[&e.to_string()]),
            };
    }

    /// Puts the companion mod into the game's Mods folder and turns it on (at the end of the
    /// list). Saved right away when the list had no unsaved changes of its own.
    pub fn install_companion(&mut self) {
        let t = self.i18n;
        let Some(mods_dir) = self.game_mods_dir() else {
            self.status_message = t.get(TranslationKey::SteamCmdNoMods).to_string();
            return;
        };
        if crate::game_install::is_game_running() {
            self.status_message = t.get(TranslationKey::CompanionCloseGame).to_string();
            return;
        }
        if let Err(e) = crate::companion::install(&mods_dir) {
            self.status_message = fmt(t.get(TranslationKey::CompanionError), &[&e.to_string()]);
            return;
        }
        let had_unsaved = self.has_unsaved_changes();
        add_active_id(&mut self.active_mods, crate::companion::PACKAGE_ID);
        if had_unsaved {
            self.status_message = t.get(TranslationKey::CompanionInstalledUnsaved).to_string();
        } else if self.save_config() {
            self.status_message = t.get(TranslationKey::CompanionInstalled).to_string();
        }
        self.rescan_mods_keep_status();
    }

    /// Turns the companion mod off and moves it to the Recycle Bin.
    pub fn remove_companion(&mut self) {
        let t = self.i18n;
        let Some(mods_dir) = self.game_mods_dir() else { return };
        if crate::game_install::is_game_running() {
            self.status_message = t.get(TranslationKey::CompanionCloseGame).to_string();
            return;
        }
        let had_unsaved = self.has_unsaved_changes();
        self.active_mods
            .retain(|id| !id.eq_ignore_ascii_case(crate::companion::PACKAGE_ID));
        if !had_unsaved {
            self.save_config();
        }
        self.status_message = match crate::companion::remove(&mods_dir) {
            Ok(()) => t.get(TranslationKey::CompanionRemoved).to_string(),
            Err(e) => fmt(t.get(TranslationKey::CompanionError), &[&e]),
        };
        self.rescan_mods_keep_status();
    }

    /// A newer companion mod comes with the manager: update the installed one (not while the
    /// game runs, it keeps the dll open).
    pub(super) fn update_companion_if_outdated(&mut self) {
        let Some(mods_dir) = self.game_mods_dir() else { return };
        if crate::companion::state(&mods_dir) == crate::companion::State::Outdated
            && !crate::game_install::is_game_running()
        {
            let _ = crate::companion::install(&mods_dir);
        }
    }

    /// For a log with a load profile from 1.7: counts each profiled mod's textures with a side
    /// that isn't a power of two, in the background (cached per mod, so only the first time
    /// reads every texture header).
    pub(super) fn poll_odd_textures(&mut self) {
        if let Some(rx) = &self.analyzer.odd_textures_rx {
            if let Ok((counts, fresh)) = rx.try_recv() {
                self.analyzer.odd_textures_rx = None;
                let cache = self.analyzer.texture_sides_cache.get_or_insert_with(HashMap::new);
                if !fresh.is_empty() {
                    for (dir, fp, odd, all) in fresh {
                        cache.insert(dir, (fp, odd, all));
                    }
                    let _ = crate::optimizer::write_texture_sides_cache(
                        &crate::optimizer::texture_sides_cache_path(),
                        cache,
                    );
                }
                self.analyzer.odd_textures = counts.into_iter().map(|(id, c)| (id.to_lowercase(), c)).collect();
            }
            return;
        }
        let Some(report) = self.analyzer.report.as_ref().filter(|r| r.is_waylith) else {
            return;
        };
        let Some(profile) = &report.load_profile else {
            return;
        };
        let key = (report.log_path.clone(), report.file_size_bytes);
        if self.analyzer.odd_textures_for.as_ref() == Some(&key) {
            return;
        }
        let mods: Vec<(String, PathBuf)> = profile
            .mods
            .iter()
            .filter_map(|p| self.find_mod(&p.id))
            .filter(|m| !self.is_official_mod(m))
            .map(|m| (m.package_id.clone(), m.dir_path.clone()))
            .collect();
        self.analyzer.odd_textures_for = Some(key);
        let cache = self
            .analyzer
            .texture_sides_cache
            .get_or_insert_with(|| {
                crate::optimizer::read_texture_sides_cache(&crate::optimizer::texture_sides_cache_path())
            })
            .clone();
        let version = self.steam_paths.game_version.clone();
        self.analyzer.odd_textures_rx =
            Some(self.spawn_task(move || crate::optimizer::non_power_of_two_by_mod(&mods, version.as_deref(), &cache)));
    }

    pub fn odd_textures_counting(&self) -> bool {
        self.analyzer.odd_textures_rx.is_some()
    }

    /// "Measure and play": the next launch runs with verbose logging (see `measure_next_launch`).
    pub fn launch_with_load_measure(&mut self) {
        self.analyzer.measure_next_launch = true;
        self.launch_game();
    }

    /// Refreshes, in the background, the diff between the active mods and the last known
    /// working snapshot (F9). It reads Steam's install records and walks the files of local
    /// mods, too slow for every click in the mod list; a change during a run reruns it.
    pub fn refresh_last_good_diff(&mut self) {
        let Some(snapshot) = self.analyzer.last_good.clone() else {
            self.analyzer.last_good_diff = None;
            return;
        };
        if self.analyzer.last_good_rx.is_some() {
            self.analyzer.last_good_rerun = true;
            return;
        }
        let active = self.active_mods.clone();
        let mods: Vec<ModInfo> = active.iter().filter_map(|id| self.find_mod(id)).cloned().collect();
        let workshop = self.steam_paths.workshop_path.clone();
        let steamcmd = self.steamcmd_dir();
        self.analyzer.last_good_rx = Some(self.spawn_task(move || {
            let timestamps =
                crate::last_good::collect_mod_timestamps(&active, &mods, workshop.as_deref(), Some(&steamcmd));
            snapshot.diff(&active, &timestamps)
        }));
    }

    pub(super) fn poll_last_good_diff(&mut self) {
        let Some(rx) = &self.analyzer.last_good_rx else { return };
        if let Ok(diff) = rx.try_recv() {
            self.analyzer.last_good_rx = None;
            self.analyzer.last_good_diff = Some(diff);
            if std::mem::take(&mut self.analyzer.last_good_rerun) {
                self.refresh_last_good_diff();
            }
        }
    }

    /// Records the current active mod configuration and file timestamps as the known good run (F9).
    pub fn remember_last_good(&mut self) {
        let mods = self.active_mods.clone();
        self.remember_last_good_for(&mods);
        self.status_message = self.i18n.get(TranslationKey::LastGoodSavedMsg).to_string();
    }

    /// Saves a `LastGoodSnapshot` for the specified verified mod order without overwriting
    /// `status_message`. In the background: it reads Steam's records and walks local mods.
    pub fn remember_last_good_for(&mut self, active_mods: &[String]) {
        let steamcmd = self.steamcmd_dir();
        let active = active_mods.to_vec();
        let mods: Vec<ModInfo> = active.iter().filter_map(|id| self.find_mod(id)).cloned().collect();
        let steam_paths = self.steam_paths.clone();
        self.analyzer.last_good_save_rx = Some(self.spawn_task(move || {
            let snap = crate::last_good::LastGoodSnapshot::create(&active, &mods, &steam_paths, Some(&steamcmd));
            let _ = snap.save();
            snap
        }));
    }

    pub(super) fn poll_last_good_save(&mut self) {
        let Some(rx) = &self.analyzer.last_good_save_rx else {
            return;
        };
        if let Ok(snap) = rx.try_recv() {
            self.analyzer.last_good_save_rx = None;
            self.analyzer.last_good = Some(snap);
            self.refresh_last_good_diff();
        }
    }

    /// Records load time (F11) and last-good snapshot (F9) only when `report` comes from a
    /// `Player.log` written after RimWorld was actually launched in this session (the launch
    /// watcher, once the game has closed).
    /// A run counts as working when the game neither crashed nor hit a graphics card or memory
    /// failure nor a broken Prepatcher / Harmony patch: big mod lists nearly always log some errors.
    pub fn handle_verified_launch_report(&mut self, report: &LogReport) {
        if !is_verified_launch_log(
            self.game_launch_time,
            self.bisect.session.is_some(),
            self.analyzer.custom_log_path.is_some(),
            &report.log_path,
        ) {
            return;
        }
        self.game_launch_time = None;
        let mods = if !self.game_launch_active_mods.is_empty() {
            std::mem::take(&mut self.game_launch_active_mods)
        } else if !self.saved_active_mods.is_empty() {
            self.saved_active_mods.clone()
        } else {
            self.active_mods.clone()
        };
        if let Some(secs) = report.load_time_secs {
            let mut history = std::mem::take(&mut self.load_time_history);
            history.record_launch(secs, &mods, |id| self.mod_display_name(id));
            let _ = history.save();
            self.load_time_history = history;
        }
        if is_working_run(report) {
            self.remember_last_good_for(&mods);
        }
    }

    pub fn run_log_analysis(&mut self) {
        let log_target = if let Some(ref custom_p) = self.analyzer.custom_log_path {
            Some(custom_p.clone())
        } else {
            detect_rimworld_log_paths().into_iter().next().map(|(_, p)| p)
        };

        if self.analyzer.rx.is_some() {
            return;
        }
        if let Some(path) = log_target {
            // Matching stack frames to mods walks every mod folder: done in the background (S5).
            let mods = self.all_mods.clone();
            self.analyzer.silent = false;
            self.status_message = tr!(
                "Анализ лога {}...",
                "Analyzing log {}...",
                "正在分析日志 {}...",
                path.display()
            );
            self.analyzer.rx = Some(self.spawn_task(move || analyze_rimworld_log(&path, &mods)));
        } else {
            self.status_message = self.i18n.get(TranslationKey::CrashFileNotFound).to_string();
        }
    }

    /// Exports the current analysis report as text or standalone HTML (F13).
    pub fn save_log_report(&mut self) {
        let Some(report) = &self.analyzer.report else { return };
        let t = self.i18n;
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(t.get(TranslationKey::ReportFilterText), &["txt"])
            .add_filter(t.get(TranslationKey::ReportFilterHtml), &["html"])
            .set_file_name("rimworld_mod_report.txt")
            .save_file()
        {
            let is_html = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("html"));
            let content = if is_html {
                crate::analyzer::generate_html_report(report)
            } else {
                crate::analyzer::generate_text_report(report)
            };
            self.status_message = match std::fs::write(&path, content) {
                Err(e) => fmt(t.get(TranslationKey::ReportSaveFailed), &[&e.to_string()]),
                Ok(()) => fmt(t.get(TranslationKey::ReportSaved), &[&path.display().to_string()]),
            };
        }
    }

    /// F10: mods that produced errors in the last analyzed Player.log (lowercase package_id -> culprit).
    pub fn log_culprit_map(&self) -> std::collections::HashMap<String, crate::analyzer::CrashCulprit> {
        let mut out = std::collections::HashMap::new();
        let Some(report) = &self.analyzer.report else {
            return out;
        };
        for c in &report.culprits {
            let Some(pkg) = &c.package_id else { continue };
            let key = pkg.to_lowercase();
            let replace = out
                .get(&key)
                .is_none_or(|prev: &crate::analyzer::CrashCulprit| c.severity < prev.severity);
            if replace {
                out.insert(key, c.clone());
            }
        }
        out
    }

    pub(super) fn crash_summary_text(&self, report: &LogReport) -> String {
        if let Some(issue) = report.system_issue {
            return match issue {
                crate::analyzer::SystemCrashIssue::GpuReset => {
                    self.i18n.get(TranslationKey::CrashGpuResetTitle).to_string()
                }
                crate::analyzer::SystemCrashIssue::OutOfMemory => {
                    self.i18n.get(TranslationKey::CrashOomTitle).to_string()
                }
            };
        }
        if report.has_hard_crash {
            fmt(
                self.i18n.get(TranslationKey::CrashSummaryHardCrash),
                &[&report.culprits.len().to_string()],
            )
        } else if !report.culprits.is_empty() {
            fmt(
                self.i18n.get(TranslationKey::CrashSummaryCulprits),
                &[&report.culprits.len().to_string()],
            )
        } else if report.total_errors > 0 {
            fmt(
                self.i18n.get(TranslationKey::CrashSummaryErrorsOnly),
                &[&report.total_errors.to_string()],
            )
        } else {
            self.i18n.get(TranslationKey::CrashSummaryClean).to_string()
        }
    }
}
