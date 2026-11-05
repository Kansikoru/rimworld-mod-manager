use eframe::egui::{self, Color32, CornerRadius, Stroke, Ui};

use super::components::*;
use super::icons::*;
use super::theme::THEME;
use crate::analyzer::generate_ai_diagnostic_prompt;
use crate::app::App;
use crate::i18n::TranslationKey;

pub fn render_analyzer_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    section_header(
        ui,
        t.get(TranslationKey::CrashTitle),
        t.get(TranslationKey::CrashSubtitle),
        None,
    );

    ui.add_space(8.0);

    // Action Header Card
    card_container(ui, |ui| {
        ui.horizontal(|ui| {
            let analyzing = app.log_rx.is_some();
            ui.add_enabled_ui(!analyzing, |ui| {
                if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::CrashBtnAnalyze)).clicked() {
                    app.run_log_analysis();
                }
            });
            if analyzing {
                ui.spinner();
            }

            if let Some(ref report) = app.log_report {
                // Also useful when there are errors but no identified culprit.
                if (!report.culprits.is_empty() || report.total_errors > 0 || report.has_hard_crash)
                    && success_button_icon(ui, ICON_COPY, t.get(TranslationKey::CrashBtnCopyPrompt)).clicked()
                {
                    let prompt = generate_ai_diagnostic_prompt(report, app.is_russian);
                    ui.ctx().copy_text(prompt);
                }

                if secondary_button_icon(ui, ICON_SAVE, t.get(TranslationKey::AnalyzerSaveReport)).clicked() {
                    app.save_log_report();
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button_icon(ui, ICON_FOLDER, t.get(TranslationKey::AnalyzerBrowseLog)).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("RimWorld Log (*.log)", &["log"])
                        .pick_file()
                    {
                        app.custom_log_path = Some(path);
                        app.run_log_analysis();
                    }
                }
            });
        });
    });

    ui.add_space(8.0);

    // Mode Switcher (Report vs Full Log)
    if app.log_report.is_some() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let is_report = app.log_view_mode == crate::analyzer::LogViewMode::Report;
            let is_full = app.log_view_mode == crate::analyzer::LogViewMode::FullLog;

            if small_button(
                ui,
                t.get(TranslationKey::AnalyzerTabReport),
                if is_report {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
            )
            .clicked()
            {
                app.log_view_mode = crate::analyzer::LogViewMode::Report;
            }

            if small_button(
                ui,
                t.get(TranslationKey::AnalyzerTabFullLog),
                if is_full {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
            )
            .clicked()
            {
                app.log_view_mode = crate::analyzer::LogViewMode::FullLog;
            }
        });
        ui.add_space(8.0);
    }

    if app.log_report.is_some() && app.log_view_mode == crate::analyzer::LogViewMode::FullLog {
        render_full_log_view(app, ui);
        return;
    }

    // The page scrolls as a whole (like Optimizer and Settings): nothing below the window
    // edge is cut off.
    let mut switch_to_optimizer = false;
    let mut measure_launch = false;
    egui::ScrollArea::vertical()
        .id_salt("analyzer_page_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            super::view_bisect::render_bisect_card(app, ui);
            ui.add_space(8.0);
            render_last_good_card(app, ui);
            ui.add_space(8.0);
            render_load_time_card(app, ui);
            ui.add_space(8.0);

            // Report Results
            if let Some(ref report) = app.log_report {
                // Summary Card
                card_container(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (icon, color) = if report.has_hard_crash {
                            (ICON_STATUS_CRASH, THEME.accent_rose)
                        } else if !report.culprits.is_empty() {
                            (ICON_STATUS_WARNING, THEME.accent_amber)
                        } else if report.total_errors > 0 {
                            (ICON_STATUS_ERROR, THEME.accent_amber)
                        } else {
                            (ICON_STATUS_OK, THEME.accent_emerald)
                        };

                        let summary_text = if let Some(issue) = report.system_issue {
                            match issue {
                                crate::analyzer::SystemCrashIssue::GpuReset => {
                                    t.get(TranslationKey::CrashGpuResetTitle).to_string()
                                }
                                crate::analyzer::SystemCrashIssue::OutOfMemory => {
                                    t.get(TranslationKey::CrashOomTitle).to_string()
                                }
                            }
                        } else if report.has_hard_crash {
                            t.get(TranslationKey::CrashSummaryHardCrash)
                                .replace("{}", &report.culprits.len().to_string())
                        } else if !report.culprits.is_empty() {
                            t.get(TranslationKey::CrashSummaryCulprits)
                                .replace("{}", &report.culprits.len().to_string())
                        } else if report.total_errors > 0 {
                            t.get(TranslationKey::CrashSummaryErrorsOnly)
                                .replace("{}", &report.total_errors.to_string())
                        } else {
                            t.get(TranslationKey::CrashSummaryClean).to_string()
                        };

                        let mut file_stats = crate::i18n::fmt(
                            t.get(TranslationKey::CrashFileStats),
                            &[
                                &report.log_file_name,
                                &report.total_lines.to_string(),
                                &report.total_errors.to_string(),
                            ],
                        );
                        if let Some(load_time) = report.load_time_secs {
                            let dur_str = crate::load_time::format_duration(load_time);
                            file_stats.push_str(&format!(" • {}: {}", t.get(TranslationKey::LoadTimeTitle), dur_str));
                        }

                        ui.add(svg_icon(icon, egui::Vec2::splat(22.0), color));
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(summary_text).color(color).size(14.0).strong());
                            ui.label(egui::RichText::new(file_stats).color(THEME.text_muted).size(12.0));
                        });
                    });
                });

                ui.add_space(8.0);

                if let Some(issue) = report.system_issue {
                    // On 1.7 the game compresses textures itself: no optimizer to send to.
                    if render_system_issue_card(ui, t, issue, !report.is_waylith) {
                        switch_to_optimizer = true;
                    }
                    ui.add_space(8.0);
                }

                render_culprits(ui, t, report);
                ui.add_space(8.0);
                if render_load_profile_card(app, ui, report) {
                    measure_launch = true;
                }
                ui.add_space(8.0);
            }
            if app.companion_report.is_some() {
                // RWMM_TAB=analyzer-companion (screenshots): the card at the top, once the
                // analysis above it is in.
                if app.log_report.is_some() && std::mem::take(&mut app.scroll_to_companion_report) {
                    ui.scroll_to_cursor(Some(egui::Align::TOP));
                }
                render_companion_report_card(app, ui);
            }
        });

    if measure_launch {
        app.launch_with_load_measure();
    }
    if switch_to_optimizer {
        app.current_tab = crate::app::AppTab::Optimizer;
    }
}

/// The mods the log points at, then performance tips.
fn render_culprits(ui: &mut Ui, t: crate::i18n::I18n, report: &crate::analyzer::LogReport) {
    if report.culprits.is_empty() {
        if report.system_issue.is_none() && !report.has_hard_crash && report.total_errors == 0 {
            card_container(ui, |ui| {
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::CrashNoErrors))
                        .color(THEME.accent_emerald)
                        .size(13.0),
                );
            });
        }
        ui.add_space(8.0);
        render_perf_tips_card(ui, t);
        return;
    }

    for culprit in &report.culprits {
        let (badge_bg, badge_fg) = culprit.severity.badge_colors();

        card_container(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        egui::RichText::new(&culprit.mod_name)
                            .color(THEME.text_primary)
                            .size(15.0)
                            .strong(),
                    );
                    if let Some(ref pkg_id) = culprit.package_id {
                        ui.label(
                            egui::RichText::new(format!("({})", pkg_id))
                                .color(THEME.text_muted)
                                .size(12.0),
                        );
                    }
                    badge(
                        ui,
                        culprit.severity.badge_text(t.language()),
                        Color32::from_rgb(badge_bg[0], badge_bg[1], badge_bg[2]),
                        Color32::from_rgb(badge_fg[0], badge_fg[1], badge_fg[2]),
                    );
                });

                ui.add_space(4.0);

                ui.label(
                    egui::RichText::new(&culprit.headline)
                        .color(THEME.text_secondary)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(&culprit.explanation)
                        .color(THEME.text_muted)
                        .size(12.0),
                );

                if !culprit.stack_trace_excerpt.is_empty() {
                    ui.add_space(6.0);
                    let code_frame = egui::Frame {
                        inner_margin: egui::Margin::same(8),
                        corner_radius: CornerRadius::same(4),
                        fill: Color32::from_rgb(15, 17, 23),
                        stroke: Stroke::new(1.0_f32, THEME.border_subtle),
                        shadow: egui::epaint::Shadow::NONE,
                        outer_margin: egui::Margin::ZERO,
                    };
                    code_frame.show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&culprit.stack_trace_excerpt)
                                .color(Color32::from_rgb(252, 165, 165))
                                .monospace()
                                .size(12.0),
                        );
                    });
                }
            });
        });
        ui.add_space(6.0);
    }

    ui.add_space(6.0);
    render_perf_tips_card(ui, t);
}

fn render_system_issue_card(
    ui: &mut Ui,
    t: crate::i18n::I18n,
    issue: crate::analyzer::SystemCrashIssue,
    optimizer_available: bool,
) -> bool {
    let mut go_to_opt = false;
    card_container(ui, |ui| {
        let (icon, color, title, desc, mut advice) = match issue {
            crate::analyzer::SystemCrashIssue::GpuReset => (
                ICON_STATUS_CRASH,
                THEME.accent_rose,
                t.get(TranslationKey::CrashGpuResetTitle),
                t.get(TranslationKey::CrashGpuResetDesc),
                vec![
                    t.get(TranslationKey::CrashGpuAdviceOptimizer),
                    t.get(TranslationKey::CrashGpuAdviceHighQuality),
                    t.get(TranslationKey::CrashGpuAdviceDriver),
                    t.get(TranslationKey::CrashGpuAdviceEventLog),
                ],
            ),
            crate::analyzer::SystemCrashIssue::OutOfMemory => (
                ICON_STATUS_WARNING,
                THEME.accent_amber,
                t.get(TranslationKey::CrashOomTitle),
                t.get(TranslationKey::CrashOomDesc),
                vec![
                    t.get(TranslationKey::CrashOomAdviceCloseApps),
                    t.get(TranslationKey::CrashOomAdvicePagingFile),
                    t.get(TranslationKey::CrashOomAdviceReduceMods),
                ],
            ),
        };
        if !optimizer_available {
            let opt_tips = [
                t.get(TranslationKey::CrashGpuAdviceOptimizer),
                t.get(TranslationKey::CrashOomAdviceReduceMods),
            ];
            advice.retain(|a| !opt_tips.contains(a));
            if issue == crate::analyzer::SystemCrashIssue::OutOfMemory {
                advice.push(t.get(TranslationKey::CrashOomAdviceFewerMods));
            }
        }

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(icon, egui::Vec2::splat(20.0), color));
                ui.label(egui::RichText::new(title).color(color).size(15.0).strong());
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new(desc).color(THEME.text_secondary).size(13.0));
            ui.add_space(6.0);
            for item in advice {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("•").color(color).strong());
                    ui.label(egui::RichText::new(item).color(THEME.text_primary).size(12.0));
                });
            }
            if optimizer_available {
                ui.add_space(8.0);
                if primary_button_icon(ui, ICON_OPTIMIZER, t.get(TranslationKey::CrashBtnGoToOptimizer)).clicked() {
                    go_to_opt = true;
                }
            }
        });
    });
    go_to_opt
}

pub fn render_last_good_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_INFO, egui::Vec2::splat(18.0), THEME.accent_cyan));
                ui.heading(
                    egui::RichText::new(t.get(TranslationKey::LastGoodTitle))
                        .color(THEME.text_primary)
                        .size(15.0)
                        .strong(),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, t.get(TranslationKey::LastGoodSaveBtn)).clicked() {
                        app.remember_last_good();
                    }
                    if let Some(snap) = &app.last_good {
                        let dt = crate::ui::components::local_date_time(snap.saved_at);
                        ui.label(
                            egui::RichText::new(crate::i18n::fmt(t.get(TranslationKey::LastGoodSavedAt), &[&dt]))
                                .color(THEME.text_muted)
                                .size(12.0),
                        );
                    }
                });
            });

            ui.add_space(4.0);

            let Some(diff) = &app.last_good_diff else {
                if app.last_good.is_none() {
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::LastGoodNoSnapshot))
                            .color(THEME.text_muted)
                            .size(12.0),
                    );
                }
                return;
            };

            if diff.is_empty() {
                ui.horizontal(|ui| {
                    ui.add(svg_icon(ICON_STATUS_OK, egui::Vec2::splat(16.0), THEME.accent_emerald));
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::LastGoodNoChanges))
                            .color(THEME.accent_emerald)
                            .size(13.0),
                    );
                });
                return;
            }

            // Badges showing changes
            ui.horizontal(|ui| {
                if !diff.updated.is_empty() {
                    tag(
                        ui,
                        &crate::i18n::fmt(
                            t.get(TranslationKey::LastGoodUpdated),
                            &[&diff.updated.len().to_string()],
                        ),
                        THEME.accent_amber,
                    );
                }
                if !diff.added.is_empty() {
                    tag(
                        ui,
                        &crate::i18n::fmt(t.get(TranslationKey::LastGoodAdded), &[&diff.added.len().to_string()]),
                        THEME.accent_cyan,
                    );
                }
                if !diff.removed.is_empty() {
                    tag(
                        ui,
                        &crate::i18n::fmt(
                            t.get(TranslationKey::LastGoodRemoved),
                            &[&diff.removed.len().to_string()],
                        ),
                        THEME.text_muted,
                    );
                }
                if diff.order_changed {
                    tag(ui, t.get(TranslationKey::LastGoodOrderChanged), THEME.accent_purple);
                }
            });

            if !diff.updated.is_empty() || !diff.added.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::LastGoodUpdatedAdvice))
                        .color(THEME.text_secondary)
                        .size(12.0),
                );
                ui.add_space(2.0);
                let suspects = diff
                    .updated
                    .iter()
                    .map(|id| (id, THEME.accent_amber))
                    .chain(diff.added.iter().map(|id| (id, THEME.accent_cyan)));
                for (id, color) in suspects {
                    let name = app.mod_display_name(id);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("•").color(color).strong());
                        ui.label(egui::RichText::new(name).color(THEME.text_primary).size(12.0));
                        ui.label(
                            egui::RichText::new(format!("({})", id))
                                .color(THEME.text_muted)
                                .size(11.0),
                        );
                    });
                }
                ui.add_space(6.0);
                if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::LastGoodBtnBisect)).clicked() {
                    app.start_bisect_on_updated();
                }
            }
        });
    });
}

pub fn render_load_time_card(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let latest_entry = app.load_time_history.entries.last();
    let current_log_secs = app.log_report.as_ref().and_then(|r| r.load_time_secs);

    if latest_entry.is_none() && current_log_secs.is_none() {
        return;
    }

    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_ROCKET, egui::Vec2::splat(18.0), THEME.accent_cyan));
                ui.heading(
                    egui::RichText::new(t.get(TranslationKey::LoadTimeTitle))
                        .color(THEME.text_primary)
                        .size(15.0)
                        .strong(),
                )
                .on_hover_text(t.get(TranslationKey::LoadTimeHint));

                if let Some(entry) = latest_entry {
                    let dur_str = crate::load_time::format_duration(entry.duration_secs);
                    tag(ui, &dur_str, THEME.accent_cyan);
                } else if let Some(secs) = current_log_secs {
                    let dur_str = crate::load_time::format_duration(secs);
                    tag(ui, &dur_str, THEME.accent_cyan);
                }
            });

            if let Some(entry) = latest_entry {
                if let Some(diff) = entry.change_text() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let badge_color = if entry.is_slower() {
                            THEME.accent_amber
                        } else {
                            THEME.accent_emerald
                        };
                        tag(ui, &diff, badge_color);
                    });
                }

                if app.load_time_history.entries.len() > 1 {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(t.get(TranslationKey::LoadTimeHistoryTitle))
                            .color(THEME.text_secondary)
                            .size(13.0),
                    )
                    .id_salt("load_time_history_collapse")
                    .show(ui, |ui| {
                        for e in app.load_time_history.entries.iter().rev().take(10) {
                            ui.horizontal(|ui| {
                                let dt = crate::ui::components::local_date_time(e.recorded_at);
                                ui.label(egui::RichText::new(dt).color(THEME.text_muted).size(11.0));
                                let dur = crate::load_time::format_duration(e.duration_secs);
                                ui.label(egui::RichText::new(dur).color(THEME.text_primary).size(12.0).strong());
                                ui.label(
                                    egui::RichText::new(crate::i18n::fmt(
                                        t.get(TranslationKey::LoadTimeModsCount),
                                        &[&e.active_mods.len().to_string()],
                                    ))
                                    .color(THEME.text_muted)
                                    .size(11.0),
                                );
                                if let Some(d) = e.change_text() {
                                    ui.label(egui::RichText::new(d).color(THEME.text_secondary).size(11.0));
                                }
                            });
                        }
                    });
                }
            }
        });
    });
}

/// Which mods load the longest, from the game's own profiler (verbose logging); without it, a
/// button that runs the game once with it. Returns true when that button was pressed.
fn render_load_profile_card(app: &App, ui: &mut Ui, report: &crate::analyzer::LogReport) -> bool {
    let t = app.i18n;
    let secs = |ms: f64| crate::load_time::format_duration((ms / 1000.0) as f32);
    let mut measure = false;
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_ROCKET, egui::Vec2::splat(18.0), THEME.accent_cyan));
                ui.heading(
                    egui::RichText::new(t.get(TranslationKey::LoadProfileTitle))
                        .color(THEME.text_primary)
                        .size(15.0)
                        .strong(),
                )
                .on_hover_text(t.get(TranslationKey::LoadProfileHint));
            });
            let pending = app.measure_next_launch || crate::load_profile::measured_launch_pending();
            let Some(profile) = &report.load_profile else {
                report_note(ui, t.get(TranslationKey::LoadProfileNone), THEME.text_secondary);
                ui.add_space(4.0);
                if pending {
                    report_note(ui, t.get(TranslationKey::LoadProfilePending), THEME.accent_amber);
                } else if primary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::LoadProfileMeasure)).clicked() {
                    measure = true;
                }
                return;
            };
            let by_mods: f64 = profile.mods.iter().map(|m| m.total_ms()).sum();
            report_note(
                ui,
                &crate::i18n::fmt(
                    t.get(TranslationKey::LoadProfileTotal),
                    &[&secs(profile.total_ms), &secs(by_mods)],
                ),
                THEME.text_secondary,
            );
            ui.add_space(4.0);
            report_rows(ui, t, "load_profile_mods", &profile.mods, |ui, m| {
                let name = if m.unmatched_class {
                    m.id.clone()
                } else {
                    app.find_mod(&m.id)
                        .map_or_else(|| m.id.clone(), |info| info.name.clone())
                };
                let mut parts: Vec<String> = Vec::new();
                for (ms, key) in [
                    (m.content_ms, TranslationKey::LoadProfileFiles),
                    (m.classes_ms, TranslationKey::LoadProfileCode),
                    (m.defs_ms + m.init_ms, TranslationKey::LoadProfileDefs),
                ] {
                    if ms >= 50.0 {
                        parts.push(crate::i18n::fmt(t.get(key), &[&secs(ms)]));
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{name} — {}", secs(m.total_ms())))
                            .color(THEME.text_primary)
                            .size(12.0),
                    );
                    if !parts.is_empty() {
                        ui.label(
                            egui::RichText::new(parts.join(" · "))
                                .color(THEME.text_muted)
                                .size(11.5),
                        );
                    }
                    if m.unmatched_class {
                        ui.label(
                            egui::RichText::new(t.get(TranslationKey::LoadProfileUnknownClass))
                                .color(THEME.text_muted)
                                .size(11.5),
                        );
                    }
                    if let Some(&(odd, _)) = app.odd_textures.get(&m.id.to_lowercase()).filter(|c| c.0 > 0) {
                        ui.label(
                            egui::RichText::new(crate::i18n::fmt(
                                t.get(TranslationKey::LoadProfileOddTextures),
                                &[&odd.to_string()],
                            ))
                            .color(THEME.accent_amber)
                            .size(11.5),
                        )
                        .on_hover_text(t.get(TranslationKey::LoadProfileOddTexturesHint));
                    }
                });
            });
            report_note(
                ui,
                &crate::i18n::fmt(t.get(TranslationKey::LoadProfileShared), &[&secs(profile.shared_ms())]),
                THEME.text_muted,
            );
            if app.odd_textures_counting() {
                report_note(ui, t.get(TranslationKey::LoadProfileCounting), THEME.text_muted);
            }
            ui.add_space(4.0);
            if pending {
                report_note(ui, t.get(TranslationKey::LoadProfilePending), THEME.accent_amber);
            } else if secondary_button_icon(ui, ICON_PLAY, t.get(TranslationKey::LoadProfileMeasureAgain)).clicked() {
                measure = true;
            }
        });
    });
    measure
}

/// Rows shown before the rest folds under "Show all".
const REPORT_ROWS: usize = 10;

/// What the companion mod wrote after loading: texture memory per mod, replaced defs and
/// methods several mods patch.
fn render_companion_report_card(app: &App, ui: &mut Ui) {
    let Some(report) = app.companion_report.as_ref() else {
        return;
    };
    let t = app.i18n;
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_SHIELD, egui::Vec2::splat(18.0), THEME.accent_cyan));
                ui.heading(
                    egui::RichText::new(t.get(TranslationKey::CompanionReportTitle))
                        .color(THEME.text_primary)
                        .size(15.0)
                        .strong(),
                )
                .on_hover_text(t.get(TranslationKey::CompanionReportHint));
            });
            let when = local_date_time(u64::try_from(report.written_unix).unwrap_or(0));
            let game = report.game.as_deref().unwrap_or("?");
            report_note(
                ui,
                &crate::i18n::fmt(t.get(TranslationKey::CompanionReportRun), &[&when, game]),
                THEME.text_muted,
            );
            if report.mods_differ(&app.active_mods) {
                report_note(
                    ui,
                    t.get(TranslationKey::CompanionReportModsChanged),
                    THEME.accent_amber,
                );
            }
            for error in &report.errors {
                report_note(
                    ui,
                    &crate::i18n::fmt(t.get(TranslationKey::CompanionReportPartFailed), &[error]),
                    THEME.accent_rose,
                );
            }
            ui.add_space(6.0);
            render_report_textures(ui, t, report);
            ui.add_space(8.0);
            render_report_defs(ui, t, report, |id| {
                crate::app::lookup_mod(&app.mod_index, &app.all_mods, id)
            });
            ui.add_space(8.0);
            render_report_patches(ui, t, report);
        });
    });
}

fn report_note(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(text).color(color).size(11.5)).wrap())
}

fn report_subheading(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(egui::RichText::new(text).color(THEME.text_primary).size(13.0).strong())
}

/// The first rows, then the rest under "Show all (N)".
fn report_rows<T>(ui: &mut Ui, t: crate::i18n::I18n, id: &str, rows: &[T], mut row: impl FnMut(&mut Ui, &T)) {
    for r in rows.iter().take(REPORT_ROWS) {
        row(ui, r);
    }
    if rows.len() > REPORT_ROWS {
        egui::CollapsingHeader::new(
            egui::RichText::new(crate::i18n::fmt(
                t.get(TranslationKey::CompanionShowAll),
                &[&rows.len().to_string()],
            ))
            .color(THEME.text_secondary)
            .size(12.0),
        )
        .id_salt(id)
        .show(ui, |ui| {
            for r in &rows[REPORT_ROWS..] {
                row(ui, r);
            }
        });
    }
}

fn size_text(t: crate::i18n::I18n, bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        crate::i18n::fmt(t.get(TranslationKey::CompanionGb), &[&format!("{:.1}", mb / 1024.0)])
    } else {
        crate::i18n::fmt(t.get(TranslationKey::CompanionMb), &[&format!("{mb:.0}")])
    }
}

fn render_report_textures(ui: &mut Ui, t: crate::i18n::I18n, report: &crate::companion_report::CompanionReport) {
    report_subheading(ui, t.get(TranslationKey::CompanionVramTitle));
    let Some(textures) = &report.textures else {
        // A failed part is already among the errors; otherwise it is 1.7.
        if !report.errors.iter().any(|e| e.starts_with("textures")) {
            report_note(ui, t.get(TranslationKey::CompanionVramNotOn17), THEME.text_muted);
        }
        return;
    };
    let mut total = crate::i18n::fmt(
        t.get(TranslationKey::CompanionVramTotal),
        &[
            &size_text(t, textures.all_bytes),
            &textures.all_count.to_string(),
            &size_text(t, textures.vram_mb * 1024 * 1024),
        ],
    );
    // Unity counts it only with texture streaming on; RimWorld 1.6 gives 0.
    if textures.engine_bytes > 0 {
        total.push_str(" · ");
        total.push_str(&crate::i18n::fmt(
            t.get(TranslationKey::CompanionVramEngine),
            &[&size_text(t, textures.engine_bytes)],
        ));
    }
    report_note(ui, &total, THEME.text_secondary);
    let heaviest = report.heaviest_mods();
    report_rows(ui, t, "companion_vram_all", &heaviest, |ui, m| {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(crate::i18n::fmt(
                    t.get(TranslationKey::CompanionVramModLine),
                    &[
                        report.mod_name(m.mod_index),
                        &size_text(t, m.bytes),
                        &m.count.to_string(),
                    ],
                ))
                .color(THEME.text_primary)
                .size(12.0),
            );
            if m.uncompressed > 0 {
                ui.label(
                    egui::RichText::new(crate::i18n::fmt(
                        t.get(TranslationKey::CompanionVramUncompressed),
                        &[&m.uncompressed.to_string(), &size_text(t, m.uncompressed_bytes)],
                    ))
                    .color(THEME.accent_amber)
                    .size(11.5),
                )
                .on_hover_text(t.get(TranslationKey::CompanionVramUncompressedHint));
            }
        });
    });
}

/// Replaced defs. Pairs where one mod names the other in About.xml (an add-on, a patch) are
/// folded away as intended; the rest come with a question, not a verdict, and links to both
/// mods' Workshop pages, where their authors say how they are meant to be used.
fn render_report_defs<'a>(
    ui: &mut Ui,
    t: crate::i18n::I18n,
    report: &crate::companion_report::CompanionReport,
    find_mod: impl Fn(&str) -> Option<&'a crate::models::ModInfo>,
) {
    use crate::companion_report::{pair_is_declared, OverridePair};
    if report.def_overrides.is_none() {
        return;
    }
    report_subheading(ui, t.get(TranslationKey::CompanionDefsTitle))
        .on_hover_text(t.get(TranslationKey::CompanionDefsHint));
    let (between, over_game) = report.override_pairs();
    let (intended, unexplained): (Vec<OverridePair>, Vec<OverridePair>) = between.into_iter().partition(|p| {
        let (w, l) = report.pair_ids(p);
        pair_is_declared(find_mod(w), w, find_mod(l), l)
    });
    let workshop_links = |ui: &mut Ui, p: &OverridePair| {
        let (w, l) = report.pair_ids(p);
        for (index, id) in [(p.winner, w), (p.loser, l)] {
            if let Some(steam_id) = find_mod(id).and_then(|m| m.steam_item_id.as_deref()) {
                let text = crate::i18n::fmt(t.get(TranslationKey::CompanionDefsPage), &[report.mod_name(index)]);
                if ui.link(egui::RichText::new(text).size(11.5)).clicked() {
                    open_in_workshop_tab(ui.ctx(), steam_id);
                }
            }
        }
    };
    let pair_row = |ui: &mut Ui, p: &OverridePair, links: bool| {
        let examples: Vec<&str> = p.defs.iter().take(3).map(String::as_str).collect();
        let more = if p.defs.len() > 3 { ", …" } else { "" };
        let all: Vec<&str> = p.defs.iter().take(40).map(String::as_str).collect();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(crate::i18n::fmt(
                    t.get(TranslationKey::CompanionDefsPair),
                    &[
                        report.mod_name(p.winner),
                        report.mod_name(p.loser),
                        &p.defs.len().to_string(),
                    ],
                ))
                .color(THEME.text_primary)
                .size(12.0),
            );
            ui.label(
                egui::RichText::new(format!("{}{more}", examples.join(", ")))
                    .color(THEME.text_muted)
                    .size(11.5),
            )
            .on_hover_text(all.join("\n"));
            if links {
                workshop_links(ui, p);
            }
        });
    };
    if unexplained.is_empty() && intended.is_empty() {
        report_note(ui, t.get(TranslationKey::CompanionDefsNone), THEME.accent_emerald);
    }
    if !unexplained.is_empty() {
        report_note(
            ui,
            &crate::i18n::fmt(
                t.get(TranslationKey::CompanionDefsBetween),
                &[&unexplained.len().to_string()],
            ),
            THEME.accent_amber,
        );
        report_note(ui, t.get(TranslationKey::CompanionDefsCheck), THEME.text_muted);
        report_rows(ui, t, "companion_defs_between", &unexplained, |ui, p| {
            pair_row(ui, p, true)
        });
    }
    if !intended.is_empty() {
        egui::CollapsingHeader::new(
            egui::RichText::new(crate::i18n::fmt(
                t.get(TranslationKey::CompanionDefsIntended),
                &[&intended.len().to_string()],
            ))
            .color(THEME.text_secondary)
            .size(12.0),
        )
        .id_salt("companion_defs_intended")
        .show(ui, |ui| {
            report_rows(ui, t, "companion_defs_intended_all", &intended, |ui, p| {
                pair_row(ui, p, false)
            })
        });
    }
    if !over_game.is_empty() {
        egui::CollapsingHeader::new(
            egui::RichText::new(crate::i18n::fmt(
                t.get(TranslationKey::CompanionDefsOverGame),
                &[&over_game.len().to_string()],
            ))
            .color(THEME.text_secondary)
            .size(12.0),
        )
        .id_salt("companion_defs_game")
        .show(ui, |ui| {
            report_rows(ui, t, "companion_defs_game_all", &over_game, |ui, p| {
                pair_row(ui, p, false)
            })
        });
    }
}

fn render_report_patches(ui: &mut Ui, t: crate::i18n::I18n, report: &crate::companion_report::CompanionReport) {
    report_subheading(ui, t.get(TranslationKey::CompanionPatchesTitle))
        .on_hover_text(t.get(TranslationKey::CompanionPatchesHint));
    let Some(harmony) = &report.harmony else {
        if !report.errors.iter().any(|e| e.starts_with("harmony")) {
            report_note(ui, t.get(TranslationKey::CompanionPatchesNoHarmony), THEME.text_muted);
        }
        return;
    };
    use crate::companion_report::PatchRisk;
    let methods = report.shared_methods();
    let count = |risk: PatchRisk| methods.iter().filter(|m| m.risk == risk).count().to_string();
    report_note(
        ui,
        &crate::i18n::fmt(
            t.get(TranslationKey::CompanionPatchesSummary),
            &[
                harmony.version.as_deref().unwrap_or("?"),
                &harmony.patched_methods.to_string(),
                &methods.len().to_string(),
                &count(PatchRisk::High),
                &count(PatchRisk::Some),
            ],
        ),
        THEME.text_secondary,
    );
    if methods.is_empty() {
        report_note(ui, t.get(TranslationKey::CompanionPatchesNone), THEME.accent_emerald);
        return;
    }
    let can_cancel = t.get(TranslationKey::CompanionPatchCanCancel);
    report_rows(ui, t, "companion_patches_all", &methods, |ui, m| {
        let patchers: Vec<String> = m
            .patchers
            .iter()
            .map(|p| {
                let mut kinds = p.kinds.join(", ");
                if p.skips {
                    kinds.push_str(" · ");
                    kinds.push_str(can_cancel);
                }
                format!("{} ({kinds})", p.name)
            })
            .collect();
        ui.horizontal_wrapped(|ui| {
            let color = match m.risk {
                PatchRisk::High => THEME.accent_rose,
                PatchRisk::Some => THEME.accent_amber,
                PatchRisk::Low => THEME.text_primary,
            };
            ui.label(
                egui::RichText::new(crate::companion_report::short_method(&m.method))
                    .color(color)
                    .size(12.0)
                    .strong(),
            )
            .on_hover_text(m.method.as_str());
            ui.label(
                egui::RichText::new(patchers.join("; "))
                    .color(THEME.text_muted)
                    .size(11.5),
            );
        });
    });
}

fn render_perf_tips_card(ui: &mut Ui, t: crate::i18n::I18n) {
    card_container(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(svg_icon(ICON_ROCKET, egui::Vec2::splat(18.0), THEME.accent_cyan));
                ui.heading(
                    egui::RichText::new(t.get(TranslationKey::AnalyzerPerfTitle))
                        .color(THEME.text_primary)
                        .size(14.0)
                        .strong(),
                );
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(t.get(TranslationKey::AnalyzerPerfBody))
                    .color(THEME.text_secondary)
                    .size(12.0),
            );
        });
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineLevel {
    Error,
    Warning,
    Info,
}

fn classify_log_line(line: &str) -> LineLevel {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error")
        || lower.contains("exception")
        || lower.contains("failed to")
        || lower.contains("could not load")
        || lower.contains("device_removed")
        || lower.contains("crash")
    {
        LineLevel::Error
    } else if lower.contains("warning") || lower.contains("warn") {
        LineLevel::Warning
    } else {
        LineLevel::Info
    }
}

/// A matching log line: (line number, byte start, byte end, level).
type LogRow = (usize, usize, usize, LineLevel);
/// The last filter of the full log view: (search, level, log file, log size) and its rows.
type FilterCache = (
    (String, crate::analyzer::LogLevelFilter, std::path::PathBuf, usize),
    std::sync::Arc<Vec<LogRow>>,
);

/// Lines of `content` of the given level that contain `filter` (already lower case).
fn filter_log_lines(content: &str, filter: &str, level_filter: crate::analyzer::LogLevelFilter) -> Vec<LogRow> {
    let mut rows = Vec::new();
    let mut start = 0usize;
    for (idx, raw) in content.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let (line_start, line_end) = (start, start + line.len());
        start += raw.len();
        let level = classify_log_line(line);
        let matches_level = match level_filter {
            crate::analyzer::LogLevelFilter::All => true,
            crate::analyzer::LogLevelFilter::Errors => level == LineLevel::Error,
            crate::analyzer::LogLevelFilter::Warnings => level == LineLevel::Warning,
            crate::analyzer::LogLevelFilter::Info => level == LineLevel::Info,
        };
        if matches_level && (filter.is_empty() || line.to_lowercase().contains(filter)) {
            rows.push((idx + 1, line_start, line_end, level));
        }
    }
    rows
}

fn render_full_log_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;
    let Some(ref report) = app.log_report else { return };

    card_container(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            search_input_width(
                ui,
                &mut app.log_filter_text,
                t.get(TranslationKey::AnalyzerFilterHint),
                220.0,
            );

            let all_sel = app.log_level_filter == crate::analyzer::LogLevelFilter::All;
            let err_sel = app.log_level_filter == crate::analyzer::LogLevelFilter::Errors;
            let warn_sel = app.log_level_filter == crate::analyzer::LogLevelFilter::Warnings;
            let info_sel = app.log_level_filter == crate::analyzer::LogLevelFilter::Info;

            if small_button(
                ui,
                t.get(TranslationKey::AnalyzerFilterAll),
                if all_sel {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
            )
            .clicked()
            {
                app.log_level_filter = crate::analyzer::LogLevelFilter::All;
            }
            if small_button(
                ui,
                t.get(TranslationKey::AnalyzerFilterErrors),
                if err_sel {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
            )
            .clicked()
            {
                app.log_level_filter = crate::analyzer::LogLevelFilter::Errors;
            }
            if small_button(
                ui,
                t.get(TranslationKey::AnalyzerFilterWarnings),
                if warn_sel {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
            )
            .clicked()
            {
                app.log_level_filter = crate::analyzer::LogLevelFilter::Warnings;
            }
            if small_button(
                ui,
                t.get(TranslationKey::AnalyzerFilterInfo),
                if info_sel {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
            )
            .clicked()
            {
                app.log_level_filter = crate::analyzer::LogLevelFilter::Info;
            }
        });
    });

    ui.add_space(6.0);

    let filter_text = app.log_filter_text.trim().to_lowercase();
    let level_filter = app.log_level_filter;

    // Matching lines, worked out again only when the search, the level or the log change: a
    // log has tens of thousands of lines, too many to sort through every frame.
    let key = (
        filter_text,
        level_filter,
        report.log_path.clone(),
        report.raw_content.len(),
    );
    let cache_id = egui::Id::new("analyzer_full_log_filter");
    let cached: Option<FilterCache> = ui.data(|d| d.get_temp(cache_id));
    let rows = match cached {
        Some(c) if c.0 == key => c.1,
        _ => {
            let rows = std::sync::Arc::new(filter_log_lines(&report.raw_content, &key.0, level_filter));
            ui.data_mut(|d| d.insert_temp(cache_id, (key, rows.clone())));
            rows
        }
    };
    let filtered_lines: Vec<(usize, &str, LineLevel)> = rows
        .iter()
        .map(|&(line_no, start, end, level)| (line_no, &report.raw_content[start..end], level))
        .collect();

    let total_lines = report.total_lines;
    let shown_count = filtered_lines.len();

    // Stats bar
    ui.horizontal(|ui| {
        tag(
            ui,
            &format!(
                "{}: {} / {}",
                t.get(TranslationKey::AnalyzerTabFullLog),
                shown_count,
                total_lines
            ),
            THEME.text_muted,
        );
    });

    ui.add_space(4.0);

    const LOG_ROW_H: f32 = 20.0;
    let code_frame = egui::Frame {
        inner_margin: egui::Margin::same(8),
        corner_radius: CornerRadius::same(6),
        fill: Color32::from_rgb(15, 17, 23),
        stroke: Stroke::new(1.0_f32, THEME.border_subtle),
        shadow: egui::epaint::Shadow::NONE,
        outer_margin: egui::Margin::ZERO,
    };

    code_frame.show(ui, |ui| {
        egui::ScrollArea::both()
            .id_salt("analyzer_full_log_scroll")
            .auto_shrink([false, false])
            .show_rows(ui, LOG_ROW_H, filtered_lines.len(), |ui, range| {
                for &(line_no, line_text, level) in &filtered_lines[range] {
                    let text_color = match level {
                        LineLevel::Error => THEME.accent_rose,
                        LineLevel::Warning => THEME.accent_amber,
                        LineLevel::Info => THEME.text_secondary,
                    };

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.allocate_ui(egui::vec2(50.0, LOG_ROW_H), |ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(format!("{}", line_no))
                                        .color(THEME.text_muted)
                                        .monospace()
                                        .size(11.0),
                                );
                            });
                        });

                        let resp = ui.add(
                            egui::Label::new(egui::RichText::new(line_text).color(text_color).monospace().size(11.0))
                                .selectable(true),
                        );

                        resp.context_menu(|ui| {
                            if ui.button(t.get(TranslationKey::AnalyzerCopyLine)).clicked() {
                                ui.ctx().copy_text(line_text.to_string());
                                ui.close();
                            }
                        });
                    });
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_log_filter_slices_lines_exactly() {
        let log = "Loading mods\r\nException: boom\r\nWarning: careful\nplain line\n";
        let all = filter_log_lines(log, "", crate::analyzer::LogLevelFilter::All);
        assert_eq!(all.len(), 4);
        let text: Vec<&str> = all.iter().map(|&(_, s, e, _)| &log[s..e]).collect();
        assert_eq!(
            text,
            ["Loading mods", "Exception: boom", "Warning: careful", "plain line"]
        );

        let errors = filter_log_lines(log, "", crate::analyzer::LogLevelFilter::Errors);
        assert_eq!(
            errors.iter().map(|r| r.0).collect::<Vec<_>>(),
            [2],
            "line numbers from 1"
        );

        let found = filter_log_lines(log, "careful", crate::analyzer::LogLevelFilter::All);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].3, LineLevel::Warning);
    }
}
