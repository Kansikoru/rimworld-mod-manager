fn main() {
    // Rebuild the resource when the icon changes (or the build moves to another folder).
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // Without the resource the exe has a blank icon in Explorer and the taskbar: say why
        // instead of keeping quiet (the build still goes on).
        if let Err(e) = res.compile() {
            println!("cargo:warning=the exe icon wasn't embedded: {e}");
        }
    }
}
