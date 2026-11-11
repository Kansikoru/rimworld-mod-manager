//! "What changed" window: a Workshop mod's change notes, the ones newer than the
//! installed copy marked as new.

use eframe::egui;

use super::components::*;
use super::theme::THEME;
use crate::app::App;
use crate::i18n::{fmt, TranslationKey};

const WIDTH: f32 = 560.0;

pub fn render_changelog_window(app: &mut App, ctx: &egui::Context) {
    let Some(window) = app.changelog.as_ref() else {
        return;
    };
    let t = app.i18n;
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let (width, max_height) = dialog_size(ctx, WIDTH);

    egui::Window::new(t.get(TranslationKey::ChangelogTitle))
        .id(egui::Id::unique("changelog_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .max_height(max_height)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&window.title)
                        .color(THEME.accent_cyan)
                        .strong()
                        .size(15.0),
                )
                .wrap(),
            );
            if let Some(installed) = window.installed {
                ui.label(
                    egui::RichText::new(fmt(
                        t.get(TranslationKey::ChangelogInstalled),
                        &[&local_date_time(installed)],
                    ))
                    .color(THEME.text_muted)
                    .size(12.0),
                );
            }
            ui.add_space(6.0);
            match &window.notes {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new(t.get(TranslationKey::ChangelogLoading)).color(THEME.text_muted));
                    });
                }
                Some(Err(e)) => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(fmt(t.get(TranslationKey::ChangelogError), &[e]))
                                .color(THEME.accent_rose),
                        )
                        .wrap(),
                    );
                }
                Some(Ok(notes)) if notes.is_empty() => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(t.get(TranslationKey::ChangelogEmpty)).color(THEME.text_muted),
                        )
                        .wrap(),
                    );
                }
                Some(Ok(notes)) => {
                    for note in notes {
                        card_container(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.label(
                                    egui::RichText::new(local_date_time(note.time))
                                        .color(THEME.text_primary)
                                        .strong(),
                                );
                                if window.installed.is_some_and(|at| note.time > at) {
                                    tag(ui, t.get(TranslationKey::ChangelogNew), THEME.accent_info)
                                        .on_hover_text(t.get(TranslationKey::ChangelogNewHint));
                                }
                            });
                            let (text, color) = if note.text.is_empty() {
                                (t.get(TranslationKey::ChangelogNoText), THEME.text_muted)
                            } else {
                                (note.text.as_str(), THEME.text_secondary)
                            };
                            ui.add(egui::Label::new(egui::RichText::new(text).color(color).size(12.5)).wrap());
                        });
                        ui.add_space(4.0);
                    }
                    ui.label(
                        egui::RichText::new(t.get(TranslationKey::ChangelogLatestOnly))
                            .color(THEME.text_muted)
                            .size(12.0),
                    );
                }
            }
            ui.add_space(6.0);
            if secondary_button(ui, t.get(TranslationKey::WsOpenInBrowser)).clicked() {
                open_external(crate::changelog::page_url(&window.workshop_id));
            }
        });
    if !open {
        app.changelog = None;
    }
}
