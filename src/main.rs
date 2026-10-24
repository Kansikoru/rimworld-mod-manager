#![windows_subsystem = "windows"]

use eframe::egui;
use rimworld_mod_manager::app::App;
use rimworld_mod_manager::fonts::setup_custom_fonts;

fn load_app_icon() -> Option<egui::IconData> {
    let icon_bytes = include_bytes!("../assets/icon.png");
    if let Ok(img) = image::load_from_memory_with_format(icon_bytes, image::ImageFormat::Png) {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Some(egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        })
    } else {
        None
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!(
            "CRITICAL PANIC: {}\nLocation: {:?}\n",
            info,
            info.location()
        );
        let _ = std::fs::write("crash_log.txt", &msg);
        eprintln!("{}", msg);
    }));

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 840.0])
        .with_min_inner_size([980.0, 640.0])
        .with_title("RimWorld Mod & Load Order Manager");

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        vsync: true,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        ..Default::default()
    };

    let result = eframe::run_native(
        "RimWorld Mod & Load Order Manager",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(App::default()))
        }),
    );

    if let Err(e) = result {
        eprintln!("[ОШИБКА] Сбой при работе интерфейса: {:?}", e);
    }
}

