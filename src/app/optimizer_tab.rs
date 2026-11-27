//! Optimizer tab: texture conversion, rollback, orientation fix and the VRAM check before a launch.

use super::*;

/// State of the Optimizer tab and the VRAM check.
pub struct OptimizerState {
    pub scope: OptimizeScope,
    pub max_size: crate::optimizer::MaxTextureSize,
    pub force: bool,
    pub delete_source: bool,
    pub threads: usize,
    pub max_system_threads: usize,
    pub is_running: bool,
    pub done: usize,
    pub total: usize,
    pub errors: usize,
    pub current_file: String,
    pub stop_flag: Arc<AtomicBool>,
    pub rx: Option<Receiver<OptimizationMsg>>,
    pub ignore_list: std::collections::HashSet<String>,
    /// Optimizer DDS on disk stored the wrong way up for the installed game (counted on every
    /// rescan): made for 1.6 and now played on 1.7, or the other way round.
    pub wrong_orientation: usize,
    /// Background "Flip textures" run.
    pub orient_rx: Option<Receiver<crate::optimizer::OrientationMsg>>,
    /// Background "Scan" result.
    pub scan_rx: Option<Receiver<TextureSummary>>,
    /// Background "Rollback" result.
    pub rollback_rx: Option<Receiver<RollbackResult>>,
    /// Background VRAM check report vs GPU capacity (V2).
    pub vram_report: Option<crate::optimizer::VramReport>,
    pub(super) vram_rx: Option<Receiver<Result<crate::optimizer::VramCheckResult, ()>>>,
    pub(super) vram_cache: HashMap<PathBuf, (u64, u64)>,
    /// Active mods changed while a VRAM scan was already running: scan again when it finishes.
    pub(super) vram_recheck: bool,
    /// "Play" is waiting for the running VRAM scan to finish before deciding whether to warn.
    pub(super) launch_after_vram_check: bool,
}

impl App {
    pub fn toggle_optimizer_ignore(&mut self, package_id: &str) {
        let lower = package_id.to_lowercase();
        if self.optimizer.ignore_list.contains(&lower) {
            self.optimizer.ignore_list.remove(&lower);
        } else {
            self.optimizer.ignore_list.insert(lower);
        }

        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let ignore_path = std::path::PathBuf::from(local_appdata)
                .join("RimWorldModManager")
                .join("optimizer_ignore.txt");

            let content = self
                .optimizer
                .ignore_list
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let _ = crate::fs_util::write_atomic(&ignore_path, content);
        }
    }

    /// Mod folders for the chosen scope (All / Active / Selected), without Core/DLC,
    /// the blacklist and the user's ignore list. Used by both "Start" and "Scan" (R7).
    pub(super) fn optimizer_target_dirs(&self) -> Vec<PathBuf> {
        let active: std::collections::HashSet<String> = self.active_mods.iter().map(|s| s.to_lowercase()).collect();
        let selected = self.selected_mod_id.as_deref().map(str::to_lowercase);
        let mods: Vec<ModInfo> = self
            .optimizable_mods()
            .into_iter()
            .filter(|m| {
                let id = m.package_id.to_lowercase();
                match self.optimizer.scope {
                    OptimizeScope::AllInstalled => true,
                    OptimizeScope::ActiveOnly => active.contains(&id),
                    OptimizeScope::SelectedOnly => selected.as_deref() == Some(id.as_str()),
                }
            })
            .collect();
        dirs_from_mods(&mods)
    }

    pub fn start_texture_optimization(&mut self) {
        let dirs = self.optimizer_target_dirs();

        if dirs.is_empty() {
            self.status_message = tr!(
                "Нет выбранных модов для оптимизации текстур",
                "No mods selected for texture optimization",
                "没有选择要优化纹理的模组"
            )
            .to_string();
            return;
        }

        let (tx, rx) = channel();
        self.optimizer.rx = Some(rx);
        self.optimizer.is_running = true;
        self.optimizer.stop_flag.store(false, Ordering::Relaxed);
        self.optimizer.done = 0;
        self.optimizer.total = 0;
        self.optimizer.errors = 0;

        run_optimization(
            dirs,
            self.optimizer.force,
            self.optimizer.delete_source,
            // The side the installed game needs: flipped on 1.7, as the PNG on 1.6.
            ConvertOptions {
                flip_vertical: crate::optimizer::engine_wants_flipped(self.steam_paths.engine),
                max_size: self.optimizer.max_size.pixels(),
            },
            self.optimizer.threads,
            tx,
            self.optimizer.stop_flag.clone(),
        );

        self.status_message = tr!(
            "Запущена конвертация текстур (BC7 / BC1 с мипмапами)...",
            "Converting textures (BC7 / BC1 with mipmaps)...",
            "正在转换纹理（BC7 / BC1，含 mipmap）..."
        )
        .to_string();
    }

    /// RimWorld 1.7: moves TextureCache and ShaderCache to the Recycle Bin (game rebuilds them).
    /// In the background: the cache can be gigabytes, moving it to the Recycle Bin takes a while (R4).
    pub fn clear_game_caches(&mut self) {
        if self.cache_rx.is_some() {
            return;
        }
        self.status_message = tr!(
            "Очистка кэша игры...",
            "Clearing the game cache...",
            "正在清除游戏缓存..."
        )
        .to_string();
        self.cache_rx = Some(self.spawn_task(|| {
            let size_mb = crate::game_install::game_cache_size() as f64 / (1024.0 * 1024.0);
            match crate::game_install::clear_game_caches() {
                Ok(cleared) if cleared.is_empty() => tr!(
                    "Кэш игры уже пуст",
                    "The game cache is already empty",
                    "游戏缓存已经是空的"
                )
                .to_string(),
                Ok(cleared) => tr!(
                    "Кэш игры перемещён в Корзину ({}, {:.0} МБ). Игра пересоберёт его при следующем запуске.",
                    "Game cache moved to the Recycle Bin ({}, {:.0} MB). The game rebuilds it on the next start.",
                    "游戏缓存已移至回收站（{}，{:.0} MB）。游戏下次启动时会重新生成。",
                    cleared.join(", "),
                    size_mb
                ),
                Err(e) => tr!(
                    "Не удалось очистить кэш: {}",
                    "Could not clear the cache: {}",
                    "无法清除缓存：{}",
                    e
                ),
            }
        }));
    }

    /// "Flip textures": compares every optimizer DDS that may be the wrong way up for the
    /// installed game with its source and re-encodes the ones that are. Background, cancellable.
    pub fn fix_texture_orientation(&mut self) {
        if self.optimizer_busy() {
            return;
        }
        let want_flipped = crate::optimizer::engine_wants_flipped(self.steam_paths.engine);
        let (tx, rx) = channel();
        self.optimizer.orient_rx = Some(rx);
        self.optimizer.is_running = true;
        self.optimizer.stop_flag.store(false, Ordering::Relaxed);
        self.optimizer.done = 0;
        self.optimizer.total = 0;
        self.optimizer.errors = 0;
        self.optimizer.current_file.clear();
        crate::optimizer::fix_orientation_async(
            want_flipped,
            self.optimizer.threads,
            tx,
            self.optimizer.stop_flag.clone(),
        );
        self.status_message = self.i18n.get(TranslationKey::OptFlipRunning).to_string();
    }

    pub(super) fn poll_orientation_fix(&mut self) {
        let Some(rx) = &self.optimizer.orient_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                crate::optimizer::OrientationMsg::Progress {
                    done,
                    total,
                    current_file,
                } => {
                    self.optimizer.done = done;
                    self.optimizer.total = total;
                    self.optimizer.current_file = current_file;
                }
                crate::optimizer::OrientationMsg::Done(summary) => finished = Some(summary),
            }
        }
        if let Some(s) = finished {
            self.optimizer.orient_rx = None;
            self.optimizer.is_running = false;
            self.optimizer.errors = s.errors;
            let key = if s.cancelled {
                TranslationKey::OptFlipStopped
            } else {
                TranslationKey::OptFlipDone
            };
            self.status_message = fmt(
                self.i18n.get(key),
                &[&s.fixed.to_string(), &s.already_ok.to_string(), &s.errors.to_string()],
            );
            self.rescan_mods_keep_status();
        }
    }

    pub fn cancel_texture_optimization(&mut self) {
        self.optimizer.stop_flag.store(true, Ordering::Relaxed);
        self.status_message = tr!("Отмена оптимизации...", "Cancelling optimization...", "正在取消优化...").to_string();
    }

    /// Mods the optimizer may touch for "All mods": no Core/DLC, no blacklist, no user ignore list.
    pub(super) fn optimizable_mods(&self) -> Vec<ModInfo> {
        self.all_mods
            .iter()
            .filter(|m| {
                !self.is_official_mod(m)
                    && !self.optimizer.ignore_list.contains(&m.package_id.to_lowercase())
                    && !is_optimizer_blacklisted(&m.package_id)
            })
            .cloned()
            .collect()
    }

    /// "Scan": counts in a background thread (no UI freeze on big mod folders).
    pub fn scan_texture_counts(&mut self) {
        let dirs = self.optimizer_target_dirs();
        let (tx, rx) = channel();
        texture_summary_async(
            dirs,
            self.optimizer.force,
            crate::optimizer::engine_wants_flipped(self.steam_paths.engine),
            self.optimizer.max_size.pixels(),
            tx,
        );
        self.optimizer.scan_rx = Some(rx);
        self.status_message = tr!("Подсчёт текстур...", "Counting textures...", "正在统计纹理...").to_string();
    }

    /// Rolls back every mod except Core/DLC — including ignored/blacklisted ones,
    /// because older versions of the optimizer did not respect those lists. Runs in background.
    pub fn rollback_textures(&mut self) {
        let mods: Vec<ModInfo> = self
            .all_mods
            .iter()
            .filter(|m| !self.is_official_mod(m))
            .cloned()
            .collect();
        let (tx, rx) = channel();
        rollback_dds_async(dirs_from_mods(&mods), false, tx);
        self.optimizer.rollback_rx = Some(rx);
        self.status_message = tr!("Откат текстур...", "Rolling back textures...", "正在回滚纹理...").to_string();
    }

    /// O1: rolls back only the mod selected on the Mods tab; other mods' DDS stay.
    pub fn rollback_selected_mod_textures(&mut self) {
        let t = self.i18n;
        let Some(m) = self
            .selected_mod_id
            .as_deref()
            .and_then(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .cloned()
        else {
            self.status_message = t.get(TranslationKey::OptRollbackPickMod).to_string();
            return;
        };
        let (tx, rx) = channel();
        rollback_dds_async(dirs_from_mods(std::slice::from_ref(&m)), true, tx);
        self.optimizer.rollback_rx = Some(rx);
        self.status_message = fmt(t.get(TranslationKey::OptRollbackOneRunning), &[&m.name]);
    }

    /// Whether a background optimizer task (scan, rollback, conversion) is running.
    pub fn optimizer_busy(&self) -> bool {
        self.optimizer.is_running
            || self.optimizer.scan_rx.is_some()
            || self.optimizer.rollback_rx.is_some()
            || self.legacy_rx.is_some()
            || matches!(self.pending_question, Some(PendingQuestion::LegacyDds(_)))
    }

    pub(super) fn poll_optimizer_tasks(&mut self) {
        self.poll_orientation_fix();
        if let Some(rx) = &self.optimizer.scan_rx {
            if let Ok(s) = rx.try_recv() {
                self.status_message = tr!(
                    "Будет сконвертировано: {} | уже готово: {} | чужих DDS (не трогаются): {}",
                    "To convert: {} | already done: {} | author DDS (left alone): {}",
                    "待转换：{} | 已完成：{} | 作者自带 DDS（不处理）：{}",
                    s.to_convert,
                    s.up_to_date,
                    s.foreign_dds
                );
                if s.same_name > 0 {
                    self.status_message.push_str(&fmt(
                        self.i18n.get(TranslationKey::OptSameNameSkipped),
                        &[&s.same_name.to_string()],
                    ));
                }
                // O2: video memory estimate. Not on 1.7: the game compresses textures to BC7
                // itself there, so DDS only speed up loading.
                if s.pixels > 0 {
                    let mb = |b: u64| (b as f64 / 1_048_576.0).round().to_string();
                    if !crate::optimizer::engine_wants_flipped(self.steam_paths.engine) {
                        let (plain, dds) = s.vram_estimate();
                        self.status_message.push_str(&fmt(
                            self.i18n.get(TranslationKey::OptVramEstimate),
                            &[&mb(plain), &mb(dds)],
                        ));
                    }
                    let saved = s.vram_saved_by_limit();
                    if saved > 0 {
                        self.status_message
                            .push_str(&fmt(self.i18n.get(TranslationKey::OptVramSavedByLimit), &[&mb(saved)]));
                    }
                }
                self.optimizer.scan_rx = None;
            }
        }
        if let Some(rx) = &self.optimizer.rollback_rx {
            if let Ok(res) = rx.try_recv() {
                self.status_message = tr!(
                    "Откат: удалено .dds: {} | восстановлено исходников из бэкапа: {} | ошибок: {}",
                    "Rollback: .dds removed: {} | sources restored from backup: {} | errors: {}",
                    "回滚：已删除 .dds：{} | 已从备份恢复源文件：{} | 错误：{}",
                    res.removed,
                    res.restored,
                    res.errors
                );
                self.optimizer.rollback_rx = None;
                // Count again what is left on disk (normally nothing).
                self.rescan_mods_keep_status();
                self.trigger_vram_check();
                if !res.unlogged.is_empty() {
                    self.pending_question = Some(PendingQuestion::LegacyDds(res.unlogged));
                }
            }
        }
        self.poll_vram_check();
    }

    /// Deletes the DDS files the user agreed to remove after a rollback (background).
    pub(super) fn delete_unlogged_dds(&mut self, paths: Vec<PathBuf>) {
        self.status_message = tr!("Удаление DDS...", "Deleting DDS...", "正在删除 DDS...").to_string();
        self.legacy_rx = Some(self.spawn_task(move || {
            let (removed, errors) = crate::optimizer::delete_files(&paths);
            tr!(
                "Удалено DDS не из журнала: {} | ошибок: {}",
                "Unlogged DDS deleted: {} | errors: {}",
                "已删除未记录的 DDS：{} | 错误：{}",
                removed,
                errors
            )
        }));
    }

    pub fn check_vram_and_launch(&mut self) {
        if self.optimizer.vram_report.is_none() && self.optimizer.vram_rx.is_none() {
            self.trigger_vram_check();
        }
        while let Some(rx) = &self.optimizer.vram_rx {
            match rx.recv_timeout(std::time::Duration::from_millis(150)) {
                Ok(Ok((report, updated_cache))) => {
                    for (dir, mtime, bytes) in updated_cache {
                        self.optimizer.vram_cache.insert(dir, (mtime, bytes));
                    }
                    self.optimizer.vram_rx = None;
                    if self.optimizer.vram_recheck {
                        self.optimizer.vram_recheck = false;
                        self.trigger_vram_check();
                        continue;
                    }
                    self.optimizer.vram_report = Some(report);
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    self.optimizer.launch_after_vram_check = true;
                    self.status_message = self.i18n.get(TranslationKey::VramCheckingBeforeLaunch).to_string();
                    return;
                }
                Ok(Err(())) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.optimizer.launch_after_vram_check = true;
                    self.fail_vram_check();
                    return;
                }
            }
        }
        self.finish_vram_check_and_launch();
    }

    /// The VRAM check broke down: it is only a warning, so a waiting "Play" goes ahead.
    pub(super) fn fail_vram_check(&mut self) {
        self.optimizer.vram_rx = None;
        self.optimizer.vram_recheck = false;
        self.optimizer.vram_report = None;
        if std::mem::take(&mut self.optimizer.launch_after_vram_check) {
            self.launch_game_now();
        }
    }

    pub(super) fn finish_vram_check_and_launch(&mut self) {
        self.optimizer.launch_after_vram_check = false;
        // What the game really took last time with these mods beats the estimate (1.6).
        let report = self.optimizer.vram_report.as_ref().map(|estimate| {
            self.analyzer
                .companion_report
                .as_ref()
                .and_then(|r| r.measured_vram(estimate, &self.active_mods))
                .unwrap_or_else(|| estimate.clone())
        });
        if let Some(report) = report.filter(crate::optimizer::VramReport::is_critical) {
            self.pending_question = Some(PendingQuestion::VramWarning {
                estimated_bytes: report.estimated_used_bytes,
                total_vram_bytes: report.total_vram_bytes,
                heavy_mods: report.heavy_mods,
                measured: report.measured,
            });
            return;
        }
        self.launch_game_now();
    }

    /// Triggers a background evaluation of active mod texture VRAM vs GPU capacity (V2).
    pub fn trigger_vram_check(&mut self) {
        self.optimizer.vram_report = None;
        if self.optimizer.vram_rx.is_some() {
            self.optimizer.vram_recheck = true;
            return;
        }
        let active_mods: Vec<(String, PathBuf)> = self
            .active_mods
            .iter()
            .filter_map(|id| self.find_mod(id))
            .filter(|m| !self.is_official_mod(m))
            .map(|m| (m.name.clone(), m.dir_path.clone()))
            .collect();

        let engine = self.steam_paths.engine;
        let version = self.steam_paths.game_version.clone();
        let cache = self.optimizer.vram_cache.clone();
        // The game's Prefs.xml lives next to the ModsConfig.xml it reads.
        let config_dir = self
            .mods_config_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        self.optimizer.vram_rx = Some(self.spawn_task(move || {
            let game_compresses = crate::optimizer::game_compresses_textures(config_dir.as_deref());
            let (tx, rx) = channel();
            crate::optimizer::check_vram_async(active_mods, engine, version, game_compresses, cache, tx);
            rx.recv().map_err(|_| ())
        }));
    }

    pub(super) fn poll_vram_check(&mut self) {
        let Some(rx) = &self.optimizer.vram_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((report, updated_cache))) => {
                for (dir, mtime, bytes) in updated_cache {
                    self.optimizer.vram_cache.insert(dir, (mtime, bytes));
                }
                self.optimizer.vram_rx = None;
                if self.optimizer.vram_recheck {
                    self.optimizer.vram_recheck = false;
                    self.trigger_vram_check();
                    return;
                }
                self.optimizer.vram_report = Some(report);
                if std::mem::take(&mut self.optimizer.launch_after_vram_check) {
                    self.finish_vram_check_and_launch();
                }
            }
            Ok(Err(())) => self.fail_vram_check(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.fail_vram_check(),
        }
    }
}
