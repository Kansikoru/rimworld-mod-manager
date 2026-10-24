use eframe::egui::{self, Color32, Rounding, Stroke, Ui};

use crate::analyzer::generate_ai_diagnostic_prompt;
use crate::app::App;
use crate::i18n::TranslationKey;
use super::components::*;
use super::icons::*;
use super::theme::THEME;

pub fn render_analyzer_view(app: &mut App, ui: &mut Ui) {
    let t = app.i18n;

    section_header(
        ui,
        t.get(TranslationKey::CrashTitle),
        t.get(TranslationKey::CrashSubtitle),
        Some(("Player.log Engine", THEME.accent_rose)),
    );

    ui.add_space(8.0);

    // Action Header Card
    card_container(ui, |ui| {
        ui.horizontal(|ui| {
            if primary_button_icon(ui, ICON_SEARCH, t.get(TranslationKey::CrashBtnAnalyze)).clicked() {
                app.run_log_analysis();
            }

            if let Some(ref report) = app.log_report {
                if !report.culprits.is_empty() {
                    if success_button_icon(ui, ICON_COPY, t.get(TranslationKey::CrashBtnCopyPrompt)).clicked() {
                        let prompt = generate_ai_diagnostic_prompt(report, app.is_russian);
                        ui.output_mut(|o| o.copied_text = prompt);
                    }
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

    // Report Results
    if let Some(ref report) = app.log_report {
        // Summary Card
        card_container(ui, |ui| {
            ui.horizontal(|ui| {
                let (icon, color) = if report.has_hard_crash {
                    ("💥", THEME.accent_rose)
                } else if !report.culprits.is_empty() {
                    ("⚠️", THEME.accent_amber)
                } else {
                    ("✅", THEME.accent_emerald)
                };

                let summary_text = if report.has_hard_crash {
                    t.get(TranslationKey::CrashSummaryHardCrash).replace("{}", &report.culprits.len().to_string())
                } else if !report.culprits.is_empty() {
                    t.get(TranslationKey::CrashSummaryCulprits).replace("{}", &report.culprits.len().to_string())
                } else if report.total_errors > 0 {
                    t.get(TranslationKey::CrashSummaryErrorsOnly).replace("{}", &report.total_errors.to_string())
                } else {
                    t.get(TranslationKey::CrashSummaryClean).to_string()
                };

                let file_stats = crate::i18n::fmt(
                    t.get(TranslationKey::CrashFileStats),
                    &[
                        &report.log_file_name,
                        &report.total_lines.to_string(),
                        &report.total_errors.to_string(),
                    ],
                );

                ui.label(egui::RichText::new(icon).size(20.0));
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(summary_text)
                            .color(color)
                            .size(14.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(file_stats)
                            .color(THEME.text_muted)
                            .size(11.0),
                    );
                });
            });
        });

        ui.add_space(8.0);

        // Culprits List
        egui::ScrollArea::vertical()
            .id_salt("log_culprits_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if report.culprits.is_empty() {
                    card_container(ui, |ui| {
                        ui.label(
                            egui::RichText::new(t.get(TranslationKey::CrashNoErrors))
                                .color(THEME.accent_emerald)
                                .size(13.0),
                        );
                    });
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
                                    stroke: Stroke::new(1.0, THEME.border_subtle),
                                    shadow: egui::epaint::Shadow::NONE,
                                    outer_margin: egui::Margin::ZERO,
                                };
                                code_frame.show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(&culprit.stack_trace_excerpt)
                                            .color(Color32::from_rgb(252, 165, 165))
                                            .monospace()
                                            .size(11.0),
                                    );
                                });
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            });
    }
}
