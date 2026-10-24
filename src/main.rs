#![windows_subsystem = "windows"]

use eframe::egui;
use rimworld_mod_manager::app::App;
use rimworld_mod_manager::fonts::setup_custom_fonts;
use rimworld_mod_manager::ui::theme::apply_dark_theme;

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
    // Panics are logged to %LOCALAPPDATA%\RimWorldModManager\crash_log.txt (appended),
    // not to whatever the current directory happens to be.
    let crash_log = rimworld_mod_manager::game_install::manager_data_dir().join("crash_log.txt");
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let msg = format!(
            "[unix {}] PANIC in thread '{}': {}\n",
            secs,
            thread.name().unwrap_or("?"),
            info
        );
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&crash_log) {
            let _ = std::io::Write::write_all(&mut f, msg.as_bytes());
        }
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
            // Always dark, whatever the Windows theme; styles are set once here, not every frame.
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            apply_dark_theme(&cc.egui_ctx);
            setup_custom_fonts(&cc.egui_ctx, &rimworld_mod_manager::ui_prefs::UiPrefs::load());
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    );

    if let Err(e) = result {
        eprintln!("UI error: {:?}", e);
    }
}

