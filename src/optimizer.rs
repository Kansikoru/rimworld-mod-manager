use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::SystemTime;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::i18n::tr;
use crate::models::ModInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptimizeScope {
    #[default]
    AllInstalled,
    ActiveOnly,
    SelectedOnly,
}

/// Texture dimension ceiling (V1). Textures exceeding this dimension along width or height
/// are downscaled preserving aspect ratio and rounded to multiples of 4 for block compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MaxTextureSize {
    #[default]
    #[serde(rename = "none")]
    NoLimit,
    #[serde(rename = "2048")]
    S2048,
    #[serde(rename = "1024")]
    S1024,
    #[serde(rename = "512")]
    S512,
}

impl MaxTextureSize {
    pub fn pixels(self) -> Option<u32> {
        match self {
            Self::NoLimit => None,
            Self::S2048 => Some(2048),
            Self::S1024 => Some(1024),
            Self::S512 => Some(512),
        }
    }

    pub fn from_pixels(px: Option<u32>) -> Self {
        match px {
            Some(2048) => Self::S2048,
            Some(1024) => Self::S1024,
            Some(512) => Self::S512,
            _ => Self::NoLimit,
        }
    }
}

// Messages
/// Result of a finished conversion run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationSummary {
    pub bc7: usize,
    pub bc1: usize,
    /// Left as PNG: side not a multiple of 4, or too small.
    pub skipped: usize,
    pub errors: usize,
    /// Sources moved to the backup folder.
    pub sources_backed_up: usize,
    /// DDS files not made by this tool (author's or left by an old version) — untouched.
    pub foreign_dds: usize,
    /// Our DDS that were already up to date.
    pub up_to_date: usize,
}

/// Messages sent from the optimizer worker thread to the UI.
pub enum OptimizationMsg {
    /// Progress update: (done, total, skipped, current_file)
    Progress {
        done: usize,
        total: usize,
        skipped: usize,
        current_file: String,
    },
    /// Single file error (non-fatal, processing continues)
    FileError(String),
    /// Optimization finished
    Done(OptimizationSummary),
    /// Optimization was cancelled by user
    Cancelled,
}

// Texture Scanning
/// Check whether a path contains a directory named "textures" (case-insensitive)
pub fn is_in_textures_folder(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.eq_ignore_ascii_case("textures"))
            .unwrap_or(false)
    })
}

/// Check whether a path is inside an About/ folder.
pub fn is_in_about_folder(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.eq_ignore_ascii_case("about"))
            .unwrap_or(false)
    })
}

/// Check whether a path is inside a dangerous folder (UI, GUI, Icons, Defs, Pawn).
pub fn is_in_dangerous_folder(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| {
                let lower = s.to_ascii_lowercase();
                matches!(
                    lower.as_str(),
                    "ui" | "gui" | "icons" | "defs" | "pawn" | "pawns" | "hair" | "heads" | "bodies" | "apparel"
                )
            })
            .unwrap_or(false)
    })
}

/// Walk a mod directory and collect all candidate texture files (PNG/JPG inside any Textures folder).
/// Skips non-texture directories for high performance.
fn walk_mod_textures(mod_dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(mod_dir)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_dir() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name != "assemblies"
                    && name != "source"
                    && name != ".git"
                    && name != "languages"
                    && name != "defs"
                    && name != "patches"
                    && name != "about"
            } else {
                true
            }
        })
        .filter_map(|e| e.ok())
        .filter(|entry| {
            if !entry.file_type().is_file() {
                return false;
            }
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if ext != "png" && ext != "jpg" && ext != "jpeg" {
                return false;
            }
            is_in_textures_folder(path) && !is_in_about_folder(path) && !is_in_dangerous_folder(path)
        })
        .map(|entry| entry.into_path())
}

// Single File Conversion
/// Options for converting one texture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConvertOptions {
    /// Store the picture flipped vertically: needed on RimWorld 1.7, which draws DDS upside
    /// down; not on 1.6 (like ToDDS). Chosen from the game version, see `engine_wants_flipped`.
    pub flip_vertical: bool,
    /// Maximum texture side (width or height). Larger textures are downscaled preserving
    /// aspect ratio (and rounded to multiples of 4 for block compression).
    pub max_size: Option<u32>,
}

/// Textures smaller than this on both sides are left as PNG: no gain, only risk.
pub const MIN_TEXTURE_SIDE: u32 = 64;

/// Compute target dimensions for a texture with maximum side limit,
/// preserving aspect ratio and rounding both sides to multiples of 4 (required by BC compression).
pub fn target_dimensions(w: u32, h: u32, max_side: Option<u32>) -> (u32, u32) {
    let Some(max_s) = max_side else {
        return (w, h);
    };
    if w <= max_s && h <= max_s {
        return (w, h);
    }
    // Scale factor so that max(w, h) fits within max_s
    let scale = (max_s as f64) / (w.max(h) as f64);
    let new_w = (w as f64 * scale).round() as u32;
    let new_h = (h as f64 * scale).round() as u32;

    // Round to nearest multiple of 4, minimum 4, never exceeding max_s
    let round_m4 = |v: u32| -> u32 {
        let rem = v % 4;
        let rounded = if rem < 2 { v - rem } else { v + (4 - rem) };
        rounded.clamp(4, max_s)
    };

    (round_m4(new_w), round_m4(new_h))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertOutcome {
    /// BC7 (texture has transparency) with mipmaps.
    Bc7,
    /// BC1 (fully opaque, half the size of BC7) with mipmaps.
    Bc1,
    /// Not converted: a side is not a multiple of 4 (block compression needs 4x4 blocks).
    /// The texture is never stretched — it stays PNG.
    SkippedSize,
    /// Not converted: smaller than MIN_TEXTURE_SIDE on both sides.
    SkippedSmall,
}

/// Picks the DDS format: BC7 when any pixel is transparent, BC1 otherwise.
pub fn choose_format(rgba: &image::RgbaImage) -> image_dds::ImageFormat {
    if rgba.pixels().any(|p| p.0[3] < 255) {
        image_dds::ImageFormat::BC7RgbaUnorm
    } else {
        image_dds::ImageFormat::BC1RgbaUnorm
    }
}

/// BC1 with a DX10 extended header -> classic "DXT1" header that every loader understands.
/// BC7 has no classic FourCC, so its DX10 header stays.
fn legacy_dxt1_header(mut dds_bytes: Vec<u8>) -> Vec<u8> {
    if dds_bytes.len() >= 148 && &dds_bytes[84..88] == b"DX10" {
        dds_bytes[84..88].copy_from_slice(b"DXT1");
        let mut fixed = Vec::with_capacity(dds_bytes.len() - 20);
        fixed.extend_from_slice(&dds_bytes[0..128]);
        fixed.extend_from_slice(&dds_bytes[148..]);
        return fixed;
    }
    dds_bytes
}

/// Converts one PNG/JPG to DDS next to it, the way ToDDS does it:
/// BC7 (with alpha) or BC1 (opaque), full mipmap chain, original size, no flip unless asked.
pub fn optimize_single_texture(source: &Path, opts: ConvertOptions) -> Result<ConvertOutcome, String> {
    encode_texture(source, &source.with_extension("dds"), opts)
}

/// Encodes `source` into `dds_path` (which may live elsewhere, e.g. when the source was moved
/// to the backup folder).
pub fn encode_texture(source: &Path, dds_path: &Path, opts: ConvertOptions) -> Result<ConvertOutcome, String> {
    let mut img = image::open(source).map_err(|e| format!("{}: {}", source.display(), e))?;

    let (mut w, mut h) = (img.width(), img.height());
    if w < MIN_TEXTURE_SIDE && h < MIN_TEXTURE_SIDE {
        return Ok(ConvertOutcome::SkippedSmall);
    }

    let (target_w, target_h) = target_dimensions(w, h, opts.max_size);
    if target_w != w || target_h != h {
        img = img.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3);
        w = img.width();
        h = img.height();
    }

    if opts.flip_vertical {
        img = img.flipv();
    }

    if w % 4 != 0 || h % 4 != 0 {
        return Ok(ConvertOutcome::SkippedSize);
    }

    let rgba = img.to_rgba8();
    let format = choose_format(&rgba);

    let dds = image_dds::dds_from_image(
        &rgba,
        format,
        image_dds::Quality::Normal,
        image_dds::Mipmaps::GeneratedAutomatic,
    )
    .map_err(|e| format!("{}: encode error: {}", source.display(), e))?;

    let mut dds_bytes = Vec::new();
    dds.write(&mut dds_bytes)
        .map_err(|e| format!("{}: buffer write error: {}", source.display(), e))?;

    let outcome = if format == image_dds::ImageFormat::BC1RgbaUnorm {
        dds_bytes = legacy_dxt1_header(dds_bytes);
        ConvertOutcome::Bc1
    } else {
        ConvertOutcome::Bc7
    };

    // Write to a temp file and rename: a cancelled or crashed run never leaves a truncated .dds
    // (a truncated file would look "newer than the PNG" and be loaded by the game).
    let tmp_path = dds_path.with_extension("dds.tmp");
    let write_res = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        std::io::Write::write_all(&mut file, &dds_bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("{}: write error: {}", dds_path.display(), e));
    }
    std::fs::rename(&tmp_path, dds_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("{}: rename error: {}", dds_path.display(), e)
    })?;

    Ok(outcome)
}

// Manifest & source backups
/// One DDS created by the optimizer. `backup` is where the source PNG/JPG was moved, if it was.
/// `source_fp` is the source file's "size:mtime" at conversion time: when a mod update changes
/// the PNG, the fingerprint differs and the DDS is re-encoded instead of shadowing the new art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub dds: PathBuf,
    pub source: PathBuf,
    pub backup: Option<PathBuf>,
    pub source_fp: Option<String>,
    /// The picture is stored flipped vertically. `None`: written before this was logged
    /// (those were made without flipping).
    pub flipped: Option<bool>,
    /// The maximum texture dimension applied at encoding time, if any.
    pub max_size: Option<u32>,
}

impl ManifestEntry {
    /// The DDS is stored the other way up than the installed game expects.
    pub fn wrong_orientation(&self, want_flipped: bool) -> bool {
        self.flipped.unwrap_or(false) != want_flipped
    }

    /// The DDS would come out another size with the requested max_size. A texture smaller
    /// than both limits stays as it is: changing the setting does not re-encode it.
    pub fn wrong_max_size(&self, source: &Path, wanted_max_size: Option<u32>) -> bool {
        if self.max_size == wanted_max_size {
            return false;
        }
        match image::image_dimensions(source) {
            Ok((w, h)) => target_dimensions(w, h, self.max_size) != target_dimensions(w, h, wanted_max_size),
            Err(_) => true,
        }
    }
}

/// RimWorld 1.6 reads DDS the same way up as the PNG; 1.7 (Waylith) draws them upside down,
/// so on 1.7 the picture has to be stored flipped.
pub fn engine_wants_flipped(engine: crate::game_install::GameEngine) -> bool {
    engine == crate::game_install::GameEngine::Waylith
}

/// "size:mtime_secs" of a file.
pub fn source_fingerprint(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let secs = meta
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{}:{}", meta.len(), secs))
}

/// Case-insensitive key for Windows paths.
fn path_key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Manifest as a map by DDS path (the latest entry wins).
pub fn manifest_index(entries: Vec<ManifestEntry>) -> HashMap<String, ManifestEntry> {
    entries.into_iter().map(|e| (path_key(&e.dds), e)).collect()
}

/// Adds or replaces entries (by DDS path) and rewrites the manifest without duplicates.
pub fn merge_manifest(path: &Path, new_entries: &[ManifestEntry]) -> std::io::Result<()> {
    if new_entries.is_empty() {
        return Ok(());
    }
    let mut index = manifest_index(read_manifest(path));
    for e in new_entries {
        index.insert(path_key(&e.dds), e.clone());
    }
    let mut all: Vec<ManifestEntry> = index.into_values().collect();
    all.sort_by(|a, b| a.dds.cmp(&b.dds));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_manifest(path, &all)
}

// Planning
/// What a conversion run would do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TexturePlan {
    /// No DDS yet, or our DDS is stale (source changed), or "force" for our DDS.
    pub to_convert: Vec<PathBuf>,
    /// Our DDS exists and matches its source.
    pub up_to_date: usize,
    /// A DDS exists that this tool did not make (shipped by the mod author, or left by an old
    /// version before the manifest existed). Never overwritten; "Rollback" cleans old ones.
    pub foreign_dds: usize,
    /// Sources that share one DDS name (`icon.png` + `icon.jpg` -> `icon.dds`). Left alone:
    /// converting one would change which picture the game shows, and both would race for
    /// the same file.
    pub same_name: usize,
}

/// `want_flipped`: the way up the installed game needs (see `engine_wants_flipped`); our DDS
/// stored the other way up are re-encoded like stale ones.
pub fn plan_textures(
    mod_dirs: &[PathBuf],
    force: bool,
    want_flipped: bool,
    want_max_size: Option<u32>,
    manifest: &HashMap<String, ManifestEntry>,
) -> TexturePlan {
    let mut plan = TexturePlan::default();
    for mod_dir in mod_dirs {
        // Sources grouped by the DDS they would become, in the order they were found.
        let mut groups: Vec<Vec<PathBuf>> = Vec::new();
        let mut group_of: HashMap<String, usize> = HashMap::new();
        for path in walk_mod_textures(mod_dir) {
            let key = path_key(&path.with_extension("dds"));
            match group_of.get(&key) {
                Some(&i) => groups[i].push(path),
                None => {
                    group_of.insert(key, groups.len());
                    groups.push(vec![path]);
                }
            }
        }
        for group in groups {
            if group.len() > 1 {
                plan.same_name += group.len();
                continue;
            }
            let Some(path) = group.into_iter().next() else { continue };
            let dds = path.with_extension("dds");
            if !dds.exists() {
                plan.to_convert.push(path);
                continue;
            }
            match manifest.get(&path_key(&dds)) {
                Some(entry) => {
                    let fresh = entry.source_fp.is_some()
                        && entry.source_fp == source_fingerprint(&path)
                        && !entry.wrong_orientation(want_flipped)
                        && !entry.wrong_max_size(&path, want_max_size);
                    if force || !fresh {
                        plan.to_convert.push(path);
                    } else {
                        plan.up_to_date += 1;
                    }
                }
                None => plan.foreign_dds += 1,
            }
        }
    }
    plan
}

/// How many DDS files from the manifest are on disk and stored the wrong way up for the
/// installed game (e.g. made for 1.6, now played on 1.7).
pub fn count_wrong_orientation(manifest: &Path, want_flipped: bool) -> usize {
    read_manifest(manifest)
        .iter()
        .filter(|e| e.wrong_orientation(want_flipped) && e.dds.is_file())
        .count()
}

/// Which way up a DDS stores its picture compared with the source image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    AsSource,
    Flipped,
    /// The image looks the same both ways (symmetric or flat): orientation doesn't matter.
    Unclear,
}

fn mean_abs_diff(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    let mut sum = 0u64;
    for (p, q) in a.pixels().zip(b.pixels()) {
        for c in 0..3 {
            sum += (p[c] as i32 - q[c] as i32).unsigned_abs() as u64;
        }
    }
    sum as f64 / (a.width() as f64 * a.height() as f64 * 3.0).max(1.0)
}

/// Compares a decoded DDS with its source: same way up, flipped, or no telling.
pub fn compare_orientation(decoded: &image::RgbaImage, source: &image::RgbaImage) -> Orientation {
    let same = mean_abs_diff(decoded, source);
    let flipped = mean_abs_diff(decoded, &image::imageops::flip_vertical(source));
    if same * 2.0 < flipped {
        Orientation::AsSource
    } else if flipped * 2.0 < same {
        Orientation::Flipped
    } else {
        Orientation::Unclear
    }
}

/// Decodes the top mip of `dds` and compares it with `source`.
pub fn detect_orientation(dds: &Path, source: &Path) -> Result<Orientation, String> {
    let file = std::fs::File::open(dds).map_err(|e| format!("{}: {}", dds.display(), e))?;
    let dds_file = image_dds::ddsfile::Dds::read(file).map_err(|e| format!("{}: {}", dds.display(), e))?;
    let decoded = image_dds::image_from_dds(&dds_file, 0).map_err(|e| format!("{}: {}", dds.display(), e))?;
    let src = image::open(source)
        .map_err(|e| format!("{}: {}", source.display(), e))?
        .to_rgba8();
    if src.dimensions() != decoded.dimensions() {
        return Err(format!("{}: size differs from {}", dds.display(), source.display()));
    }
    Ok(compare_orientation(&decoded, &src))
}

/// Result of "Flip textures".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OrientationSummary {
    /// Re-encoded the other way up.
    pub fixed: usize,
    /// Already the right way up (only the log was updated).
    pub already_ok: usize,
    pub errors: usize,
    pub cancelled: bool,
}

pub enum OrientationMsg {
    Progress {
        done: usize,
        total: usize,
        current_file: String,
    },
    Done(OrientationSummary),
}

/// "Flip textures": for every logged DDS not known to be the right way up for the game,
/// compares it with its source and re-encodes it the right way up if needed. Background.
pub fn fix_orientation_async(
    want_flipped: bool,
    max_threads: usize,
    tx: mpsc::Sender<OrientationMsg>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let summary = fix_orientation_with_manifest(&manifest_path(), want_flipped, max_threads, &tx, &stop);
        let _ = tx.send(OrientationMsg::Done(summary));
    });
}

pub fn fix_orientation_with_manifest(
    manifest: &Path,
    want_flipped: bool,
    max_threads: usize,
    tx: &mpsc::Sender<OrientationMsg>,
    stop: &AtomicBool,
) -> OrientationSummary {
    use std::sync::atomic::AtomicUsize;
    let todo: Vec<ManifestEntry> = read_manifest(manifest)
        .into_iter()
        .filter(|e| e.flipped != Some(want_flipped) && e.dds.is_file())
        .collect();
    let total = todo.len();
    let done = AtomicUsize::new(0);
    let fixed = AtomicUsize::new(0);
    let already_ok = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);
    let updated = std::sync::Mutex::new(Vec::new());

    let work = || {
        todo.par_iter().for_each(|e| {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            // The source is next to the DDS, or in the backup folder if it was moved there.
            let source = if e.source.is_file() {
                Some(e.source.clone())
            } else {
                e.backup.clone().filter(|b| b.is_file())
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<bool, String> {
                let source = source.ok_or_else(|| format!("{}: source image not found", e.dds.display()))?;
                let is_flipped = match detect_orientation(&e.dds, &source)? {
                    Orientation::AsSource => false,
                    Orientation::Flipped => true,
                    Orientation::Unclear => want_flipped,
                };
                if is_flipped == want_flipped {
                    return Ok(false);
                }
                encode_texture(
                    &source,
                    &e.dds,
                    ConvertOptions {
                        flip_vertical: want_flipped,
                        max_size: e.max_size,
                    },
                )?;
                Ok(true)
            }));
            match result {
                Ok(Ok(changed)) => {
                    if changed { &fixed } else { &already_ok }.fetch_add(1, Ordering::Relaxed);
                    if let Ok(mut list) = updated.lock() {
                        list.push(ManifestEntry {
                            flipped: Some(want_flipped),
                            ..e.clone()
                        });
                    }
                }
                _ => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(10) || n == total {
                let current_file = e
                    .dds
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let _ = tx.send(OrientationMsg::Progress {
                    done: n,
                    total,
                    current_file,
                });
            }
        });
    };
    match rayon::ThreadPoolBuilder::new()
        .num_threads(max_threads.clamp(1, total_system_threads()))
        .build()
    {
        Ok(pool) => pool.install(work),
        Err(_) => work(),
    }

    // Record the new orientation, also after a cancel (what was done stays done).
    if let Ok(list) = updated.lock() {
        let _ = merge_manifest(manifest, &list);
    }
    OrientationSummary {
        fixed: fixed.load(Ordering::Relaxed),
        already_ok: already_ok.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
        cancelled: stop.load(Ordering::Relaxed),
    }
}

/// `%LOCALAPPDATA%\RimWorldModManager\dds_manifest.tsv`: every DDS this tool created.
pub fn manifest_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("dds_manifest.tsv")
}

pub fn read_manifest(path: &Path) -> Vec<ManifestEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let dds = parts.next()?.trim();
            let source = parts.next()?.trim();
            let backup = parts.next().map(str::trim).filter(|s| !s.is_empty());
            let source_fp = parts.next().map(str::trim).filter(|s| !s.is_empty());
            let flipped = match parts.next().map(str::trim) {
                Some("1") => Some(true),
                Some("0") => Some(false),
                _ => None,
            };
            let max_size = parts.next().map(str::trim).and_then(|s| s.parse::<u32>().ok());
            if dds.is_empty() || source.is_empty() {
                return None;
            }
            Some(ManifestEntry {
                dds: PathBuf::from(dds),
                source: PathBuf::from(source),
                backup: backup.map(PathBuf::from),
                source_fp: source_fp.map(str::to_string),
                flipped,
                max_size,
            })
        })
        .collect()
}

fn manifest_line(e: &ManifestEntry) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        e.dds.display(),
        e.source.display(),
        e.backup.as_ref().map(|b| b.display().to_string()).unwrap_or_default(),
        e.source_fp.clone().unwrap_or_default(),
        match e.flipped {
            Some(true) => "1",
            Some(false) => "0",
            None => "",
        },
        e.max_size.map(|s| s.to_string()).unwrap_or_default()
    )
}

pub fn append_manifest(path: &Path, entries: &[ManifestEntry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let text: String = entries.iter().map(manifest_line).collect();
    std::io::Write::write_all(&mut file, text.as_bytes())
}

fn write_manifest(path: &Path, entries: &[ManifestEntry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    // Through a temporary file and a rename: a crash mid-write never leaves a cut log,
    // which rollback needs to find the DDS files and the moved sources.
    let text: String = entries.iter().map(manifest_line).collect();
    let tmp = path.with_extension("tsv.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// `F:\Steam\...\tex.png` -> `<backup_root>\F\Steam\...\tex.png`
pub fn backup_path_for(backup_root: &Path, source: &Path) -> PathBuf {
    let mut out = backup_root.to_path_buf();
    for comp in source.components() {
        match comp {
            std::path::Component::Prefix(p) => {
                let s: String = p
                    .as_os_str()
                    .to_string_lossy()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect();
                out.push(s);
            }
            std::path::Component::Normal(n) => out.push(n),
            _ => {}
        }
    }
    out
}

/// Moves a file, falling back to copy + delete across drives.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

fn texture_backup_root() -> PathBuf {
    crate::game_install::manager_data_dir().join("texture_backup")
}

// Background Worker
/// Total logical CPU threads available on this system.
pub fn total_system_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

pub fn calc_threads_for_percentage(total: usize, pct: usize) -> usize {
    if total <= 1 {
        1
    } else if pct >= 100 {
        total
    } else if total == 2 {
        1
    } else {
        let threads = (total * pct) / 100;
        threads.clamp(1, total.saturating_sub(1))
    }
}

pub fn default_cpu_threads() -> usize {
    calc_threads_for_percentage(total_system_threads(), 75)
}

pub fn dirs_from_mods(mods: &[ModInfo]) -> Vec<PathBuf> {
    mods.iter().map(|m| m.dir_path.clone()).collect()
}

pub fn run_optimization(
    mod_dirs: Vec<PathBuf>,
    force: bool,
    delete_source: bool,
    opts: ConvertOptions,
    max_threads: usize,
    tx: mpsc::Sender<OptimizationMsg>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let manifest = manifest_index(read_manifest(&manifest_path()));
        let plan = plan_textures(&mod_dirs, force, opts.flip_vertical, opts.max_size, &manifest);
        let textures = plan.to_convert;
        let total = textures.len();

        if total == 0 {
            let _ = tx.send(OptimizationMsg::Done(OptimizationSummary {
                foreign_dds: plan.foreign_dds,
                up_to_date: plan.up_to_date,
                ..Default::default()
            }));
            return;
        }

        use std::sync::atomic::AtomicUsize;
        let done_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));
        let deleted_count = Arc::new(AtomicUsize::new(0));
        let bc7_count = Arc::new(AtomicUsize::new(0));
        let bc1_count = Arc::new(AtomicUsize::new(0));
        let skipped_count = Arc::new(AtomicUsize::new(0));
        let created: Arc<std::sync::Mutex<Vec<ManifestEntry>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let backup_root = texture_backup_root();
        let log_path = manifest_path();
        let log_lock = std::sync::Mutex::new(());
        let tx_clone = tx.clone();
        let stop = stop_flag.clone();

        let worker_threads = max_threads.clamp(1, total_system_threads());
        let pool_res = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_threads)
            .thread_name(|i| format!("tex-opt-{}", i))
            .start_handler(|_| {
                #[cfg(windows)]
                {
                    extern "system" {
                        fn GetCurrentThread() -> isize;
                        fn SetThreadPriority(h_thread: isize, priority: i32) -> i32;
                    }
                    unsafe {
                        SetThreadPriority(GetCurrentThread(), -1);
                    }
                }
            })
            .build();

        let process_textures = || {
            textures.par_iter().for_each(|path| {
                if stop.load(Ordering::Relaxed) {
                    return;
                }

                let current_file = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();

                let source_fp = source_fingerprint(path);
                let conv_res =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| optimize_single_texture(path, opts)));

                match conv_res {
                    Ok(Ok(ConvertOutcome::SkippedSize | ConvertOutcome::SkippedSmall)) => {
                        skipped_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Ok(outcome)) => {
                        if outcome == ConvertOutcome::Bc7 {
                            bc7_count.fetch_add(1, Ordering::Relaxed);
                        } else {
                            bc1_count.fetch_add(1, Ordering::Relaxed);
                        }
                        let dds_path = path.with_extension("dds");
                        let backup_target =
                            (delete_source && dds_path.exists()).then(|| backup_path_for(&backup_root, path));
                        // Log the DDS right away, before the source is moved: if the app is closed or
                        // crashes mid-run, rollback still knows this DDS and where its source went (R3).
                        {
                            let _guard = log_lock.lock().unwrap_or_else(|e| e.into_inner());
                            let entry = ManifestEntry {
                                dds: dds_path.clone(),
                                source: path.clone(),
                                backup: backup_target.clone(),
                                source_fp: source_fp.clone(),
                                flipped: Some(opts.flip_vertical),
                                max_size: opts.max_size,
                            };
                            if let Err(e) = append_manifest(&log_path, &[entry]) {
                                let _ = tx_clone.send(OptimizationMsg::FileError(tr!(
                                    "Не удалось записать журнал DDS {}: {}",
                                    "Could not write the DDS log {}: {}",
                                    "无法写入 DDS 记录 {}：{}",
                                    log_path.display(),
                                    e
                                )));
                            }
                        }
                        let mut backup = None;
                        // Sources are never deleted: they are moved to a backup folder so rollback can restore them.
                        if let Some(target) = backup_target {
                            match move_file(path, &target) {
                                Ok(()) => {
                                    deleted_count.fetch_add(1, Ordering::Relaxed);
                                    backup = Some(target);
                                }
                                Err(e) => {
                                    let _ = tx_clone.send(OptimizationMsg::FileError(tr!(
                                        "{}: не удалось перенести исходник в бэкап: {}",
                                        "{}: could not move the source to the backup: {}",
                                        "{}：无法将源文件移到备份：{}",
                                        path.display(),
                                        e
                                    )));
                                }
                            }
                        }
                        if let Ok(mut list) = created.lock() {
                            list.push(ManifestEntry {
                                dds: dds_path,
                                source: path.clone(),
                                backup,
                                source_fp,
                                flipped: Some(opts.flip_vertical),
                                max_size: opts.max_size,
                            });
                        }
                    }
                    Ok(Err(e)) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_clone.send(OptimizationMsg::FileError(e));
                    }
                    Err(_panic) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_clone.send(OptimizationMsg::FileError(tr!(
                            "{}: сбой при конвертации (panic)",
                            "{}: conversion crashed (panic)",
                            "{}：转换崩溃 (panic)",
                            path.display()
                        )));
                    }
                }

                let current_done = done_count.fetch_add(1, Ordering::Relaxed) + 1;

                if current_done.is_multiple_of(5) || current_done == total {
                    let _ = tx_clone.send(OptimizationMsg::Progress {
                        done: current_done,
                        total,
                        skipped: 0,
                        current_file,
                    });
                }
            });
        };

        if let Ok(pool) = pool_res {
            pool.install(process_textures);
        } else {
            process_textures();
        }

        // Rewrite the log without duplicates, with the final backup paths (a failed move
        // leaves the source in place, so its entry gets no backup).
        if let Ok(list) = created.lock() {
            if let Err(e) = merge_manifest(&manifest_path(), &list) {
                let _ = tx.send(OptimizationMsg::FileError(tr!(
                    "Не удалось записать журнал DDS {}: {}",
                    "Could not write the DDS log {}: {}",
                    "无法写入 DDS 记录 {}：{}",
                    manifest_path().display(),
                    e
                )));
            }
        }

        if stop_flag.load(Ordering::Relaxed) {
            let _ = tx.send(OptimizationMsg::Cancelled);
        } else {
            let _ = tx.send(OptimizationMsg::Done(OptimizationSummary {
                bc7: bc7_count.load(Ordering::Relaxed),
                bc1: bc1_count.load(Ordering::Relaxed),
                skipped: skipped_count.load(Ordering::Relaxed),
                errors: error_count.load(Ordering::Relaxed),
                sources_backed_up: deleted_count.load(Ordering::Relaxed),
                foreign_dds: plan.foreign_dds,
                up_to_date: plan.up_to_date,
            }));
        }
    });
}

/// Counts for the "Scan" button — the same plan a conversion run would use
/// (same folders, same exclusions, same manifest), so the numbers match what will happen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextureSummary {
    pub to_convert: usize,
    pub up_to_date: usize,
    pub foreign_dds: usize,
    pub same_name: usize,
    /// Pixels of the textures that will really be compressed (O2: video memory estimate).
    pub pixels: u64,
    /// Pixels after applying the max_size limit (for calculating VRAM savings).
    pub resized_pixels: u64,
}

impl TextureSummary {
    /// Video memory of those textures as plain RGBA with mipmaps, and as DDS (at most: BC7,
    /// 1 byte per pixel; opaque ones become BC1, half that), in bytes. Mipmaps add a third.
    pub fn vram_estimate(&self) -> (u64, u64) {
        let with_mips = |bytes: u64| bytes + bytes / 3;
        let dds_px = if self.resized_pixels > 0 {
            self.resized_pixels
        } else {
            self.pixels
        };
        (with_mips(self.pixels * 4), with_mips(dds_px))
    }

    /// Video memory saved specifically by the max_size limit (in DDS bytes).
    pub fn vram_saved_by_limit(&self) -> u64 {
        if self.resized_pixels > 0 && self.pixels > self.resized_pixels {
            let with_mips = |bytes: u64| bytes + bytes / 3;
            with_mips(self.pixels - self.resized_pixels)
        } else {
            0
        }
    }
}

/// Pixels a texture adds to the estimate: (original, resized).
/// 0 if it stays PNG (same rules as `encode_texture`). Only the file header is read.
pub fn compressed_pixels_with_limit(path: &Path, max_size: Option<u32>) -> (u64, u64) {
    match image::image_dimensions(path) {
        Ok((w, h)) if !(w < MIN_TEXTURE_SIDE && h < MIN_TEXTURE_SIDE) => {
            let (tw, th) = target_dimensions(w, h, max_size);
            if tw % 4 == 0 && th % 4 == 0 {
                let orig = u64::from(w) * u64::from(h);
                let resized = u64::from(tw) * u64::from(th);
                (orig, resized)
            } else {
                (0, 0)
            }
        }
        _ => (0, 0),
    }
}

/// Pixels a texture adds to the estimate: 0 if it stays PNG (same rules as `encode_texture`).
/// Only the file header is read.
#[cfg(test)]
fn compressed_pixels(path: &Path) -> u64 {
    compressed_pixels_with_limit(path, None).0
}

pub fn texture_summary(
    mod_dirs: &[PathBuf],
    force: bool,
    want_flipped: bool,
    want_max_size: Option<u32>,
) -> TextureSummary {
    let manifest = manifest_index(read_manifest(&manifest_path()));
    let plan = plan_textures(mod_dirs, force, want_flipped, want_max_size, &manifest);
    let (pixels, resized_pixels) = plan
        .to_convert
        .par_iter()
        .map(|p| compressed_pixels_with_limit(p, want_max_size))
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));

    TextureSummary {
        to_convert: plan.to_convert.len(),
        up_to_date: plan.up_to_date,
        foreign_dds: plan.foreign_dds,
        same_name: plan.same_name,
        pixels,
        resized_pixels,
    }
}

/// Runs `texture_summary` in a background thread (walking big mod folders takes a while).
pub fn texture_summary_async(
    mod_dirs: Vec<PathBuf>,
    force: bool,
    want_flipped: bool,
    want_max_size: Option<u32>,
    tx: mpsc::Sender<TextureSummary>,
) {
    std::thread::spawn(move || {
        let _ = tx.send(texture_summary(&mod_dirs, force, want_flipped, want_max_size));
    });
}

// V2: VRAM Detection & Memory Check
/// Report on active mod texture VRAM usage vs GPU capacity (V2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VramReport {
    /// Dedicated video memory of the graphics card in bytes (0 if unknown).
    pub total_vram_bytes: u64,
    /// Estimated memory used by active mod textures in bytes.
    pub estimated_used_bytes: u64,
    /// Heaviest mods sorted descending by texture memory: (mod_name, vram_bytes).
    pub heavy_mods: Vec<(String, u64)>,
}

impl VramReport {
    /// Whether texture memory exceeds ~80% of dedicated video memory.
    pub fn is_critical(&self) -> bool {
        self.total_vram_bytes > 0 && self.estimated_used_bytes > (self.total_vram_bytes * 8) / 10
    }
}

pub type VramCheckResult = (VramReport, Vec<(PathBuf, u64, u64)>);

/// Detects dedicated video memory in bytes. Uses DXGI on Windows, falling back to registry.
#[cfg(windows)]
pub fn detect_dedicated_vram() -> u64 {
    if let Some(vram) = detect_vram_dxgi() {
        if vram > 0 {
            return vram;
        }
    }
    detect_vram_registry().unwrap_or(0)
}

#[cfg(not(windows))]
pub fn detect_dedicated_vram() -> u64 {
    0
}

#[cfg(windows)]
fn detect_vram_dxgi() -> Option<u64> {
    unsafe {
        extern "system" {
            fn LoadLibraryA(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
            fn GetProcAddress(module: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
            fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
        }

        let module = LoadLibraryA(c"dxgi.dll".as_ptr());
        if module.is_null() {
            return None;
        }
        let proc = GetProcAddress(module, c"CreateDXGIFactory".as_ptr());
        if proc.is_null() {
            FreeLibrary(module);
            return None;
        }

        type CreateDXGIFactoryFn =
            unsafe extern "system" fn(riid: *const [u8; 16], pp_factory: *mut *mut std::ffi::c_void) -> i32;
        let create_factory: CreateDXGIFactoryFn = std::mem::transmute(proc);

        // IID_IDXGIFactory = {7b7166ec-21c7-44ae-b21a-c9ae321ae369}
        let iid_idxgi_factory: [u8; 16] = [
            0xec, 0x66, 0x71, 0x7b, 0xc7, 0x21, 0xae, 0x44, 0xb2, 0x1a, 0xc9, 0xae, 0x32, 0x1a, 0xe3, 0x69,
        ];

        let mut factory: *mut std::ffi::c_void = std::ptr::null_mut();
        if create_factory(&iid_idxgi_factory, &mut factory) != 0 || factory.is_null() {
            FreeLibrary(module);
            return None;
        }

        let factory_vtable = *(factory as *const *const usize);
        let release_fn: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
            std::mem::transmute(*factory_vtable.add(2));
        let enum_adapters_fn: unsafe extern "system" fn(*mut std::ffi::c_void, u32, *mut *mut std::ffi::c_void) -> i32 =
            std::mem::transmute(*factory_vtable.add(7));

        #[repr(C)]
        struct DxgiAdapterDesc {
            description: [u16; 128],
            vendor_id: u32,
            device_id: u32,
            sub_sys_id: u32,
            revision: u32,
            dedicated_video_memory: usize,
            dedicated_system_memory: usize,
            shared_system_memory: usize,
            adapter_luid: [u32; 2],
        }

        let mut max_vram = 0u64;
        let mut adapter_idx = 0u32;
        loop {
            let mut adapter: *mut std::ffi::c_void = std::ptr::null_mut();
            if enum_adapters_fn(factory, adapter_idx, &mut adapter) != 0 || adapter.is_null() {
                break;
            }
            adapter_idx += 1;

            let adapter_vtable = *(adapter as *const *const usize);
            let adapter_release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                std::mem::transmute(*adapter_vtable.add(2));
            let get_desc_fn: unsafe extern "system" fn(*mut std::ffi::c_void, *mut DxgiAdapterDesc) -> i32 =
                std::mem::transmute(*adapter_vtable.add(8));

            let mut desc = std::mem::zeroed::<DxgiAdapterDesc>();
            if get_desc_fn(adapter, &mut desc) == 0 {
                let vram = desc.dedicated_video_memory as u64;
                if vram > max_vram {
                    max_vram = vram;
                }
            }
            adapter_release(adapter);
        }

        release_fn(factory);
        FreeLibrary(module);

        Some(max_vram)
    }
}

#[cfg(windows)]
fn detect_vram_registry() -> Option<u64> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let class_key = hklm
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}")
        .ok()?;

    let mut max_vram = 0u64;
    for subkey_name in class_key.enum_keys().filter_map(|k| k.ok()) {
        if !subkey_name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(card) = class_key.open_subkey(&subkey_name) {
            let qw: Option<u64> = card.get_value("HardwareInformation.qwMemorySize").ok();
            let dword: Option<u32> = card.get_value("HardwareInformation.MemorySize").ok();
            let vram = qw.unwrap_or_else(|| dword.map(|d| d as u64).unwrap_or(0));
            if vram > max_vram {
                max_vram = vram;
            }
        }
    }
    (max_vram > 0).then_some(max_vram)
}

/// "1.6" from a game version like "1.6.4633 rev1261".
pub fn short_game_version(version: &str) -> Option<String> {
    let mut parts = version.trim().split(|c: char| !c.is_ascii_digit());
    let major = parts.next().filter(|s| !s.is_empty())?;
    let minor = parts.next().filter(|s| !s.is_empty())?;
    Some(format!("{major}.{minor}"))
}

/// (major, minor) of a folder or section name like "1.6" / "v1.6".
fn version_of(name: &str) -> Option<(u32, u32)> {
    let name = name.trim().trim_start_matches(['v', 'V']);
    let (major, minor) = name.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Folders of `<loadFolders>` for `version` (exact, else `default`, else the newest older one).
fn load_folders_from_xml(xml: &str, version: Option<(u32, u32)>) -> Option<Vec<String>> {
    let re_open = regex::Regex::new(r"<([vV]\d+\.\d+|default)\s*>").ok()?;
    let re_li = regex::Regex::new(r"(?s)<li\b[^>]*>(.*?)</li>").ok()?;
    let mut sections: Vec<(String, &str)> = Vec::new();
    for c in re_open.captures_iter(xml) {
        let (Some(whole), Some(name)) = (c.get(0), c.get(1)) else {
            continue;
        };
        let body = &xml[whole.end()..];
        if let Some(len) = body.find(&format!("</{}>", name.as_str())) {
            sections.push((name.as_str().to_string(), &body[..len]));
        }
    }
    let pick = |name: &dyn Fn(&str) -> bool| sections.iter().find(|(n, _)| name(n)).map(|(_, body)| *body);
    let body = version
        .and_then(|v| pick(&|n| version_of(n) == Some(v)))
        .or_else(|| pick(&|n| n.eq_ignore_ascii_case("default")))
        .or_else(|| {
            sections
                .iter()
                .filter_map(|(n, body)| Some((version_of(n)?, *body)))
                .filter(|(v, _)| version.is_none_or(|want| *v <= want))
                .max_by_key(|(v, _)| *v)
                .map(|(_, body)| body)
        })?;
    Some(
        re_li
            .captures_iter(body)
            .filter_map(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .collect(),
    )
}

/// The folders the game loads from a mod (V2): those of `loadFolders.xml` for this version,
/// else the mod root, `Common` and the best version folder (the game's own, or the newest
/// older one).
pub fn mod_load_folders(mod_dir: &Path, game_version: Option<&str>) -> Vec<PathBuf> {
    let version = game_version.and_then(short_game_version).and_then(|v| version_of(&v));
    let xml = ["loadFolders.xml", "LoadFolders.xml"]
        .iter()
        .find_map(|n| std::fs::read_to_string(mod_dir.join(n)).ok());
    if let Some(folders) = xml.and_then(|x| load_folders_from_xml(&x, version)) {
        if !folders.is_empty() {
            let mut out: Vec<PathBuf> = Vec::new();
            for f in folders {
                let rel = f.trim_matches(['/', '\\']);
                let dir = if rel.is_empty() {
                    mod_dir.to_path_buf()
                } else {
                    mod_dir.join(rel)
                };
                if !out.contains(&dir) {
                    out.push(dir);
                }
            }
            return out;
        }
    }
    let mut out = vec![mod_dir.to_path_buf()];
    let common = mod_dir.join("Common");
    if common.is_dir() {
        out.push(common);
    }
    let best_version_dir = std::fs::read_dir(mod_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((version_of(&e.file_name().to_string_lossy())?, e.path())))
        .filter(|(v, _)| version.is_none_or(|want| *v <= want))
        .max_by_key(|(v, _)| *v);
    if let Some((_, dir)) = best_version_dir {
        out.push(dir);
    }
    out
}

/// Texture files the game loads from one mod: everything under `<load folder>/Textures`
/// (apparel, bodies and UI too: they fill video memory like any other).
fn mod_texture_files(mod_dir: &Path, game_version: Option<&str>) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for folder in mod_load_folders(mod_dir, game_version) {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let textures = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("textures"));
        let Some(textures) = textures else { continue };
        for entry in WalkDir::new(textures.path()).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "dds") {
                continue;
            }
            if seen.insert(path_key(entry.path())) {
                if let Ok(meta) = entry.metadata() {
                    out.push((entry.into_path(), meta));
                }
            }
        }
    }
    out
}

/// Changes when a texture is added, removed or rewritten anywhere in the mod's texture folders.
fn texture_fingerprint(files: &[(PathBuf, std::fs::Metadata)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    files.len().hash(&mut h);
    for (path, meta) in files {
        path_key(path).hash(&mut h);
        meta.len().hash(&mut h);
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .hash(&mut h);
    }
    h.finish()
}

/// Estimated VRAM used by textures of one mod in bytes (V2).
/// DDS files are counted at their file length (compressed data + mipmaps); a PNG/JPG next to
/// a DDS is not loaded. PNG/JPG: uncompressed RGBA on 1.6 (Unity), BC7 on 1.7 (Waylith).
fn estimate_texture_bytes(files: &[(PathBuf, std::fs::Metadata)], engine: crate::game_install::GameEngine) -> u64 {
    let with_mips = |b: u64| b + b / 3;
    let bytes_per_px = if engine == crate::game_install::GameEngine::Waylith {
        1
    } else {
        4
    };
    let dds: std::collections::HashSet<String> = files
        .iter()
        .filter(|(p, _)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dds")))
        .map(|(p, _)| path_key(&p.with_extension("")))
        .collect();
    files
        .iter()
        .map(|(path, meta)| {
            let is_dds = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dds"));
            if is_dds {
                meta.len()
            } else if dds.contains(&path_key(&path.with_extension(""))) {
                0
            } else {
                image::image_dimensions(path)
                    .map(|(w, h)| with_mips(u64::from(w) * u64::from(h) * bytes_per_px))
                    .unwrap_or(0)
            }
        })
        .sum()
}

/// Estimated VRAM used by textures of one mod in bytes (V2).
pub fn estimate_mod_vram(mod_dir: &Path, engine: crate::game_install::GameEngine, game_version: Option<&str>) -> u64 {
    estimate_texture_bytes(&mod_texture_files(mod_dir, game_version), engine)
}

/// Evaluates active mod texture memory against dedicated video memory (V2).
/// A mod's estimate is reused while its texture files stay the same (cache: dir ->
/// (fingerprint, bytes)); returns the new cache entries too.
pub fn check_vram_usage(
    active_mods: &[(String, PathBuf)],
    engine: crate::game_install::GameEngine,
    game_version: Option<&str>,
    cache: &HashMap<PathBuf, (u64, u64)>,
) -> (VramReport, Vec<(PathBuf, u64, u64)>) {
    let total_vram = detect_dedicated_vram();
    // Per mod: its bytes and, when estimated anew, the cache entry (dir, fingerprint, bytes).
    type CacheEntry = (PathBuf, u64, u64);
    let per_mod: Vec<(u64, Option<CacheEntry>)> = active_mods
        .par_iter()
        .map(|(_, dir)| {
            let files = mod_texture_files(dir, game_version);
            let fp = texture_fingerprint(&files);
            match cache.get(dir) {
                Some(&(cached_fp, bytes)) if cached_fp == fp => (bytes, None),
                _ => {
                    let bytes = estimate_texture_bytes(&files, engine);
                    (bytes, Some((dir.clone(), fp, bytes)))
                }
            }
        })
        .collect();

    let total_used = per_mod.iter().map(|(b, _)| b).sum();
    let mut heavy_mods: Vec<(String, u64)> = active_mods
        .iter()
        .zip(&per_mod)
        .filter(|(_, (bytes, _))| *bytes >= 25 * 1024 * 1024)
        .map(|((name, _), (bytes, _))| (name.clone(), *bytes))
        .collect();
    heavy_mods.sort_by_key(|b| std::cmp::Reverse(b.1));
    heavy_mods.truncate(8);
    let updated_cache = per_mod.into_iter().filter_map(|(_, c)| c).collect();

    (
        VramReport {
            total_vram_bytes: total_vram,
            estimated_used_bytes: total_used,
            heavy_mods,
        },
        updated_cache,
    )
}

/// Asynchronous VRAM check for active mods (V2).
pub fn check_vram_async(
    active_mods: Vec<(String, PathBuf)>,
    engine: crate::game_install::GameEngine,
    game_version: Option<String>,
    cache: HashMap<PathBuf, (u64, u64)>,
    tx: mpsc::Sender<VramCheckResult>,
) {
    std::thread::spawn(move || {
        let res = check_vram_usage(&active_mods, engine, game_version.as_deref(), &cache);
        let _ = tx.send(res);
    });
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RollbackResult {
    /// .dds files removed
    pub removed: usize,
    /// source PNG/JPG files restored from backup
    pub restored: usize,
    pub errors: usize,
    /// DDS files with a PNG/JPG twin that are NOT in the manager's log: made by an older
    /// version of this optimizer, by RimPy, or shipped by the mod author. Not deleted here —
    /// the user decides after seeing the list (R2).
    pub unlogged: Vec<PathBuf>,
}

/// Undoes texture optimization:
/// 1. every DDS listed in the manifest is removed and its backed-up source restored;
/// 2. DDS files that are not in the manifest but have a PNG/JPG twin inside a Textures
///    folder are only listed (`unlogged`); `delete_files` removes them if the user agrees.
pub fn rollback_dds(mod_dirs: &[PathBuf]) -> RollbackResult {
    rollback_with_manifest(mod_dirs, &manifest_path())
}

pub fn rollback_with_manifest(mod_dirs: &[PathBuf], manifest: &Path) -> RollbackResult {
    rollback_in(mod_dirs, manifest, false)
}

/// `folder\` in lower case with `\` separators, to tell whether a path is inside it.
fn dir_prefix(dir: &Path) -> String {
    let mut key = path_key(dir).replace('/', "\\");
    if !key.ends_with('\\') {
        key.push('\\');
    }
    key
}

/// Rollback; with `only_these`, only the DDS inside `mod_dirs` (e.g. one mod) are undone
/// and the rest of the log is kept as it is (O1).
pub fn rollback_in(mod_dirs: &[PathBuf], manifest: &Path, only_these: bool) -> RollbackResult {
    let mut res = RollbackResult::default();
    // Entries that stay in the log: failed ones, and (with `only_these`) other mods'.
    let mut failed: Vec<ManifestEntry> = Vec::new();
    let roots: Vec<String> = mod_dirs.iter().map(|d| dir_prefix(d)).collect();
    let inside = |p: &Path| {
        let key = path_key(p).replace('/', "\\");
        roots.iter().any(|root| key.starts_with(root.as_str()))
    };

    for entry in read_manifest(manifest) {
        if only_these && !inside(&entry.dds) {
            failed.push(entry);
            continue;
        }
        let mut ok = true;
        if let Some(ref backup) = entry.backup {
            if backup.is_file() && !entry.source.exists() {
                match move_file(backup, &entry.source) {
                    Ok(()) => res.restored += 1,
                    Err(_) => ok = false,
                }
            }
        }
        // Only remove the DDS once the source is back in place, so the mod keeps a texture.
        if ok && entry.dds.is_file() && entry.source.exists() {
            match std::fs::remove_file(&entry.dds) {
                Ok(()) => res.removed += 1,
                Err(_) => ok = false,
            }
        }
        if !ok {
            res.errors += 1;
            failed.push(entry);
        }
    }
    if write_manifest(manifest, &failed).is_err() {
        res.errors += 1;
    }

    for mod_dir in mod_dirs {
        for entry in WalkDir::new(mod_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file() || !is_in_textures_folder(path) {
                continue;
            }
            let is_dds = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("dds"))
                .unwrap_or(false);
            if !is_dds {
                continue;
            }
            let has_source = ["png", "jpg", "jpeg"]
                .iter()
                .any(|ext| path.with_extension(ext).is_file());
            if has_source {
                res.unlogged.push(path.to_path_buf());
            }
        }
    }

    res
}

/// Deletes the given files; returns (removed, errors).
pub fn delete_files(paths: &[PathBuf]) -> (usize, usize) {
    paths.iter().fold((0, 0), |(ok, err), p| match std::fs::remove_file(p) {
        Ok(()) => (ok + 1, err),
        Err(_) => (ok, err + 1),
    })
}

/// Rollback in a background thread; `only_these`: see `rollback_in`.
pub fn rollback_dds_async(mod_dirs: Vec<PathBuf>, only_these: bool, tx: mpsc::Sender<RollbackResult>) {
    std::thread::spawn(move || {
        let res = rollback_in(&mod_dirs, &manifest_path(), only_these);
        let _ = tx.send(res);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_opt_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Live check of the user's optimizer DDS against their PNG sources:
    /// `cargo test --lib live_dds_orientation -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_dds_orientation() {
        let entries = read_manifest(&manifest_path());
        let step = (entries.len() / 40).max(1);
        let mut counts = [0usize; 3];
        for e in entries.iter().step_by(step).take(40) {
            match detect_orientation(&e.dds, &e.source) {
                Ok(o) => {
                    counts[o as usize] += 1;
                    println!("{o:?}  {}", e.dds.display());
                }
                Err(err) => println!("skip: {err}"),
            }
        }
        println!(
            "--- as source: {}, flipped: {}, unclear: {}",
            counts[0], counts[1], counts[2]
        );
    }

    /// Runs the flip pass on copies of real optimizer DDS (originals untouched):
    /// `cargo test --release --lib live_flip_on_copies -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_flip_on_copies() {
        let dir = temp_dir("live_flip");
        let entries = read_manifest(&manifest_path());
        let step = (entries.len() / 30).max(1);
        let mut copies = Vec::new();
        for (i, e) in entries.iter().step_by(step).take(30).enumerate() {
            let (dds, png) = (dir.join(format!("{i}.dds")), dir.join(format!("{i}.png")));
            if std::fs::copy(&e.dds, &dds).is_ok() && std::fs::copy(&e.source, &png).is_ok() {
                copies.push(ManifestEntry {
                    dds,
                    source: png,
                    backup: None,
                    source_fp: None,
                    flipped: e.flipped,
                    max_size: e.max_size,
                });
            }
        }
        let manifest = dir.join("dds_manifest.tsv");
        append_manifest(&manifest, &copies).unwrap();
        let (tx, _rx) = mpsc::channel();
        let s = fix_orientation_with_manifest(&manifest, true, 8, &tx, &AtomicBool::new(false));
        println!(
            "copies: {}, flipped: {}, already right: {}, errors: {}",
            copies.len(),
            s.fixed,
            s.already_ok,
            s.errors
        );
        let mut wrong = 0;
        for c in &copies {
            match detect_orientation(&c.dds, &c.source) {
                Ok(Orientation::AsSource) => wrong += 1,
                Ok(_) => {}
                Err(e) => println!("check failed: {e}"),
            }
        }
        println!("still the PNG way up after the pass: {wrong}");
        assert_eq!(wrong, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_logged_dds_stored_the_wrong_way_up() {
        let dir = temp_dir("leftover");
        let kept = dir.join("a.dds");
        let flipped = dir.join("b.dds");
        std::fs::write(&kept, b"DDS ").unwrap();
        std::fs::write(&flipped, b"DDS ").unwrap();
        let entry = |dds: PathBuf, f: Option<bool>| ManifestEntry {
            source: dds.with_extension("png"),
            dds,
            backup: None,
            source_fp: None,
            flipped: f,
            max_size: None,
        };
        let manifest = dir.join("dds_manifest.tsv");
        append_manifest(
            &manifest,
            &[
                entry(kept, None),
                entry(flipped, Some(true)),
                entry(dir.join("gone.dds"), None),
            ],
        )
        .unwrap();
        // On 1.7 the old (unflipped) one is wrong, on 1.6 the flipped one; missing files don't count.
        assert_eq!(count_wrong_orientation(&manifest, true), 1);
        assert_eq!(count_wrong_orientation(&manifest, false), 1);
        assert_eq!(count_wrong_orientation(&dir.join("missing.tsv"), true), 0);
        assert_eq!(
            read_manifest(&manifest)[1].flipped,
            Some(true),
            "orientation survives a save/load"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flip_pass_turns_only_wrong_textures_and_is_repeatable() {
        let dir = temp_dir("flip");
        let png = dir.join("Textures").join("tex.png");
        std::fs::create_dir_all(png.parent().unwrap()).unwrap();
        write_png(&png, 64, 64, 255);
        optimize_single_texture(&png, ConvertOptions::default()).unwrap();
        let dds = png.with_extension("dds");
        assert_eq!(detect_orientation(&dds, &png).unwrap(), Orientation::AsSource);

        let manifest = dir.join("dds_manifest.tsv");
        let entry = ManifestEntry {
            dds: dds.clone(),
            source: png.clone(),
            backup: None,
            source_fp: None,
            flipped: None,
            max_size: None,
        };
        append_manifest(&manifest, &[entry]).unwrap();
        let (tx, _rx) = mpsc::channel();
        let stop = AtomicBool::new(false);

        // For 1.7: the DDS gets the picture upside down, the log remembers it.
        let first = fix_orientation_with_manifest(&manifest, true, 2, &tx, &stop);
        assert_eq!((first.fixed, first.already_ok, first.errors), (1, 0, 0));
        assert_eq!(detect_orientation(&dds, &png).unwrap(), Orientation::Flipped);
        assert_eq!(read_manifest(&manifest)[0].flipped, Some(true));
        assert_eq!(count_wrong_orientation(&manifest, true), 0);

        // Pressing again does nothing; switching back to 1.6 turns it back.
        let again = fix_orientation_with_manifest(&manifest, true, 2, &tx, &stop);
        assert_eq!(again.fixed + again.already_ok, 0);
        let back = fix_orientation_with_manifest(&manifest, false, 2, &tx, &stop);
        assert_eq!(back.fixed, 1);
        assert_eq!(detect_orientation(&dds, &png).unwrap(), Orientation::AsSource);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_png(path: &Path, w: u32, h: u32, alpha: u8) {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 4) as u8, (y * 4) as u8, 120, alpha]));
        img.save(path).unwrap();
    }

    /// Reads (fourcc, mipmap count) from a DDS header.
    fn dds_info(path: &Path) -> (String, u32) {
        let b = std::fs::read(path).unwrap();
        assert_eq!(&b[0..4], b"DDS ");
        let mips = u32::from_le_bytes([b[28], b[29], b[30], b[31]]);
        (String::from_utf8_lossy(&b[84..88]).to_string(), mips)
    }

    #[test]
    fn converts_like_todds_bc7_bc1_with_mipmaps() {
        let dir = temp_dir("convert");
        let opaque = dir.join("opaque.png");
        let transparent = dir.join("transparent.png");
        let odd = dir.join("odd.png");
        let tiny = dir.join("tiny.png");
        write_png(&opaque, 128, 64, 255);
        write_png(&transparent, 64, 64, 128);
        write_png(&odd, 66, 64, 255);
        write_png(&tiny, 32, 32, 255);

        let opts = ConvertOptions::default();
        assert_eq!(optimize_single_texture(&opaque, opts).unwrap(), ConvertOutcome::Bc1);
        let (fourcc, mips) = dds_info(&opaque.with_extension("dds"));
        assert_eq!(fourcc, "DXT1", "BC1 uses the classic DXT1 header");
        assert_eq!(mips, 8, "128x64 -> full chain down to 1x1");

        assert_eq!(
            optimize_single_texture(&transparent, opts).unwrap(),
            ConvertOutcome::Bc7
        );
        let (fourcc, mips) = dds_info(&transparent.with_extension("dds"));
        assert_eq!(fourcc, "DX10", "BC7 needs the DX10 header");
        assert_eq!(mips, 7);

        assert_eq!(
            optimize_single_texture(&odd, opts).unwrap(),
            ConvertOutcome::SkippedSize
        );
        assert!(!odd.with_extension("dds").exists(), "never stretched, left as PNG");
        assert_eq!(
            optimize_single_texture(&tiny, opts).unwrap(),
            ConvertOutcome::SkippedSmall
        );
        assert!(!dir.join("opaque.dds.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_name_sources_are_left_alone() {
        let dir = temp_dir("same_name");
        let tex = dir.join("Mod").join("Textures").join("Things");
        std::fs::create_dir_all(&tex).unwrap();
        // icon.png and icon.JPG would both become icon.dds; solo.png is on its own.
        write_png(&tex.join("icon.png"), 64, 64, 255);
        std::fs::copy(tex.join("icon.png"), tex.join("icon.JPG")).unwrap();
        write_png(&tex.join("solo.png"), 64, 64, 255);

        let plan = plan_textures(&[dir.join("Mod")], true, false, None, &HashMap::new());
        let names: Vec<String> = plan
            .to_convert
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["solo.png"], "neither icon source is converted");
        assert_eq!(plan.same_name, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_rewrite_leaves_no_temp_file() {
        let dir = temp_dir("manifest_tmp");
        let log = dir.join("dds_manifest.tsv");
        let entry = |name: &str| ManifestEntry {
            dds: dir.join(format!("{name}.dds")),
            source: dir.join(format!("{name}.png")),
            backup: None,
            source_fp: Some("1:1".into()),
            flipped: Some(false),
            max_size: None,
        };
        merge_manifest(&log, &[entry("a"), entry("b")]).unwrap();
        merge_manifest(&log, &[entry("c")]).unwrap();
        assert_eq!(read_manifest(&log).len(), 3);
        assert!(!log.with_extension("tsv.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_detects_stale_and_foreign_dds() {
        let dir = temp_dir("plan");
        let tex = dir.join("Mod").join("Textures").join("Things");
        std::fs::create_dir_all(&tex).unwrap();
        let ours_fresh = tex.join("a.png");
        let ours_stale = tex.join("b.png");
        let foreign = tex.join("c.png");
        let new_tex = tex.join("d.png");
        for p in [&ours_fresh, &ours_stale, &foreign, &new_tex] {
            write_png(p, 64, 64, 255);
        }
        for p in [&ours_fresh, &ours_stale, &foreign] {
            std::fs::write(p.with_extension("dds"), b"dds").unwrap();
        }
        let entries = vec![
            ManifestEntry {
                dds: ours_fresh.with_extension("dds"),
                source: ours_fresh.clone(),
                backup: None,
                source_fp: source_fingerprint(&ours_fresh),
                flipped: None,
                max_size: None,
            },
            ManifestEntry {
                dds: ours_stale.with_extension("dds"),
                source: ours_stale.clone(),
                backup: None,
                source_fp: Some("1:1".into()),
                flipped: None,
                max_size: None,
            },
        ];
        let manifest = manifest_index(entries);

        let plan = plan_textures(&[dir.join("Mod")], false, false, None, &manifest);
        let mut names: Vec<String> = plan
            .to_convert
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["b.png", "d.png"],
            "stale ours + brand new; fresh ours and foreign skipped"
        );
        assert_eq!((plan.up_to_date, plan.foreign_dds), (1, 1));

        let forced = plan_textures(&[dir.join("Mod")], true, false, None, &manifest);
        assert_eq!(
            forced.to_convert.len(),
            3,
            "force re-encodes ours, never the author's DDS"
        );

        // On 1.7 the fresh but unflipped DDS is the wrong way up: re-encoded too.
        let on_waylith = plan_textures(&[dir.join("Mod")], false, true, None, &manifest);
        assert_eq!(
            on_waylith.to_convert.len(),
            3,
            "wrong-way-up ours are redone, the author's DDS still never"
        );
        assert_eq!(on_waylith.up_to_date, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_path_keeps_drive_and_folders() {
        let root = PathBuf::from(r"C:\backup");
        let p = backup_path_for(&root, Path::new(r"F:\Steam\294100\123\Textures\a.png"));
        assert_eq!(p, PathBuf::from(r"C:\backup\F\Steam\294100\123\Textures\a.png"));
    }

    #[test]
    fn rollback_restores_backup_and_removes_dds() {
        let dir = temp_dir("rollback");
        let tex = dir.join("Mod").join("Textures").join("UI");
        std::fs::create_dir_all(&tex).unwrap();
        let source = tex.join("icon.png");
        let dds = tex.join("icon.dds");
        let backup = dir.join("backup").join("icon.png");
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        std::fs::write(&backup, b"png").unwrap();
        std::fs::write(&dds, b"dds").unwrap();

        // Legacy DDS (no manifest entry) in an excluded UI folder, with its PNG twin.
        let legacy_png = tex.join("legacy.png");
        let legacy_dds = tex.join("legacy.dds");
        std::fs::write(&legacy_png, b"png").unwrap();
        std::fs::write(&legacy_dds, b"dds").unwrap();

        let manifest = dir.join("manifest.tsv");
        append_manifest(
            &manifest,
            &[ManifestEntry {
                dds: dds.clone(),
                source: source.clone(),
                backup: Some(backup.clone()),
                source_fp: None,
                flipped: None,
                max_size: None,
            }],
        )
        .unwrap();

        let res = rollback_with_manifest(&[dir.join("Mod")], &manifest);
        assert_eq!(res.restored, 1);
        assert_eq!(res.removed, 1, "only the logged DDS is removed");
        assert_eq!(res.errors, 0);
        assert!(source.is_file(), "source restored");
        assert!(!dds.exists(), "logged dds removed");
        assert!(legacy_dds.exists(), "unlogged dds is kept until the user agrees");
        assert_eq!(res.unlogged, vec![legacy_dds.clone()]);
        assert!(!manifest.exists(), "manifest cleared");

        assert_eq!(delete_files(&res.unlogged), (1, 0));
        assert!(!legacy_dds.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vram_estimate_counts_only_textures_that_get_compressed() {
        let dir = temp_dir("vram");
        write_png(&dir.join("big.png"), 256, 128, 255); // compressed: 32768 px
        write_png(&dir.join("tiny.png"), 32, 32, 255); // < 64 px: stays PNG
        write_png(&dir.join("odd.png"), 130, 64, 255); // not a multiple of 4: stays PNG
        assert_eq!(compressed_pixels(&dir.join("big.png")), 256 * 128);
        assert_eq!(compressed_pixels(&dir.join("tiny.png")), 0);
        assert_eq!(compressed_pixels(&dir.join("odd.png")), 0);

        let summary = TextureSummary {
            pixels: 3 * 1024 * 1024,
            ..Default::default()
        };
        // RGBA: 4 bytes per pixel + a third for mipmaps; BC7: 1 byte per pixel + a third.
        assert_eq!(summary.vram_estimate(), (16 * 1024 * 1024, 4 * 1024 * 1024));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_of_one_mod_keeps_the_others() {
        let dir = temp_dir("rollback_one");
        let manifest = dir.join("manifest.tsv");
        // Two mods, one of them a prefix of the other's name ("Mod" / "Mod2").
        let mut entries = Vec::new();
        let mut files = Vec::new();
        for name in ["Mod", "Mod2"] {
            let tex = dir.join(name).join("Textures").join("Things");
            std::fs::create_dir_all(&tex).unwrap();
            let (source, dds) = (tex.join("a.png"), tex.join("a.dds"));
            let backup = dir.join("backup").join(name).join("a.png");
            std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
            std::fs::write(&backup, b"png").unwrap();
            std::fs::write(&dds, b"dds").unwrap();
            entries.push(ManifestEntry {
                dds: dds.clone(),
                source: source.clone(),
                backup: Some(backup),
                source_fp: None,
                flipped: Some(false),
                max_size: None,
            });
            files.push((source, dds));
        }
        append_manifest(&manifest, &entries).unwrap();

        let res = rollback_in(&[dir.join("Mod")], &manifest, true);
        assert_eq!((res.restored, res.removed, res.errors), (1, 1, 0));
        assert!(files[0].0.is_file() && !files[0].1.exists(), "Mod rolled back");
        assert!(!files[1].0.exists() && files[1].1.is_file(), "Mod2 untouched");
        let left = read_manifest(&manifest);
        assert_eq!(left.len(), 1, "Mod2 stays in the log");
        assert_eq!(left[0].dds, files[1].1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_dimensions_preserves_aspect_ratio_and_guarantees_multiples_of_4() {
        assert_eq!(target_dimensions(4096, 4096, Some(2048)), (2048, 2048));
        assert_eq!(target_dimensions(3840, 2160, Some(2048)), (2048, 1152));
        assert_eq!(target_dimensions(2500, 1500, Some(1024)), (1024, 616));
        assert_eq!(target_dimensions(100, 3000, Some(512)), (16, 512));
        assert_eq!(target_dimensions(1024, 1024, Some(2048)), (1024, 1024));
        assert_eq!(target_dimensions(1024, 1024, None), (1024, 1024));
        // Clamp to max_s and minimum 4
        let (w, h) = target_dimensions(5000, 2, Some(512));
        assert_eq!(w, 512);
        assert_eq!(h, 4);
    }

    #[test]
    fn encode_texture_with_max_size_downscales_to_requested_limit() {
        let dir = temp_dir("encode_max_size");
        let png = dir.join("wide.png");
        write_png(&png, 512, 256, 255);
        let dds = png.with_extension("dds");

        let opts = ConvertOptions {
            flip_vertical: false,
            max_size: Some(256),
        };
        assert_eq!(optimize_single_texture(&png, opts).unwrap(), ConvertOutcome::Bc1);
        assert!(dds.is_file());

        let bytes = std::fs::read(&dds).unwrap();
        let height = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let width = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        assert_eq!((width, height), (256, 128), "downscaled 512x256 -> 256x128");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_textures_re_encodes_when_max_size_setting_differs() {
        let dir = temp_dir("plan_max_size");
        let tex = dir.join("Mod").join("Textures").join("Things");
        std::fs::create_dir_all(&tex).unwrap();
        let png = tex.join("art.png");
        write_png(&png, 1600, 800, 255);
        let dds = png.with_extension("dds");
        std::fs::write(&dds, b"dds").unwrap();

        let manifest = manifest_index(vec![ManifestEntry {
            dds: dds.clone(),
            source: png.clone(),
            backup: None,
            source_fp: source_fingerprint(&png),
            flipped: Some(false),
            max_size: Some(2048),
        }]);

        // Same size setting -> up to date
        let plan_same = plan_textures(&[dir.join("Mod")], false, false, Some(2048), &manifest);
        assert_eq!(plan_same.up_to_date, 1);
        assert_eq!(plan_same.to_convert.len(), 0);

        // Different size setting -> to_convert (re-encode)
        let plan_smaller = plan_textures(&[dir.join("Mod")], false, false, Some(1024), &manifest);
        assert_eq!(plan_smaller.up_to_date, 0);
        assert_eq!(plan_smaller.to_convert.len(), 1);

        // No limit: 1600 px fits under 2048 too, it comes out the same, nothing to redo.
        let plan_none = plan_textures(&[dir.join("Mod")], false, false, None, &manifest);
        assert_eq!(plan_none.up_to_date, 1);
        assert_eq!(plan_none.to_convert.len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vram_saved_by_limit_calculates_correct_savings() {
        let summary = TextureSummary {
            pixels: 4096 * 4096,
            resized_pixels: 2048 * 2048,
            ..Default::default()
        };
        // 16M px -> 4M px. Saved 12M px. With mips (+ 1/3) = 16M bytes = 16 MB.
        assert_eq!(summary.vram_saved_by_limit(), 16 * 1024 * 1024);
    }

    #[test]
    fn vram_critical_threshold_test() {
        let rep_critical = VramReport {
            total_vram_bytes: 10_000_000_000,
            estimated_used_bytes: 8_500_000_000, // 85%
            heavy_mods: vec![("Heavy Mod".into(), 4_000_000_000)],
        };
        assert!(rep_critical.is_critical());

        let rep_ok = VramReport {
            total_vram_bytes: 10_000_000_000,
            estimated_used_bytes: 7_500_000_000, // 75%
            heavy_mods: vec![],
        };
        assert!(!rep_ok.is_critical());

        let rep_no_total = VramReport {
            total_vram_bytes: 0,
            estimated_used_bytes: 999_999_999,
            heavy_mods: vec![],
        };
        assert!(!rep_no_total.is_critical());
    }

    #[test]
    fn detect_dedicated_vram_smoke() {
        let vram = detect_dedicated_vram();
        #[cfg(target_os = "windows")]
        {
            // On a dev PC with a dedicated GPU (e.g. RX 5700 XT), this returns the VRAM in bytes.
            // In headless CI or VMs it may be 0, so we check that it runs safely and returns a valid value.
            assert!(vram < u64::MAX);
        }
        #[cfg(not(target_os = "windows"))]
        assert_eq!(vram, 0);
    }

    #[test]
    fn check_vram_usage_and_cache_test() {
        use crate::game_install::GameEngine;
        let dir = temp_dir("vram_check");
        let mod_dir = dir.join("TestMod");
        let tex_dir = mod_dir.join("Textures");
        std::fs::create_dir_all(&tex_dir).unwrap();
        let png = tex_dir.join("test.png");
        write_png(&png, 64, 64, 255);
        let dds = tex_dir.join("test.dds");
        std::fs::write(&dds, vec![0u8; 1024]).unwrap();

        let mods = vec![("Test Mod".to_string(), mod_dir.clone())];
        let mut cache = HashMap::new();

        // 1st pass: cache miss, calculates usage
        let (report1, new_cache) = check_vram_usage(&mods, GameEngine::Unity, None, &cache);
        assert!(report1.estimated_used_bytes > 0);
        assert_eq!(new_cache.len(), 1);

        for (p, mtime, bytes) in new_cache {
            cache.insert(p, (mtime, bytes));
        }

        // 2nd pass: cache hit, gives exact same used bytes
        let (report2, new_cache2) = check_vram_usage(&mods, GameEngine::Unity, None, &cache);
        assert_eq!(report1.estimated_used_bytes, report2.estimated_used_bytes);
        assert_eq!(new_cache2.len(), 0, "no new items should be cached on cache hit");

        // A texture changed deep inside a subfolder: estimated again.
        let deep = tex_dir.join("Things").join("Item");
        std::fs::create_dir_all(&deep).unwrap();
        write_png(&deep.join("new.png"), 128, 128, 255);
        let (report3, new_cache3) = check_vram_usage(&mods, GameEngine::Unity, None, &cache);
        assert_eq!(new_cache3.len(), 1);
        assert!(report3.estimated_used_bytes > report2.estimated_used_bytes);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vram_estimate_counts_only_loaded_folders_and_all_texture_kinds() {
        use crate::game_install::GameEngine;
        let dir = temp_dir("vram_folders");
        let m = dir.join("Mod");
        // Old and current version folders, an apparel texture (skipped by the optimizer,
        // still in video memory).
        for (sub, side) in [
            ("1.5/Textures", 256u32),
            ("1.6/Textures/Things/Apparel", 128),
            ("Textures", 64),
        ] {
            std::fs::create_dir_all(m.join(sub)).unwrap();
            write_png(&m.join(sub).join("t.png"), side, side, 255);
        }
        let px = |s: u64| (s * s * 4) + (s * s * 4) / 3;
        // 1.6: root + 1.6, not 1.5.
        assert_eq!(
            estimate_mod_vram(&m, GameEngine::Unity, Some("1.6.4633 rev1261")),
            px(128) + px(64)
        );
        // 1.5: root + 1.5.
        assert_eq!(
            estimate_mod_vram(&m, GameEngine::Unity, Some("1.5.4409")),
            px(256) + px(64)
        );

        // loadFolders.xml decides, for this version.
        std::fs::write(
            m.join("loadFolders.xml"),
            "<loadFolders>\n  <v1.5>\n    <li>/</li>\n  </v1.5>\n  <v1.6>\n    <li>1.5</li>\n    <li IfModActive=\"x.y\">/</li>\n  </v1.6>\n</loadFolders>",
        )
        .unwrap();
        assert_eq!(
            estimate_mod_vram(&m, GameEngine::Unity, Some("1.6.1")),
            px(256) + px(64)
        );
        assert_eq!(estimate_mod_vram(&m, GameEngine::Unity, Some("1.5.1")), px(64));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn short_game_version_takes_major_and_minor() {
        assert_eq!(short_game_version("1.6.4633 rev1261").as_deref(), Some("1.6"));
        assert_eq!(short_game_version("1.7.4990.1069").as_deref(), Some("1.7"));
        assert_eq!(short_game_version("unknown"), None);
    }
}
