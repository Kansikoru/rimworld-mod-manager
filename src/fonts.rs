use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::path::PathBuf;

pub fn setup_custom_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = FontDefinitions::default();

    // On Windows, load Segoe UI / Segoe UI Variable for pristine native rendering with complete Cyrillic support
    let font_candidates = [
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\segoeuil.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\tahoma.ttf",
    ];

    for path_str in font_candidates {
        let path = PathBuf::from(path_str);
        if path.exists() {
            if let Ok(font_bytes) = std::fs::read(&path) {
                fonts.font_data.insert(
                    "SystemPrimaryFont".to_owned(),
                    FontData::from_owned(font_bytes),
                );

                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .insert(0, "SystemPrimaryFont".to_owned());

                break;
            }
        }
    }

    // CJK (Chinese, Japanese, Korean) font fallback — loaded after the primary font
    let cjk_candidates: [(&str, &str); 5] = [
        ("C:\\Windows\\Fonts\\msyh.ttc", "CJK_Chinese"),       // Microsoft YaHei (Simplified Chinese)
        ("C:\\Windows\\Fonts\\msyhbd.ttc", "CJK_ChineseBold"),  // Microsoft YaHei Bold
        ("C:\\Windows\\Fonts\\meiryo.ttc", "CJK_Japanese"),     // Meiryo (Japanese)
        ("C:\\Windows\\Fonts\\yugothm.ttc", "CJK_Japanese2"),   // Yu Gothic Medium (Japanese)
        ("C:\\Windows\\Fonts\\malgun.ttf", "CJK_Korean"),       // Malgun Gothic (Korean)
    ];
    for (path_str, font_name) in cjk_candidates {
        let path = PathBuf::from(path_str);
        if path.exists() {
            if let Ok(font_bytes) = std::fs::read(&path) {
                fonts.font_data.insert(
                    font_name.to_owned(),
                    FontData::from_owned(font_bytes),
                );

                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(font_name.to_owned());
            }
        }
    }

    // Emoji / symbols font fallback if present
    let emoji_candidates = [
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        "C:\\Windows\\Fonts\\seguisym.ttf",
    ];
    for path_str in emoji_candidates {
        let path = PathBuf::from(path_str);
        if path.exists() {
            if let Ok(font_bytes) = std::fs::read(&path) {
                fonts.font_data.insert(
                    "SystemEmojiFont".to_owned(),
                    FontData::from_owned(font_bytes),
                );

                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push("SystemEmojiFont".to_owned());
                break;
            }
        }
    }

    ctx.set_fonts(fonts);
}
