//! Translations tab: the manual Workshop search for translations and the modpack-wide search.

use super::*;

/// State of the Translations tab.
pub struct TranslationsState {
    pub target_lang: TranslationTargetLang,
    pub search_query: String,
    pub search_results: Vec<WorkshopItemResult>,
    pub search_rx: Option<Receiver<WorkshopSearchOutcome>>,
    /// Modpack-wide translation search: messages, stop flag, progress (done, total).
    pub batch_rx: Option<Receiver<BatchSearchMsg>>,
    pub batch_stop: Arc<AtomicBool>,
    pub batch_progress: (usize, usize),
    /// Mods with translations found, by name; the language they were searched for.
    pub batch_results: Vec<ModTranslations>,
    pub batch_lang: TranslationTargetLang,
    /// Active mods not searched: the language is already in the mod or a translation is installed.
    pub batch_skipped: usize,
    /// Workshop IDs ticked for "Subscribe", and the ones already sent to Steam.
    pub batch_checked: std::collections::HashSet<String>,
    pub batch_subscribed: std::collections::HashSet<String>,
    /// The right column shows the modpack results (true) or the manual search (false).
    pub show_batch_results: bool,
    /// RWMM_TAB=translations-batch: start the modpack search once the mods are scanned (screenshots).
    pub(super) batch_search_after_scan: bool,
}

impl App {
    /// The manual Workshop search for translations finished.
    pub(super) fn poll_translation_search(&mut self) {
        if let Some(rx) = &self.translations.search_rx {
            if let Ok(res) = rx.try_recv() {
                self.translations.search_rx = None;
                match res {
                    Ok((items, status)) => {
                        self.translations.search_results = items;
                        self.status_message = status;
                    }
                    Err(e) => {
                        self.status_message = tr!(
                            "Ошибка поиска в Workshop: {}",
                            "Workshop search failed: {}",
                            "创意工坊搜索失败：{}",
                            e
                        )
                    }
                }
            }
        }
    }

    /// Translation mods installed for the language selected on the Translations tab.
    pub fn translations_for_target_lang(&self) -> Vec<&ModInfo> {
        self.all_mods
            .iter()
            .filter(|m| m.is_translation_mod && !self.is_official_mod(m))
            .filter(|m| self.translations.target_lang.matches_mod_info(m))
            .collect()
    }

    /// "Delete all translations": only for the selected language, and only after confirmation.
    pub fn request_delete_translations(&mut self) {
        let mods = self.translations_for_target_lang();
        if mods.is_empty() {
            self.status_message = fmt(
                self.i18n.get(TranslationKey::TransNoneForLang),
                &[self.translations.target_lang.label()],
            );
            return;
        }
        let pending = PendingDelete {
            kind: DeleteKind::Translations,
            ids: mods.iter().map(|m| m.package_id.clone()).collect(),
            names: mods.iter().map(|m| m.name.clone()).collect(),
        };
        self.pending_delete = Some(pending);
    }

    /// Searches the Workshop for translations of the selected mod — in the background (S5).
    pub fn search_workshop_translations_for_selected(&mut self) {
        if self.translations.search_rx.is_some() {
            return;
        }
        let Some(m) = self.selected_mod_id.as_deref().and_then(|id| self.find_mod(id)) else {
            self.status_message = tr!(
                "Выберите мод в списке, чтобы найти для него перевод",
                "Select a mod in the list to find a translation for it",
                "请在列表中选择一个模组以查找它的翻译"
            )
            .to_string();
            return;
        };
        let name = m.name.clone();
        let steam_id = m.steam_item_id.clone();
        let lang = self.translations.target_lang;
        self.translations.show_batch_results = false;
        let query = format!("{} {}", name, lang.search_suffix());
        self.translations.search_query = query.clone();
        self.status_message = tr!(
            "Поиск в Workshop: {}...",
            "Searching the Workshop: {}...",
            "正在搜索创意工坊：{}...",
            query
        );

        let route = self.workshop_route();
        self.translations.search_rx = Some(self.spawn_task(move || {
            let raw = search_steam_workshop(&query, &route)?;
            let found = raw.len();
            // Keep only translations of this mod in the selected language,
            // drop the mod itself and unrelated items, best matches first.
            let filtered = filter_and_rank_workshop_translations(raw, &name, steam_id.as_deref(), lang);
            let status = tr!(
                "Workshop: найдено {}, подходящих переводов «{}»: {}",
                "Workshop: found {}, matching translations of \"{}\": {}",
                "创意工坊：找到 {} 个，「{}」的匹配翻译：{}",
                found,
                name,
                filtered.len()
            );
            Ok((filtered, status))
        }));
    }

    pub fn is_batch_searching(&self) -> bool {
        self.translations.batch_rx.is_some()
    }

    /// "Find translations for the whole modpack" (T1): searches the Workshop for a translation
    /// of every active mod into the selected language, in the background (3 requests at a time).
    /// Skipped: Core/DLC, translation mods, mods that already ship the language, mods that
    /// already have a translation installed. Shown: only items whose title names the mod.
    pub fn start_batch_translation_search(&mut self) {
        if self.translations.batch_rx.is_some() {
            return;
        }
        let lang = self.translations.target_lang;
        let installed_steam: std::collections::HashSet<String> =
            self.all_mods.iter().filter_map(|m| m.steam_item_id.clone()).collect();
        let translated: std::collections::HashSet<String> = self
            .all_mods
            .iter()
            .filter(|m| m.is_translation_mod && lang.matches_mod_info(m))
            .filter_map(|m| m.parent_package_id.as_deref().map(str::to_lowercase))
            .collect();

        let mut skipped = 0;
        let mut seen = std::collections::HashSet::new();
        let mut candidates: Vec<(String, String, Option<String>)> = Vec::new();
        for id in &self.active_mods {
            let Some(m) = self.find_mod(id) else { continue };
            if self.is_official_mod(m) || m.is_translation_mod || !seen.insert(m.package_id.to_lowercase()) {
                continue;
            }
            if lang.is_built_into(m) || translated.contains(&m.package_id.to_lowercase()) {
                skipped += 1;
                continue;
            }
            candidates.push((m.package_id.to_lowercase(), m.name.clone(), m.steam_item_id.clone()));
        }

        self.translations.batch_results.clear();
        self.translations.batch_checked.clear();
        self.translations.batch_lang = lang;
        self.translations.batch_skipped = skipped;
        self.translations.batch_progress = (0, candidates.len());
        self.translations.show_batch_results = true;
        if candidates.is_empty() {
            self.status_message = tr!(
                "Все активные моды уже переведены на {} — искать нечего",
                "Every active mod already has {} — nothing to search",
                "所有已启用的模组都已有 {} — 无需搜索",
                lang.label()
            );
            return;
        }
        self.status_message = tr!(
            "Поиск переводов на {} для {} модов...",
            "Searching {} translations for {} mods...",
            "正在为 {1} 个模组搜索 {0} 翻译...",
            lang.label(),
            candidates.len()
        );

        let stop = Arc::new(AtomicBool::new(false));
        self.translations.batch_stop = stop.clone();
        let (tx, rx) = channel();
        self.translations.batch_rx = Some(rx);
        let ctx = self.repaint_ctx.clone();
        let route = self.workshop_route();
        std::thread::spawn(move || {
            use std::sync::atomic::AtomicUsize;
            let total = candidates.len();
            let next = AtomicUsize::new(0);
            let done = AtomicUsize::new(0);
            let errors = AtomicUsize::new(0);
            std::thread::scope(|s| {
                // A few requests at a time: fast enough, and gentle with Steam.
                for _ in 0..3 {
                    s.spawn(|| loop {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((package_id, name, steam_id)) = candidates.get(i) else {
                            break;
                        };
                        let query = format!("{} {}", name, lang.search_suffix());
                        match search_steam_workshop(&query, &route) {
                            Ok(raw) => {
                                let items = crate::matcher::translations_for_mod(
                                    raw,
                                    name,
                                    steam_id.as_deref(),
                                    lang,
                                    &installed_steam,
                                );
                                if !items.is_empty() {
                                    let _ = tx.send(BatchSearchMsg::Found(ModTranslations {
                                        package_id: package_id.clone(),
                                        mod_name: name.clone(),
                                        items,
                                    }));
                                }
                            }
                            Err(_) => {
                                errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        let _ = tx.send(BatchSearchMsg::Progress { done: d, total });
                        if let Some(ctx) = &ctx {
                            ctx.request_repaint();
                        }
                    });
                }
            });
            let _ = tx.send(BatchSearchMsg::Done {
                searched: done.load(Ordering::Relaxed),
                errors: errors.load(Ordering::Relaxed),
                cancelled: stop.load(Ordering::Relaxed),
            });
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        });
    }

    /// T2: mods for which the last modpack search found a translation that isn't subscribed
    /// yet: lowercase package ID -> the best match's title.
    pub fn translation_hints(&self) -> std::collections::HashMap<String, String> {
        self.translations
            .batch_results
            .iter()
            .filter(|r| {
                !r.items
                    .iter()
                    .any(|i| self.translations.batch_subscribed.contains(&i.id))
            })
            .filter_map(|r| r.items.first().map(|best| (r.package_id.clone(), best.title.clone())))
            .collect()
    }

    /// Opens the Translations tab on the modpack search results.
    pub fn open_batch_translations(&mut self) {
        self.translations.show_batch_results = true;
        self.current_tab = AppTab::Translations;
    }

    pub fn stop_batch_translation_search(&mut self) {
        self.translations.batch_stop.store(true, Ordering::Relaxed);
        self.status_message = tr!("Останавливаю поиск...", "Stopping the search...", "正在停止搜索...").to_string();
    }

    pub(super) fn poll_batch_search(&mut self) {
        let Some(rx) = &self.translations.batch_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BatchSearchMsg::Progress { done, total } => self.translations.batch_progress = (done, total),
                BatchSearchMsg::Found(found) => {
                    // The best match of each mod is ticked; results stay sorted by mod name.
                    if let Some(best) = found.items.first() {
                        self.translations.batch_checked.insert(best.id.clone());
                    }
                    let key = found.mod_name.to_lowercase();
                    let pos = self
                        .translations
                        .batch_results
                        .partition_point(|r| r.mod_name.to_lowercase() < key);
                    self.translations.batch_results.insert(pos, found);
                }
                BatchSearchMsg::Done {
                    searched,
                    errors,
                    cancelled,
                } => finished = Some((searched, errors, cancelled)),
            }
        }
        if let Some((searched, errors, cancelled)) = finished {
            self.translations.batch_rx = None;
            let mut msg = tr!(
                "Проверено модов: {} | с переводом на {}: {}",
                "Mods checked: {} | with a {} translation: {}",
                "已检查模组：{} | 有 {} 翻译的：{}",
                searched,
                self.translations.batch_lang.label(),
                self.translations.batch_results.len()
            );
            if cancelled {
                msg.push_str(tr!(" | остановлено", " | stopped", " | 已停止"));
            }
            if errors > 0 {
                msg.push_str(&tr!(
                    " | без ответа Steam: {}",
                    " | no answer from Steam: {}",
                    " | Steam 无响应：{}",
                    errors
                ));
            }
            self.status_message = msg;
        }
    }

    /// Subscribes to the ticked translations in one Steam session (background).
    pub fn subscribe_checked_translations(&mut self) {
        if self.steam_busy() || self.translations.batch_checked.is_empty() {
            return;
        }
        if self.steamcmd_active() {
            let ids: Vec<String> = self.translations.batch_checked.drain().collect();
            self.translations.batch_subscribed.extend(ids.iter().cloned());
            self.start_steamcmd_job(ids);
            return;
        }
        let Some(dll) = self.steam_paths.steam_dll_path.clone() else {
            self.status_message = tr!(
                "{} не найден — проверьте папку игры в Настройках",
                "{} not found — check the game folder in Settings",
                "找不到 {} — 请在设置中检查游戏文件夹",
                crate::platform::STEAM_API_LIB
            );
            return;
        };
        let ids: Vec<String> = self.translations.batch_checked.drain().collect();
        self.translations.batch_subscribed.extend(ids.iter().cloned());
        self.status_message = tr!(
            "Подписка на переводы: {}...",
            "Subscribing to translations: {}...",
            "正在订阅翻译：{}...",
            ids.len()
        );
        self.start_steam_client_download(dll, ids, false);
    }

    /// Free-text Workshop search — in the background (S5).
    pub fn search_workshop_custom(&mut self) {
        if self.translations.search_rx.is_some() || self.translations.search_query.trim().is_empty() {
            return;
        }
        self.translations.show_batch_results = false;
        let query = self.translations.search_query.clone();
        self.status_message = tr!(
            "Поиск в Workshop: {}...",
            "Searching the Workshop: {}...",
            "正在搜索创意工坊：{}...",
            query
        );
        let route = self.workshop_route();
        self.translations.search_rx = Some(self.spawn_task(move || {
            let res = search_steam_workshop(&query, &route)?;
            let status = tr!(
                "Поиск в Workshop «{}»: результатов {}",
                "Workshop search \"{}\": {} results",
                "创意工坊搜索「{}」：{} 个结果",
                query,
                res.len()
            );
            Ok((res, status))
        }));
    }
}
