use eframe::egui::{self, Color32, Rounding, Stroke, Ui};

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
                    ui.output_mut(|o| o.copied_text = prompt);
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

    super::view_bisect::render_bisect_card(app, ui);
    ui.add_space(8.0);
    render_last_good_card(app, ui);
    ui.add_space(8.0);
    render_load_time_card(app, ui);
    ui.add_space(8.0);

    // Report Results
    let mut switch_to_optimizer = false;
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
                    let dur_str = crate::load_time::format_duration(load_time, app.is_russian);
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
            if render_system_issue_card(ui, t, issue) {
                switch_to_optimizer = true;
            }
            ui.add_space(8.0);
        }

        // Culprits List
        egui::ScrollArea::vertical()
            .id_salt("log_culprits_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                                    culprit.severity.badge_text(app.is_russian),
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
                                    inner_margin: egui::Margin::same(8.0),
                                    rounding: Rounding::same(4.0),
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
            });
    }

    if switch_to_optimizer {
        app.current_tab = crate::app::AppTab::Optimizer;
    }
}

fn render_system_issue_card(ui: &mut Ui, t: crate::i18n::I18n, issue: crate::analyzer::SystemCrashIssue) -> bool {
    let mut go_to_opt = false;
    card_container(ui, |ui| {
        let (icon, color, title, desc, advice) = match issue {
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
            ui.add_space(8.0);
            if primary_button_icon(ui, ICON_OPTIMIZER, t.get(TranslationKey::CrashBtnGoToOptimizer)).clicked() {
                go_to_opt = true;
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

            if !diff.updated.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(t.get(TranslationKey::LastGoodUpdatedAdvice))
                        .color(THEME.text_secondary)
                        .size(12.0),
                );
                ui.add_space(2.0);
                for id in &diff.updated {
                    let name = app.mod_display_name(id);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("•").color(THEME.accent_amber).strong());
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
                );

                if let Some(entry) = latest_entry {
                    let dur_str = crate::load_time::format_duration(entry.duration_secs, app.is_russian);
                    tag(ui, &dur_str, THEME.accent_cyan);
                } else if let Some(secs) = current_log_secs {
                    let dur_str = crate::load_time::format_duration(secs, app.is_russian);
                    tag(ui, &dur_str, THEME.accent_cyan);
                }
            });

            if let Some(entry) = latest_entry {
                if let Some(ref diff) = entry.diff_summary {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let is_faster = diff.starts_with('-');
                        let badge_color = if is_faster {
                            THEME.accent_emerald
                        } else {
                            THEME.accent_amber
                        };
                        tag(ui, diff, badge_color);
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
                                let dur = crate::load_time::format_duration(e.duration_secs, app.is_russian);
                                ui.label(egui::RichText::new(dur).color(THEME.text_primary).size(12.0).strong());
                                ui.label(
                                    egui::RichText::new(format!("({} mods)", e.active_mods.len()))
                                        .color(THEME.text_muted)
                                        .size(11.0),
                                );
                                if let Some(ref d) = e.diff_summary {
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

    // Filter matching lines
    let filtered_lines: Vec<(usize, &str, LineLevel)> = report
        .raw_content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let level = classify_log_line(line);
            let matches_level = match level_filter {
                crate::analyzer::LogLevelFilter::All => true,
                crate::analyzer::LogLevelFilter::Errors => level == LineLevel::Error,
                crate::analyzer::LogLevelFilter::Warnings => level == LineLevel::Warning,
                crate::analyzer::LogLevelFilter::Info => level == LineLevel::Info,
            };
            if !matches_level {
                return None;
            }
            if !filter_text.is_empty() && !line.to_lowercase().contains(&filter_text) {
                return None;
            }
            Some((idx + 1, line, level))
        })
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
        inner_margin: egui::Margin::same(8.0),
        rounding: Rounding::same(6.0),
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
                                ui.output_mut(|o| o.copied_text = line_text.to_string());
                                ui.close_menu();
                            }
                        });
                    });
                }
            });
    });
}
