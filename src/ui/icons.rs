use eframe::egui::{self, Color32, Image, ImageSource, Vec2};

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

pub fn svg_icon(source: ImageSource<'static>, size: Vec2, tint: Color32) -> Image<'static> {
    Image::new(source)
        .fit_to_exact_size(size)
        .tint(tint)
}
