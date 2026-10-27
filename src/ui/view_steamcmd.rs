//! SteamCMD window: what SteamCMD is doing (it runs hidden), in the manager's own look —
//! the step, a progress bar, each mod with its state, and SteamCMD's own output folded away.

use eframe::egui::{self, Rounding, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::app::App;
use crate::i18n::{fmt, TranslationKey};
use crate::steamcmd::{ItemState, Phase};

const WIDTH: f32 = 600.0;

pub fn render_steamcmd_window(app: &mut App, ctx: &egui::Context) {
    if !app.steamcmd_window {
        return;
    }
    let console = app.steamcmd_console;
    if app.steamcmd_job.is_none() && !console {
        app.steamcmd_window = false;
        return;
    }
    let t = app.i18n;
    let running = app.steamcmd_busy();
    // The console's text is edited apart from `app` (the job is borrowed while drawing).
    let mut input = std::mem::take(&mut app.steamcmd_console_input);
    let empty = crate::steamcmd::Job::new(&[]);
    let has_job = app.steamcmd_job.is_some();
    let job = app.steamcmd_job.as_ref().unwrap_or(&empty);
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let (mut stop, mut close, mut retry, mut run_console) = (false, false, Vec::new(), false);
    let (width, max_height) = dialog_size(ctx, WIDTH);

    egui::Window::new(t.get(TranslationKey::SteamCmdTitle))
        .id(egui::Id::new("steamcmd_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .max_height(max_height)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.set_width(width);

            // The console: own commands, run after an anonymous login.
            if console {
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::SteamCmdConsole))
                        .color(THEME.accent_cyan)
                        .strong(),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t.get(TranslationKey::SteamCmdConsoleHint))
                            .color(THEME.text_muted)
                            .size(12.0),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                egui::Frame {
                    inner_margin: egui::Margin::same(6.0),
                    rounding: Rounding::same(6.0),
                    fill: THEME.bg_dark,
                    stroke: egui::Stroke::new(1.0_f32, THEME.border_subtle),
                    ..Default::default()
                }
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut input)
                            .code_editor()
                            .frame(false)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .hint_text("workshop_download_item 294100 818773962"),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled_ui(!running && !input.trim().is_empty(), |ui| {
                            primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::SteamCmdConsoleRun))
                        })
                        .inner
                        .clicked()
                    {
                        run_console = true;
                    }
                });
                ui.add_space(8.0);
            }

            // The step and the progress bar
            if has_job {
                let (phase_text, phase_color) = match &job.phase {
                    Phase::Starting => (
                        t.get(TranslationKey::SteamCmdPhaseStarting).to_string(),
                        THEME.accent_cyan,
                    ),
                    Phase::Installing => (
                        t.get(TranslationKey::SteamCmdPhaseInstalling).to_string(),
                        THEME.accent_cyan,
                    ),
                    Phase::Updating => (
                        t.get(TranslationKey::SteamCmdPhaseUpdating).to_string(),
                        THEME.accent_cyan,
                    ),
                    Phase::Downloading => (
                        t.get(TranslationKey::SteamCmdPhaseDownloading).to_string(),
                        THEME.accent_cyan,
                    ),
                    Phase::Linking => (
                        t.get(TranslationKey::SteamCmdPhaseLinking).to_string(),
                        THEME.accent_cyan,
                    ),
                    Phase::Console => (
                        t.get(TranslationKey::SteamCmdPhaseConsole).to_string(),
                        THEME.accent_cyan,
                    ),
                    Phase::Done => (
                        t.get(TranslationKey::SteamCmdPhaseDone).to_string(),
                        THEME.accent_emerald,
                    ),
                    Phase::Stopped => (t.get(TranslationKey::SteamCmdStopped).to_string(), THEME.accent_amber),
                    Phase::Failed(e) => (fmt(t.get(TranslationKey::SteamCmdFailed), &[e]), THEME.accent_rose),
                };
                ui.horizontal(|ui| {
                    if running {
                        ui.spinner();
                    }
                    ui.add(egui::Label::new(egui::RichText::new(phase_text).color(phase_color).strong()).wrap());
                });
                ui.add_space(6.0);
                let done = job
                    .items
                    .iter()
                    .filter(|i| !matches!(i.state, ItemState::Waiting | ItemState::Downloading))
                    .count();
                let mut bar = egui::ProgressBar::new(job.progress())
                    .fill(if job.failed_items() > 0 && !running {
                        THEME.accent_amber
                    } else {
                        THEME.accent_purple
                    })
                    .desired_height(8.0)
                    .rounding(Rounding::same(4.0));
                if !job.items.is_empty() {
                    bar = bar.text(
                        egui::RichText::new(fmt(
                            t.get(TranslationKey::SteamCmdCount),
                            &[&done.to_string(), &job.items.len().to_string()],
                        ))
                        .size(11.0),
                    );
                } else if running {
                    bar = bar.animate(true);
                }
                ui.add(bar);
                ui.add_space(8.0);
            }

            // Each mod
            if !job.items.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("steamcmd_items")
                    .max_height((ctx.screen_rect().height() * 0.35).clamp(120.0, 320.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for item in &job.items {
                            egui::Frame {
                                inner_margin: egui::Margin::symmetric(10.0, 5.0),
                                outer_margin: egui::Margin::symmetric(0.0, 2.0),
                                rounding: Rounding::same(6.0),
                                fill: THEME.bg_card,
                                ..Default::default()
                            }
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    if icon_button(ui, ICON_STEAM, Vec2::splat(14.0), THEME.text_muted)
                                        .on_hover_text(t.get(TranslationKey::NameSearchOpenSteam))
                                        .clicked()
                                    {
                                        open_external(crate::steam::workshop_url(&item.id));
                                    }
                                    let (state, color) = match &item.state {
                                        ItemState::Waiting => (
                                            t.get(TranslationKey::SteamCmdStateWaiting).to_string(),
                                            THEME.text_muted,
                                        ),
                                        ItemState::Downloading => (
                                            t.get(TranslationKey::SteamCmdStateDownloading).to_string(),
                                            THEME.accent_cyan,
                                        ),
                                        ItemState::Downloaded => (
                                            t.get(TranslationKey::SteamCmdStateDownloaded).to_string(),
                                            THEME.accent_cyan,
                                        ),
                                        ItemState::Failed(reason) if reason.is_empty() => (
                                            t.get(TranslationKey::SteamCmdStateFailed).to_string(),
                                            THEME.accent_rose,
                                        ),
                                        ItemState::Failed(reason) => (
                                            format!("{}: {reason}", t.get(TranslationKey::SteamCmdStateFailed)),
                                            THEME.accent_rose,
                                        ),
                                        ItemState::Linked => (
                                            t.get(TranslationKey::SteamCmdStateLinked).to_string(),
                                            THEME.accent_emerald,
                                        ),
                                        ItemState::OwnCopy => (
                                            t.get(TranslationKey::SteamCmdStateOwnCopy).to_string(),
                                            THEME.accent_amber,
                                        ),
                                    };
                                    if item.state == ItemState::Downloading {
                                        ui.spinner();
                                    }
                                    tag(ui, &state, color);
                                    let name = if item.title.is_empty() {
                                        format!("Workshop {}", item.id)
                                    } else {
                                        item.title.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(name).color(THEME.text_primary).size(12.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&item.id);
                                });
                            });
                        }
                    });
            }

            let failed: Vec<String> = job
                .items
                .iter()
                .filter(|i| matches!(i.state, ItemState::Failed(_)))
                .map(|i| i.id.clone())
                .collect();
            if !running && !failed.is_empty() {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t.get(TranslationKey::SteamCmdFailedHint))
                            .color(THEME.text_muted)
                            .size(12.0),
                    )
                    .wrap(),
                );
            }

            // SteamCMD's own output, folded away
            ui.add_space(6.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(t.get(TranslationKey::SteamCmdLog))
                    .color(THEME.text_secondary)
                    .size(12.0),
            )
            .id_salt(if console {
                "steamcmd_console_log"
            } else {
                "steamcmd_log"
            })
            .default_open(console)
            .show(ui, |ui| {
                egui::Frame {
                    inner_margin: egui::Margin::same(8.0),
                    rounding: Rounding::same(6.0),
                    fill: THEME.bg_dark,
                    stroke: egui::Stroke::new(1.0_f32, THEME.border_subtle),
                    ..Default::default()
                }
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("steamcmd_log_scroll")
                        .max_height(180.0)
                        .auto_shrink([false, true])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for line in &job.log {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(line).monospace().size(11.0).color(THEME.text_muted),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                });
            });

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                if running {
                    if secondary_button_icon(ui, ICON_STOP, t.get(TranslationKey::SteamCmdStop)).clicked() {
                        stop = true;
                    }
                } else if !failed.is_empty()
                    && primary_button_icon(
                        ui,
                        ICON_REFRESH,
                        &fmt(t.get(TranslationKey::SteamCmdRetry), &[&failed.len().to_string()]),
                    )
                    .clicked()
                {
                    retry = failed.clone();
                }
                if secondary_button(ui, t.get(TranslationKey::SteamCmdClose)).clicked() {
                    close = true;
                }
            });
        });

    app.steamcmd_console_input = input;
    if run_console {
        app.run_steamcmd_console();
    }
    if stop {
        app.stop_steamcmd();
    }
    if !retry.is_empty() {
        app.start_steamcmd_job(retry);
    }
    // Closing only hides the window: a download keeps going (Settings can show it again).
    if !open || close {
        app.steamcmd_window = false;
    }
}
