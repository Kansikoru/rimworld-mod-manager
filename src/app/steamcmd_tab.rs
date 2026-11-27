//! SteamCMD: its settings, the install / download job and the console window.

use super::*;

/// State of SteamCMD downloads and its window.
pub struct SteamCmdState {
    /// SteamCMD: download mods with it instead of the Steam client (Settings).
    pub enabled: bool,
    /// The user set the checkbox; until then it follows "SteamCMD is installed".
    pub enabled_set: bool,
    pub custom_dir: Option<PathBuf>,
    /// The last SteamCMD job (install or download) for its window; `rx` while it runs.
    pub job: Option<crate::steamcmd::Job>,
    pub(super) rx: Option<Receiver<crate::steamcmd::Msg>>,
    pub(super) kill: crate::steamcmd::KillSwitch,
    pub window: bool,
    /// The running job installs SteamCMD: once it's in place, downloads go through it.
    pub(super) fresh_install: bool,
    /// The SteamCMD window shows the console (own commands) instead of the last download.
    pub console: bool,
    pub console_input: String,
}

impl App {
    /// The SteamCMD folder: picked in Settings, else next to the settings.
    pub fn steamcmd_dir(&self) -> PathBuf {
        self.steamcmd
            .custom_dir
            .clone()
            .unwrap_or_else(crate::steamcmd::default_dir)
    }

    /// Mods are downloaded with SteamCMD: turned on in Settings, or there is no Steam client.
    pub fn steamcmd_active(&self) -> bool {
        self.steamcmd.enabled || self.steam_paths.steam_path.is_none()
    }

    pub fn steamcmd_busy(&self) -> bool {
        self.steamcmd.rx.is_some()
    }

    /// Installs or updates SteamCMD (nothing to download).
    pub fn install_steamcmd(&mut self) {
        self.start_steamcmd_job(Vec::new());
    }

    /// Downloads Workshop items with SteamCMD in the background (installing it first if
    /// needed) and opens its window; the mods get links in the game's Mods folder.
    pub fn start_steamcmd_job(&mut self, mut ids: Vec<String>) {
        if self.steamcmd_busy() {
            self.steamcmd.window = true;
            return;
        }
        ids.sort();
        ids.dedup();
        let t = self.i18n;
        let mods_dir = self.game_mods_dir();
        if !ids.is_empty() && mods_dir.is_none() {
            self.status_message = t.get(TranslationKey::SteamCmdNoMods).to_string();
            return;
        }
        let dir = self.steamcmd_dir();
        self.steamcmd.fresh_install = !crate::steamcmd::is_installed(&dir);
        self.steamcmd.job = Some(crate::steamcmd::Job::new(&ids));
        self.steamcmd.window = true;
        self.steamcmd.console = false;
        self.status_message = if ids.is_empty() {
            t.get(TranslationKey::SteamCmdInstalling).to_string()
        } else {
            fmt(t.get(TranslationKey::SteamCmdDownloading), &[&ids.len().to_string()])
        };
        let (tx, rx) = channel();
        self.steamcmd.rx = Some(rx);
        let kill = self.steamcmd.kill.clone();
        let ctx = self.repaint_ctx.clone();
        std::thread::spawn(move || {
            let repaint = || {
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            };
            crate::steamcmd::run_job(&dir, &ids, mods_dir.as_deref(), &kill, &tx, &repaint);
        });
    }

    pub fn stop_steamcmd(&mut self) {
        crate::steamcmd::stop(&self.steamcmd.kill);
    }

    /// Opens the SteamCMD window on its console.
    pub fn open_steamcmd_console(&mut self) {
        self.steamcmd.console = true;
        self.steamcmd.window = true;
    }

    /// Runs the console's commands (after an anonymous login) in the background.
    pub fn run_steamcmd_console(&mut self) {
        if self.steamcmd_busy() || self.steamcmd.console_input.trim().is_empty() {
            return;
        }
        let (args, dropped_login) = crate::steamcmd::console_args(&self.steamcmd.console_input);
        let mut job = crate::steamcmd::Job::new(&[]);
        if dropped_login {
            job.apply(crate::steamcmd::Msg::Line(
                self.i18n.get(TranslationKey::SteamCmdConsoleLogin).to_string(),
            ));
        }
        self.steamcmd.job = Some(job);
        self.steamcmd.console = true;
        self.steamcmd.window = true;
        let dir = self.steamcmd_dir();
        let (tx, rx) = channel();
        self.steamcmd.rx = Some(rx);
        let kill = self.steamcmd.kill.clone();
        let ctx = self.repaint_ctx.clone();
        std::thread::spawn(move || {
            let repaint = || {
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            };
            crate::steamcmd::run_console(&dir, &args, &kill, &tx, &repaint);
        });
    }

    pub(super) fn poll_steamcmd(&mut self) {
        let Some(rx) = &self.steamcmd.rx else { return };
        let Some(job) = self.steamcmd.job.as_mut() else { return };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    job.apply(msg);
                    finished |= job.phase.is_finished();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            return;
        }
        let t = self.i18n;
        let (done, failed) = (job.finished_items(), job.failed_items());
        let status = match &job.phase {
            crate::steamcmd::Phase::Failed(e) => fmt(t.get(TranslationKey::SteamCmdFailed), &[e]),
            crate::steamcmd::Phase::Stopped => t.get(TranslationKey::SteamCmdStopped).to_string(),
            _ if job.items.is_empty() => t.get(TranslationKey::SteamCmdReady).to_string(),
            _ => fmt(
                t.get(TranslationKey::SteamCmdDone),
                &[&done.to_string(), &failed.to_string()],
            ),
        };
        let done_ids: std::collections::HashSet<String> = job
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.state,
                    crate::steamcmd::ItemState::Linked | crate::steamcmd::ItemState::OwnCopy
                )
            })
            .map(|i| i.id.clone())
            .collect();
        self.steamcmd.rx = None;
        self.status_message = status;
        // Just installed: from now on every "Download" goes through SteamCMD (Settings can
        // turn it off again).
        if std::mem::take(&mut self.steamcmd.fresh_install)
            && !self.steamcmd.enabled
            && crate::steamcmd::is_installed(&self.steamcmd_dir())
        {
            self.steamcmd.enabled = true;
            self.status_message = format!("{} | {}", self.status_message, t.get(TranslationKey::SteamCmdNowOn));
        }
        if done > 0 {
            if let Some(report) = self.mod_updates.as_mut() {
                report.mods.retain(|_, u| !done_ids.contains(&u.workshop_id));
                if report.outdated_count() == 0 {
                    self.filter_updates = false;
                }
            }
            self.rescan_mods_keep_status();
        }
    }

    /// SC1: outdated mods SteamCMD can update — the ones in the game's Mods folder. Mods the
    /// Steam client keeps in its Workshop folder are left to Steam (a SteamCMD copy would
    /// give the mod twice).
    pub fn steamcmd_updatable_ids(&self) -> Vec<String> {
        let Some(report) = &self.mod_updates else {
            return Vec::new();
        };
        let mut ids: Vec<String> = report
            .mods
            .iter()
            .filter(|(pkg, u)| u.outdated && self.can_update_with_steamcmd(pkg))
            .map(|(_, u)| u.workshop_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// The mod is on disk outside the Steam client's Workshop folder.
    pub fn can_update_with_steamcmd(&self, package_id: &str) -> bool {
        self.find_mod(package_id)
            .is_some_and(|m| m.origin != crate::models::ModOrigin::Workshop && !m.frozen)
    }
}
