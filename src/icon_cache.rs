use eframe::egui;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Дисковый кэш превью модов (128x128 PNG).
/// Хранит сжатые миниатюры в локальной директории кэша (%LOCALAPPDATA%/RimWorldModManager/cache/thumbnails/),
/// чтобы при повторных запусках мгновенно загружать миниатюры без чтения и масштабирования
/// тяжелых оригиналов из Steam Workshop.
pub struct IconDiskCache {
    cache_dir: PathBuf,
}

impl IconDiskCache {
    /// Создать кэш в указанном каталоге. Каталог создается при необходимости.
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&cache_dir);
        Self { cache_dir }
    }

    /// Расположение кэша по умолчанию:
    /// 1. `%LOCALAPPDATA%/RimWorldModManager/cache/thumbnails/`
    /// 2. `<exe_dir>/cache/thumbnails/` (если переменная окружения недоступна)
    pub fn default_cache_dir() -> PathBuf {
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let path = PathBuf::from(local_appdata)
                .join("RimWorldModManager")
                .join("cache")
                .join("thumbnails");
            return path;
        }

        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("cache").join("thumbnails");
            }
        }

        PathBuf::from(".cache").join("thumbnails")
    }

    /// Генерирует стабильный ключ (имя файла) для исходного пути к превью.
    /// Использует хеш полного каноничного пути, чтобы исключить коллизии и проблемы со спецсимволами.
    pub fn cache_key(source_path: &Path) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        source_path.hash(&mut hasher);
        format!("{:016x}.png", hasher.finish())
    }

    /// Полный путь к кэшированному файлу.
    fn cached_path(&self, source_path: &Path) -> PathBuf {
        self.cache_dir.join(Self::cache_key(source_path))
    }

    /// Загрузить кэшированную миниатюру.
    /// Возвращает `Some(ColorImage)`, если кэш актуален (не старше исходного файла).
    pub fn load(&self, source_path: &Path) -> Option<egui::ColorImage> {
        let cached = self.cached_path(source_path);
        if !cached.exists() {
            return None;
        }

        // Проверяем актуальность: кэш не должен быть старше исходника
        let source_modified = fs::metadata(source_path)
            .and_then(|m| m.modified())
            .ok();
        let cache_modified = fs::metadata(&cached)
            .and_then(|m| m.modified())
            .ok();

        if let (Some(src_time), Some(cache_time)) = (source_modified, cache_modified) {
            if src_time > cache_time {
                // Исходник обновился - кэш устарел, удаляем
                let _ = fs::remove_file(&cached);
                return None;
            }
        }

        // Загружаем компактный кэшированный PNG
        match image::open(&cached) {
            Ok(img) => {
                let size = [img.width() as _, img.height() as _];
                let rgba = img.to_rgba8();
                let pixels = rgba.as_flat_samples();
                Some(egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    pixels.as_slice(),
                ))
            }
            Err(_) => {
                // Файл поврежден - удаляем
                let _ = fs::remove_file(&cached);
                None
            }
        }
    }

    /// Сохранить миниатюру в дисковый кэш.
    pub fn save(&self, source_path: &Path, rgba_data: &[u8], width: u32, height: u32) {
        let cached = self.cached_path(source_path);
        let _ = fs::create_dir_all(&self.cache_dir);

        if let Some(buf) = image::RgbaImage::from_raw(width, height, rgba_data.to_vec()) {
            let _ = buf.save_with_format(&cached, image::ImageFormat::Png);
        }
    }

    /// Очистка записей кэша старше `max_age` (например, 30 дней).
    pub fn cleanup_expired(&self, max_age: Duration) {
        let entries = match fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let now = SystemTime::now();
        let mut removed = 0usize;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            if let Ok(meta) = fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > max_age {
                            let _ = fs::remove_file(&path);
                            removed += 1;
                        }
                    }
                }
            }
        }

        if removed > 0 {
            eprintln!("[icon_cache] Очистка: удалено {} устаревших записей по таймеру", removed);
        }
    }

    /// Сборщик мусора (GC): удаляет из папки кэша миниатюры, которых больше нет
    /// среди известных путей превью установленных модов.
    pub fn cleanup_unreferenced(&self, known_preview_paths: &HashSet<PathBuf>) {
        let entries = match fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Множество ожидаемых имен файлов
        let expected_filenames: HashSet<String> = known_preview_paths
            .iter()
            .map(|p| Self::cache_key(p))
            .collect();

        let mut removed = 0usize;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                if !expected_filenames.contains(filename) {
                    let _ = fs::remove_file(&path);
                    removed += 1;
                }
            }
        }

        if removed > 0 {
            eprintln!("[icon_cache] Сборщик мусора: удалено {} неиспользуемых файлов кэша", removed);
        }
    }
}

/// Загружает миниатюру: сначала проверяет быстрый дисковый кэш, а при промахе
/// читает исходник, масштабирует его до `target_size` и сохраняет в дисковый кэш.
pub fn load_or_cache_image(
    disk_cache: &IconDiskCache,
    source_path: &Path,
    target_size: u32,
) -> Option<egui::ColorImage> {
    // 1. Попытка загрузить готовый мини-файл из дискового кэша
    if let Some(img) = disk_cache.load(source_path) {
        return Some(img);
    }

    // 2. Промах кэша: читаем оригинал с диска
    let img = image::open(source_path).ok()?;
    let img = img.resize_exact(target_size, target_size, image::imageops::FilterType::Lanczos3);
    let size = [img.width() as _, img.height() as _];
    let rgba = img.to_rgba8();

    // 3. Сохраняем уменьшенную копию в дисковый кэш
    disk_cache.save(source_path, rgba.as_raw(), img.width(), img.height());

    let pixels = rgba.as_flat_samples();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        pixels.as_slice(),
    ))
}
