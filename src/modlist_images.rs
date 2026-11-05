//! Pictures of mod sets ("Modpacks" tab): the user's own picture, kept as a square PNG in
//! the program's data folder (not in the game's ModLists folder), or a placeholder drawn
//! from the set's name, the same for the same name.

use std::path::{Path, PathBuf};

use image::{imageops::FilterType, Rgba, RgbaImage};

/// Side of a stored picture, in pixels.
const SIDE: u32 = 256;
/// Side of a generated placeholder, in pixels.
const PLACEHOLDER_SIDE: u32 = 128;

/// Card background the placeholder colors are mixed into (`THEME.bg_card`).
const BASE: [u8; 3] = [0x1B, 0x15, 0x30];
/// Accent colors of the theme (lavender, violet, mint, rose, amber, sky).
const PALETTE: [[u8; 3]; 6] = [
    [0xB6, 0x9C, 0xFF],
    [0x7C, 0x5C, 0xE0],
    [0x6E, 0xE7, 0xB7],
    [0xF4, 0x7F, 0xA0],
    [0xFB, 0xBF, 0x77],
    [0x8E, 0xC5, 0xFF],
];

pub fn images_dir() -> PathBuf {
    crate::game_install::manager_data_dir().join("modlist_images")
}

/// Where the picture of a set lives. Set names are file names already (`check_name`).
pub fn image_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.png"))
}

/// Makes `source` the picture of a set: cut to a centered square and scaled to 256 px.
pub fn set_image(dir: &Path, name: &str, source: &Path) -> Result<(), String> {
    let img = image::open(source).map_err(|e| e.to_string())?;
    let side = img.width().min(img.height());
    if side == 0 {
        return Err("empty image".into());
    }
    let x = (img.width() - side) / 2;
    let y = (img.height() - side) / 2;
    let square = img.crop_imm(x, y, side, side);
    let target = SIDE.min(side);
    let square = square.resize_exact(target, target, FilterType::Lanczos3).to_rgba8();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    square.save(image_path(dir, name)).map_err(|e| e.to_string())
}

/// The set's own picture, if it has one.
pub fn load_image(dir: &Path, name: &str) -> Option<RgbaImage> {
    let path = image_path(dir, name);
    if !path.is_file() {
        return None;
    }
    image::open(path).ok().map(|i| i.to_rgba8())
}

pub fn remove_image(dir: &Path, name: &str) {
    let _ = std::fs::remove_file(image_path(dir, name));
}

/// The picture follows a renamed set.
pub fn rename_image(dir: &Path, from: &str, to: &str) {
    let source = image_path(dir, from);
    if source.is_file() {
        let _ = std::fs::rename(source, image_path(dir, to));
    }
}

/// A copied set gets the same picture.
pub fn copy_image(dir: &Path, from: &str, to: &str) {
    let source = image_path(dir, from);
    if source.is_file() {
        let _ = std::fs::copy(source, image_path(dir, to));
    }
}

/// FNV-1a: a steady number per name (the std hasher may change between Rust versions).
fn name_hash(name: &str) -> u64 {
    name.to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The two accent colors of a set's placeholder; the first one also colors its icon.
pub fn placeholder_colors(name: &str) -> ([u8; 3], [u8; 3]) {
    let h = name_hash(name);
    let n = PALETTE.len() as u64;
    let a = (h % n) as usize;
    let b = ((h % n + 1 + (h >> 16) % (n - 1)) % n) as usize;
    (PALETTE[a], PALETTE[b])
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] as f32 + (b[i] as f32 - a[i] as f32) * t)
}

/// A placeholder for a set without a picture: a diagonal gradient of two theme colors with
/// a few soft circles, chosen by the name. The view draws the modpack icon on top.
pub fn placeholder(name: &str) -> RgbaImage {
    let (a, b) = placeholder_colors(name);
    let mut seed = name_hash(name) | 1;
    let mut next = move || {
        // xorshift64
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32
    };
    let side = PLACEHOLDER_SIDE as f32;
    let circles: Vec<(f32, f32, f32, [u8; 3])> = (0..3)
        .map(|i| {
            let color = if i % 2 == 0 { a } else { b };
            (next() * side, next() * side, side * (0.18 + next() * 0.28), color)
        })
        .collect();

    RgbaImage::from_fn(PLACEHOLDER_SIDE, PLACEHOLDER_SIDE, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let t = (fx + fy) / (2.0 * side);
        let tone = mix(a, b, t).map(|c| c.round() as u8);
        let mut px = mix(BASE, tone, 0.38);
        for &(cx, cy, r, color) in &circles {
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            // Soft edge 6 px wide.
            let inside = ((r - d) / 6.0).clamp(0.0, 1.0);
            if inside > 0.0 {
                let light = mix(BASE, color, 0.65);
                for i in 0..3 {
                    px[i] += (light[i] - px[i]) * 0.42 * inside;
                }
            }
        }
        Rgba([px[0].round() as u8, px[1].round() as u8, px[2].round() as u8, 255])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_is_steady_and_differs_by_name() {
        let one = placeholder("Main run");
        assert_eq!(one, placeholder("Main run"), "same name, same picture");
        assert_eq!(one, placeholder("MAIN RUN"), "letter case does not matter");
        assert_ne!(one, placeholder("Medieval"));
        assert_eq!(one.dimensions(), (PLACEHOLDER_SIDE, PLACEHOLDER_SIDE));
        let (a, b) = placeholder_colors("Main run");
        assert_ne!(a, b);
    }

    #[test]
    fn picture_is_cut_to_a_square_and_follows_the_set() {
        let dir = std::env::temp_dir().join(format!("rwmm_set_pics_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("wide.png");
        RgbaImage::from_pixel(600, 300, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();

        set_image(&dir, "Run", &source).unwrap();
        let pic = load_image(&dir, "Run").unwrap();
        assert_eq!(pic.dimensions(), (SIDE, SIDE));

        copy_image(&dir, "Run", "Run (copy)");
        rename_image(&dir, "Run", "Second");
        assert!(load_image(&dir, "Run").is_none());
        assert!(load_image(&dir, "Second").is_some());
        assert!(load_image(&dir, "Run (copy)").is_some());
        remove_image(&dir, "Second");
        assert!(load_image(&dir, "Second").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
