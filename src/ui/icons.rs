use eframe::egui::{self, Color32, Image, ImageSource, Vec2};

use crate::categories::ModCategory;

// White line icons (assets/icons), tinted in code.

// Interface
pub const ICON_MODS: ImageSource<'static> = egui::include_image!("../../assets/icons/mods.svg");
pub const ICON_TRANSLATIONS: ImageSource<'static> = egui::include_image!("../../assets/icons/translations.svg");
pub const ICON_OPTIMIZER: ImageSource<'static> = egui::include_image!("../../assets/icons/optimizer.svg");
pub const ICON_ANALYZER: ImageSource<'static> = egui::include_image!("../../assets/icons/analyzer.svg");
pub const ICON_SETTINGS: ImageSource<'static> = egui::include_image!("../../assets/icons/settings.svg");
pub const ICON_PLAY: ImageSource<'static> = egui::include_image!("../../assets/icons/play.svg");
pub const ICON_SORT: ImageSource<'static> = egui::include_image!("../../assets/icons/sort.svg");
pub const ICON_SAVE: ImageSource<'static> = egui::include_image!("../../assets/icons/save.svg");
pub const ICON_REFRESH: ImageSource<'static> = egui::include_image!("../../assets/icons/refresh.svg");
pub const ICON_CHECK_ALL: ImageSource<'static> = egui::include_image!("../../assets/icons/check_all.svg");
pub const ICON_X_ALL: ImageSource<'static> = egui::include_image!("../../assets/icons/x_all.svg");
pub const ICON_SEARCH: ImageSource<'static> = egui::include_image!("../../assets/icons/search.svg");
pub const ICON_ARROW_UP: ImageSource<'static> = egui::include_image!("../../assets/icons/arrow_up.svg");
pub const ICON_ARROW_DOWN: ImageSource<'static> = egui::include_image!("../../assets/icons/arrow_down.svg");
pub const ICON_CLOSE: ImageSource<'static> = egui::include_image!("../../assets/icons/close.svg");
pub const ICON_PLUS: ImageSource<'static> = egui::include_image!("../../assets/icons/plus.svg");
pub const ICON_FOLDER: ImageSource<'static> = egui::include_image!("../../assets/icons/folder.svg");
pub const ICON_STEAM: ImageSource<'static> = egui::include_image!("../../assets/icons/steam.svg");
pub const ICON_INFO: ImageSource<'static> = egui::include_image!("../../assets/icons/info.svg");
pub const ICON_SHIELD: ImageSource<'static> = egui::include_image!("../../assets/icons/shield.svg");
pub const ICON_ROCKET: ImageSource<'static> = egui::include_image!("../../assets/icons/rocket.svg");
pub const ICON_UNDO: ImageSource<'static> = egui::include_image!("../../assets/icons/undo.svg");
pub const ICON_STOP: ImageSource<'static> = egui::include_image!("../../assets/icons/stop.svg");
pub const ICON_COPY: ImageSource<'static> = egui::include_image!("../../assets/icons/copy.svg");
pub const ICON_TRASH: ImageSource<'static> = egui::include_image!("../../assets/icons/trash.svg");

// Status
pub const ICON_STATUS_WARNING: ImageSource<'static> = egui::include_image!("../../assets/icons/status_warning.svg");
pub const ICON_STATUS_ERROR: ImageSource<'static> = egui::include_image!("../../assets/icons/status_error.svg");
pub const ICON_STATUS_OK: ImageSource<'static> = egui::include_image!("../../assets/icons/status_ok.svg");
pub const ICON_STATUS_MISSING: ImageSource<'static> = egui::include_image!("../../assets/icons/status_missing.svg");
pub const ICON_STATUS_NOT_ON_DISK: ImageSource<'static> = egui::include_image!("../../assets/icons/status_not_on_disk.svg");
pub const ICON_STATUS_DUPLICATE: ImageSource<'static> = egui::include_image!("../../assets/icons/status_duplicate.svg");
pub const ICON_STATUS_CRASH: ImageSource<'static> = egui::include_image!("../../assets/icons/status_crash.svg");
pub const ICON_STATUS_LOCAL: ImageSource<'static> = egui::include_image!("../../assets/icons/status_local.svg");
pub const ICON_STATUS_WORKSHOP: ImageSource<'static> = egui::include_image!("../../assets/icons/status_workshop.svg");

// Brand (logo.svg is full colour: draw it without tint)
pub const ICON_LOGO: ImageSource<'static> = egui::include_image!("../../assets/icons/logo.svg");

// Mod categories
pub const ICON_CAT_FRAMEWORK: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_framework.svg");
pub const ICON_CAT_QOL: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_qol.svg");
pub const ICON_CAT_RACE: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_race.svg");
pub const ICON_CAT_FACTION: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_faction.svg");
pub const ICON_CAT_BUILDING: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_building.svg");
pub const ICON_CAT_WEAPON: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_weapon.svg");
pub const ICON_CAT_APPAREL: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_apparel.svg");
pub const ICON_CAT_ANIMAL: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_animal.svg");
pub const ICON_CAT_BIOME: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_biome.svg");
pub const ICON_CAT_STORYTELLER: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_storyteller.svg");
pub const ICON_CAT_PATCH: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_patch.svg");
pub const ICON_CAT_RETEXTURE: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_retexture.svg");
pub const ICON_CAT_TRANSLATION: ImageSource<'static> = egui::include_image!("../../assets/icons/cat_translation.svg");

pub fn category_icon(cat: ModCategory) -> ImageSource<'static> {
    match cat {
        ModCategory::Framework => ICON_CAT_FRAMEWORK,
        ModCategory::QoL => ICON_CAT_QOL,
        ModCategory::Race => ICON_CAT_RACE,
        ModCategory::Faction => ICON_CAT_FACTION,
        ModCategory::Building => ICON_CAT_BUILDING,
        ModCategory::Weapon => ICON_CAT_WEAPON,
        ModCategory::Apparel => ICON_CAT_APPAREL,
        ModCategory::Animal => ICON_CAT_ANIMAL,
        ModCategory::Biome => ICON_CAT_BIOME,
        ModCategory::Storyteller => ICON_CAT_STORYTELLER,
        ModCategory::Patch => ICON_CAT_PATCH,
        ModCategory::Retexture => ICON_CAT_RETEXTURE,
        ModCategory::Translation => ICON_CAT_TRANSLATION,
    }
}

/// An SVG icon rasterized at exactly `size` and drawn 1:1 without smoothing,
/// so thin lines stay sharp instead of being blurred by texture filtering.
/// Sizes are rounded to whole pixels; 16 px or more keeps the line icons legible.
pub fn svg_icon(source: ImageSource<'static>, size: Vec2, tint: Color32) -> Image<'static> {
    Image::new(source)
        .fit_to_exact_size(size.round())
        .texture_options(egui::TextureOptions::NEAREST)
        .tint(tint)
}
