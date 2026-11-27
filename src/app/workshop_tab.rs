//! Workshop tab: browsing the Steam Workshop, collections, previews and downloads.

use super::*;

/// State of the Workshop tab: the page shown, its items, previews by URL.
pub struct WorkshopTabState {
    pub query: crate::workshop_browse::BrowseQuery,
    pub search: String,
    pub items: Vec<crate::workshop_browse::WorkshopItem>,
    pub selected: Option<String>,
    pub error: Option<String>,
    pub(super) rx: Option<Receiver<Result<crate::workshop_browse::Loaded, String>>>,
    /// Where the last list came from: once the website refuses, the Steam client goes first.
    pub source: crate::workshop_browse::Source,
    pub(super) loaded: bool,
    /// The picked collection's mods: its ID and the result.
    pub collection: Option<CollectionMods>,
    pub(super) collection_rx: Option<Receiver<CollectionMods>>,
    /// Items sent to Steam or SteamCMD from the tab this session.
    pub requested: std::collections::HashSet<String>,
    pub(super) previews: std::collections::HashMap<String, PreviewSlot>,
    pub(super) preview_tx: Option<std::sync::mpsc::Sender<(String, String, u32)>>,
    pub(super) preview_rx: Option<Receiver<(String, Option<egui::ColorImage>)>>,
}

impl App {
    /// Loads the first page the first time the tab is shown.
    pub fn ensure_workshop_loaded(&mut self) {
        if !self.workshop.loaded {
            self.workshop.loaded = true;
            self.browse_workshop();
        }
    }

    /// Loads `ws_query`'s page in the background.
    pub fn browse_workshop(&mut self) {
        if self.workshop.rx.is_some() {
            return;
        }
        self.workshop.error = None;
        let query = self.workshop.query.clone();
        let route = self.workshop_route();
        self.workshop.rx = Some(self.spawn_task(move || crate::workshop_browse::load_page(&query, &route)));
    }

    /// With the setting on, the Steam client is the other way to the Workshop (the game's
    /// `steam_api64.dll` is needed for it), and goes first once the website has refused.
    pub(super) fn workshop_route(&self) -> crate::workshop_browse::Route {
        crate::workshop_browse::Route {
            steam_dll: self
                .workshop_via_steam_client
                .then(|| self.steam_paths.steam_dll_path.clone())
                .flatten(),
            prefer_steam: self.workshop.source == crate::workshop_browse::Source::SteamClient,
        }
    }

    pub fn workshop_loading(&self) -> bool {
        self.workshop.rx.is_some()
    }

    /// Opens the Workshop tab searching for a mod's name (version tags dropped). The tab
    /// opens even when it's hidden in Settings.
    pub fn open_workshop_search(&mut self, name: &str) {
        self.workshop.search = crate::name_search::clean_name(name);
        self.workshop.query.section = crate::workshop_browse::Section::Mods;
        self.workshop.selected = None;
        self.workshop.loaded = true;
        self.current_tab = AppTab::Workshop;
        // A page still loading would be replaced by the search right after.
        self.workshop.rx = None;
        self.search_workshop_tab();
    }

    /// Opens mods in the Workshop tab instead of the browser: the grid shows just them, the
    /// first one's card on the right ("Open the Steam page" is still there). The tab opens
    /// even when hidden.
    pub fn open_workshop_items(&mut self, item_ids: Vec<String>) {
        let Some(first) = item_ids.first() else {
            return;
        };
        self.workshop.selected = Some(first.clone());
        self.workshop.loaded = true;
        self.workshop.error = None;
        self.workshop.query.section = crate::workshop_browse::Section::Mods;
        self.current_tab = AppTab::Workshop;
        let route = self.workshop_route();
        self.workshop.rx = Some(self.spawn_task(move || crate::workshop_browse::load_items(&item_ids, &route)));
    }

    /// "Mods" / "Collections": page 1 of the other section (a page still loading is dropped).
    pub fn set_workshop_section(&mut self, section: crate::workshop_browse::Section) {
        if self.workshop.query.section == section {
            return;
        }
        self.workshop.query.section = section;
        self.workshop.selected = None;
        self.workshop.items.clear();
        self.workshop.rx = None;
        self.search_workshop_tab();
    }

    /// Loads the picked collection's mods once (in the background).
    pub fn ensure_collection_mods(&mut self) {
        let Some(item) = self
            .workshop
            .selected
            .as_ref()
            .and_then(|id| self.workshop.items.iter().find(|i| &i.id == id))
            .filter(|i| i.is_collection)
        else {
            return;
        };
        let loaded = self.workshop.collection.as_ref().is_some_and(|(id, _)| id == &item.id);
        if loaded || self.workshop.collection_rx.is_some() {
            return;
        }
        let (id, children) = (item.id.clone(), item.children.clone());
        let route = self.workshop_route();
        self.workshop.collection_rx =
            Some(self.spawn_task(move || (id, crate::workshop_browse::load_collection_mods(&children, &route))));
    }

    /// "Save as a modpack" on the Workshop tab: the collection becomes a new modpack.
    pub fn import_workshop_collection(&mut self, id: &str) {
        if let Err(e) = self.import_steam_collection(id) {
            self.status_message = e;
        }
    }

    /// A new search or sort: back to page 1.
    pub fn search_workshop_tab(&mut self) {
        self.workshop.query.text = self.workshop.search.trim().to_string();
        self.workshop.query.page = 1;
        self.browse_workshop();
    }

    pub fn workshop_page(&mut self, page: u32) {
        if self.workshop.rx.is_some() {
            return;
        }
        self.workshop.query.page = page.max(1);
        self.browse_workshop();
    }

    pub(super) fn poll_workshop(&mut self) {
        if let Some(rx) = &self.workshop.rx {
            if let Ok(result) = rx.try_recv() {
                self.workshop.rx = None;
                match result {
                    Ok(loaded) => {
                        let items = loaded.items;
                        if !self
                            .workshop
                            .selected
                            .as_ref()
                            .is_some_and(|s| items.iter().any(|i| &i.id == s))
                        {
                            self.workshop.selected = None;
                        }
                        self.workshop.items = items;
                        self.workshop.source = loaded.source;
                    }
                    Err(e) => {
                        // The site refused and the other way is turned off: say where it is.
                        let hint = (!self.workshop_via_steam_client && self.steam_paths.steam_dll_path.is_some())
                            .then(|| format!("\n{}", crate::i18n::text(crate::i18n::TranslationKey::WsSteamClientOff)));
                        self.workshop.error = Some(e + hint.as_deref().unwrap_or(""));
                    }
                }
            }
        }
        if let Some(rx) = &self.workshop.collection_rx {
            if let Ok(result) = rx.try_recv() {
                self.workshop.collection_rx = None;
                self.workshop.collection = Some(result);
            }
        }
        if let Some(rx) = &self.workshop.preview_rx {
            let ctx = self.repaint_ctx.clone();
            while let Ok((key, image)) = rx.try_recv() {
                let slot = match (image, &ctx) {
                    (Some(image), Some(ctx)) => {
                        PreviewSlot::Ready(ctx.load_texture(&key, image, egui::TextureOptions::LINEAR))
                    }
                    _ => PreviewSlot::Failed,
                };
                self.workshop.previews.insert(key, slot);
            }
        }
    }

    /// A Workshop preview at `width` px: the texture once it's downloaded (four at a time,
    /// in the background); None while loading or when there's none.
    pub fn workshop_preview(&mut self, url: &str, width: u32) -> Option<egui::TextureHandle> {
        let key = format!("{width}:{url}");
        if let Some(slot) = self.workshop.previews.get(&key) {
            return match slot {
                PreviewSlot::Ready(t) => Some(t.clone()),
                _ => None,
            };
        }
        // Many pages browsed: the oldest textures go (they load again if needed).
        if self.workshop.previews.len() > 400 {
            self.workshop.previews.retain(|_, s| matches!(s, PreviewSlot::Loading));
        }
        if self.workshop.preview_tx.is_none() {
            self.start_preview_workers();
        }
        if let Some(tx) = &self.workshop.preview_tx {
            let _ = tx.send((key.clone(), url.to_string(), width));
            self.workshop.previews.insert(key, PreviewSlot::Loading);
        }
        None
    }

    pub(super) fn start_preview_workers(&mut self) {
        let (req_tx, req_rx) = channel::<(String, String, u32)>();
        let (res_tx, res_rx) = channel();
        let req_rx = Arc::new(std::sync::Mutex::new(req_rx));
        for _ in 0..4 {
            let (req_rx, res_tx, ctx) = (req_rx.clone(), res_tx.clone(), self.repaint_ctx.clone());
            std::thread::spawn(move || loop {
                let next = req_rx.lock().ok().and_then(|rx| rx.recv().ok());
                let Some((key, url, width)) = next else { break };
                let image =
                    crate::workshop_browse::load_preview(&crate::workshop_browse::sized_preview(&url, width), width)
                        .map(|rgba| {
                            egui::ColorImage::from_rgba_unmultiplied(
                                [rgba.width() as usize, rgba.height() as usize],
                                rgba.as_raw(),
                            )
                        });
                if res_tx.send((key, image)).is_err() {
                    break;
                }
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            });
        }
        self.workshop.preview_tx = Some(req_tx);
        self.workshop.preview_rx = Some(res_rx);
    }

    /// Workshop IDs on disk: Workshop mods, and folders named by a Workshop ID in the game's
    /// Mods folder (SteamCMD links, copies on a game without Steam).
    pub fn installed_workshop_ids(&self) -> std::collections::HashSet<String> {
        self.all_mods
            .iter()
            .filter_map(|m| {
                m.steam_item_id.clone().or_else(|| {
                    m.dir_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .filter(|n| crate::modlists::is_workshop_id(n))
                        .map(str::to_string)
                })
            })
            .collect()
    }

    /// "Download" on the Workshop tab: Steam subscription or SteamCMD, as set up.
    pub fn download_workshop_item(&mut self, id: &str) {
        if self.steam_busy() {
            return;
        }
        self.workshop.requested.insert(id.to_string());
        self.subscribe_steam_mod(id);
    }

    /// W1: downloads the Workshop item together with its missing dependencies.
    pub fn download_workshop_items(&mut self, ids: Vec<String>) {
        if self.steam_busy() || ids.is_empty() {
            return;
        }
        for id in &ids {
            self.workshop.requested.insert(id.clone());
        }
        if ids.len() == 1 {
            self.subscribe_steam_mod(&ids[0]);
        } else {
            self.subscribe_workshop_ids(ids);
        }
    }
}
