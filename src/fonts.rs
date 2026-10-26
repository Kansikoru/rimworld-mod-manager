use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, FontTweak, TextStyle};
use std::path::PathBuf;

use crate::ui_prefs::UiPrefs;

fn windows_font(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("Fonts");
    std::fs::read(dir.join(name)).ok()
}

/// Loads the interface font chosen in the preferences and sets the base text sizes.
/// Call again after the preferences change.
pub fn setup_custom_fonts(ctx: &egui::Context, prefs: &UiPrefs) {
    let mut fonts = FontDefinitions::default();
    let scale = prefs.text_scale;

    // Primary interface font (with Cyrillic). FontTweak::scale makes every glyph bigger;
    // glyphs are still rasterized at the final pixel size, so this stays sharp.
    // A bundled font goes first; the Windows font then fills in letters it lacks.
    let tweak = FontTweak {
        scale,
        ..Default::default()
    };
    let mut primary: Vec<(&str, FontData)> = Vec::new();
    if let Some(bytes) = prefs.font.bundled() {
        primary.push(("BundledPrimaryFont", FontData::from_static(bytes).tweak(tweak)));
    }
    if let Some(bytes) = prefs.font.files().iter().find_map(|f| windows_font(f)) {
        primary.push(("SystemPrimaryFont", FontData::from_owned(bytes).tweak(tweak)));
    }
    for (i, (name, data)) in primary.into_iter().enumerate() {
        fonts.font_data.insert(name.to_owned(), data);
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(i, name.to_owned());
    }

    // Monospace (log excerpts, package IDs): Consolas is designed for screens.
    if let Some(bytes) = windows_font("consola.ttf") {
        fonts.font_data.insert(
            "SystemMono".to_owned(),
            FontData::from_owned(bytes).tweak(FontTweak {
                scale,
                ..Default::default()
            }),
        );
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "SystemMono".to_owned());
    }

    // CJK (Chinese, Japanese, Korean) fallback — loaded after the primary font
    for (file, name) in [
        ("msyh.ttc", "CJK_Chinese"),
        ("meiryo.ttc", "CJK_Japanese"),
        ("malgun.ttf", "CJK_Korean"),
    ] {
        if let Some(bytes) = windows_font(file) {
            fonts.font_data.insert(name.to_owned(), FontData::from_owned(bytes));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .push(name.to_owned());
        }
    }

    // Emoji / symbols fallback
    for file in ["seguiemj.ttf", "seguisym.ttf"] {
        if let Some(bytes) = windows_font(file) {
            fonts
                .font_data
                .insert("SystemEmojiFont".to_owned(), FontData::from_owned(bytes));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .push("SystemEmojiFont".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);

    // Base sizes for widgets that don't set their own (labels, buttons, inputs).
    ctx.style_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::proportional(18.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
    });
}
