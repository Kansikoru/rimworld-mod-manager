use eframe::egui;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Disk cache of mod preview thumbnails (small square PNGs) in
/// `%LOCALAPPDATA%\RimWorldModManager\cache\thumbnails`, so later runs don't
/// decode and scale the full-size Workshop previews again.
pub struct IconDiskCache {
    cache_dir: PathBuf,
}

/// Bumped when the thumbnail format changes, so old files are not reused.
const CACHE_VERSION: &str = "v2";

impl IconDiskCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&cache_dir);
        Self { cache_dir }
    }

    /// `%LOCALAPPDATA%\RimWorldModManager\cache\thumbnails`, or `<exe dir>\cache\thumbnails`.
    pub fn default_cache_dir() -> PathBuf {
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local_appdata)
                .join("RimWorldModManager")
                .join("cache")
                .join("thumbnails");
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("cache").join("thumbnails");
            }
        }
        PathBuf::from(".cache").join("thumbnails")
    }

    /// File name for a preview path. FNV-1a over the path text: unlike `DefaultHasher`,
    /// it gives the same name in every build, so the cache survives updates.
    pub fn cache_key(source_path: &Path) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in source_path.to_string_lossy().to_lowercase().bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{CACHE_VERSION}-{hash:016x}.png")
    }

    fn cached_path(&self, source_path: &Path) -> PathBuf {
        self.cache_dir.join(Self::cache_key(source_path))
    }

    /// Cached thumbnail, if it exists and is not older than the source image.
    pub fn load(&self, source_path: &Path) -> Option<egui::ColorImage> {
        let cached = self.cached_path(source_path);
        let cache_time = fs::metadata(&cached).and_then(|m| m.modified()).ok()?;
        let source_time = fs::metadata(source_path).and_then(|m| m.modified()).ok();
        if source_time.is_some_and(|src| src > cache_time) {
            let _ = fs::remove_file(&cached);
            return None;
        }

        match image::open(&cached) {
            Ok(img) => Some(to_color_image(&img.to_rgba8())),
            Err(_) => {
                let _ = fs::remove_file(&cached);
                None
            }
        }
    }

    pub fn save(&self, source_path: &Path, thumb: &image::RgbaImage) {
        let _ = fs::create_dir_all(&self.cache_dir);
        let _ = thumb.save_with_format(self.cached_path(source_path), image::ImageFormat::Png);
    }

    /// Deletes thumbnails whose preview image is no longer used by any installed mod,
    /// plus files left by older cache versions. Returns how many files were removed.
    pub fn cleanup_unreferenced(&self, known_preview_paths: &HashSet<PathBuf>) -> usize {
        let Ok(entries) = fs::read_dir(&self.cache_dir) else {
            return 0;
        };
        let expected: HashSet<String> = known_preview_paths.iter().map(|p| Self::cache_key(p)).collect();

        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            let name = path.file_name().and_then(|f| f.to_str()).unwrap_or_default();
            if !expected.contains(name) && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

fn to_color_image(rgba: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )
}

/// Square thumbnail without distortion: scaled to cover `size`×`size`, then the
/// centre is cropped (previews are usually 16:9, the icon slots are square).
pub fn make_thumbnail(img: &image::DynamicImage, size: u32) -> image::RgbaImage {
    img.resize_to_fill(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8()
}

/// Thumbnail from the disk cache, or made from the source image and cached.
pub fn load_or_cache_image(
    disk_cache: &IconDiskCache,
    source_path: &Path,
    target_size: u32,
) -> Option<egui::ColorImage> {
    if let Some(img) = disk_cache.load(source_path) {
        return Some(img);
    }
    let img = image::open(source_path).ok()?;
    let thumb = make_thumbnail(&img, target_size);
    disk_cache.save(source_path, &thumb);
    Some(to_color_image(&thumb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_case_insensitive() {
        let a = IconDiskCache::cache_key(Path::new(r"C:\Mods\Foo\About\Preview.png"));
        let b = IconDiskCache::cache_key(Path::new(r"c:\mods\foo\about\preview.png"));
        assert_eq!(a, b);
        // Fixed value: the key must not change between builds or Rust versions.
        assert_eq!(a, "v2-82499346215f0d51.png");
        assert!(a.starts_with("v2-") && a.ends_with(".png"));
        assert_ne!(a, IconDiskCache::cache_key(Path::new(r"C:\Mods\Bar\About\Preview.png")));
    }

    #[test]
    fn thumbnail_keeps_proportions() {
        // 16:9 picture: left half red, right half blue. A centre crop keeps both colours
        // undistorted and gives a square.
        let mut wide = image::RgbaImage::new(160, 90);
        for (x, _, p) in wide.enumerate_pixels_mut() {
            *p = if x < 80 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) };
        }
        let thumb = make_thumbnail(&image::DynamicImage::ImageRgba8(wide), 64);
        assert_eq!(thumb.dimensions(), (64, 64));
        assert_eq!(thumb.get_pixel(2, 32)[0], 255);
        assert_eq!(thumb.get_pixel(61, 32)[2], 255);
    }
}
