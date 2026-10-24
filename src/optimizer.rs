use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::SystemTime;

use rayon::prelude::*;
use walkdir::WalkDir;

use crate::models::ModInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptimizeScope {
    #[default]
    AllInstalled,
    ActiveOnly,
    SelectedOnly,
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
        c.as_os_str().to_str().map(|s| {
            let lower = s.to_ascii_lowercase();
            matches!(lower.as_str(), "ui" | "gui" | "icons" | "defs" | "pawn" | "pawns" | "hair" | "heads" | "bodies" | "apparel")
        }).unwrap_or(false)
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
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
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
    /// Flip the image vertically before encoding (what RimPy does). Off by default:
    /// ToDDS — the converter that works on RimWorld 1.6 — does not flip.
    pub flip_vertical: bool,
}

/// Textures smaller than this on both sides are left as PNG: no gain, only risk.
pub const MIN_TEXTURE_SIDE: u32 = 64;

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
    let mut img = image::open(source).map_err(|e| format!("{}: {}", source.display(), e))?;
    if opts.flip_vertical {
        img = img.flipv();
    }

    let (w, h) = (img.width(), img.height());
    if w < MIN_TEXTURE_SIDE && h < MIN_TEXTURE_SIDE {
        return Ok(ConvertOutcome::SkippedSmall);
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
    let dds_path = source.with_extension("dds");
    let tmp_path = source.with_extension("dds.tmp");
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
    std::fs::rename(&tmp_path, &dds_path).map_err(|e| {
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
}

pub fn plan_textures(
    mod_dirs: &[PathBuf],
    force: bool,
    manifest: &HashMap<String, ManifestEntry>,
) -> TexturePlan {
    let mut plan = TexturePlan::default();
    for mod_dir in mod_dirs {
        for path in walk_mod_textures(mod_dir) {
            let dds = path.with_extension("dds");
            if !dds.exists() {
                plan.to_convert.push(path);
                continue;
            }
            match manifest.get(&path_key(&dds)) {
                Some(entry) => {
                    let fresh = entry.source_fp.is_some() && entry.source_fp == source_fingerprint(&path);
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
            if dds.is_empty() || source.is_empty() {
                return None;
            }
            Some(ManifestEntry {
                dds: PathBuf::from(dds),
                source: PathBuf::from(source),
                backup: backup.map(PathBuf::from),
                source_fp: source_fp.map(str::to_string),
            })
        })
        .collect()
}

fn manifest_line(e: &ManifestEntry) -> String {
    format!(
        "{}\t{}\t{}\t{}\n",
        e.dds.display(),
        e.source.display(),
        e.backup.as_ref().map(|b| b.display().to_string()).unwrap_or_default(),
        e.source_fp.clone().unwrap_or_default()
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
    let text: String = entries.iter().map(manifest_line).collect();
    std::fs::write(path, text)
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
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
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
        let plan = plan_textures(&mod_dirs, force, &manifest);
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
                let conv_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    optimize_single_texture(path, opts)
                }));

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
                        let mut backup = None;
                        // Sources are never deleted: they are moved to a backup folder so rollback can restore them.
                        if delete_source && dds_path.exists() {
                            let target = backup_path_for(&backup_root, path);
                            match move_file(path, &target) {
                                Ok(()) => {
                                    deleted_count.fetch_add(1, Ordering::Relaxed);
                                    backup = Some(target);
                                }
                                Err(e) => {
                                    let _ = tx_clone.send(OptimizationMsg::FileError(format!(
                                        "{}: не удалось перенести исходник в бэкап: {}",
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
                            });
                        }
                    }
                    Ok(Err(e)) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_clone.send(OptimizationMsg::FileError(e));
                    }
                    Err(_panic) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_clone.send(OptimizationMsg::FileError(format!(
                            "{}: сбой при конвертации (panic)",
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

        // Record every created DDS (also after cancel) so rollback can find all of them.
        if let Ok(list) = created.lock() {
            if let Err(e) = merge_manifest(&manifest_path(), &list) {
                let _ = tx.send(OptimizationMsg::FileError(format!(
                    "Не удалось записать журнал DDS {}: {}",
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
}

pub fn texture_summary(mod_dirs: &[PathBuf], force: bool) -> TextureSummary {
    let manifest = manifest_index(read_manifest(&manifest_path()));
    let plan = plan_textures(mod_dirs, force, &manifest);
    TextureSummary {
        to_convert: plan.to_convert.len(),
        up_to_date: plan.up_to_date,
        foreign_dds: plan.foreign_dds,
    }
}

/// Runs `texture_summary` in a background thread (walking big mod folders takes a while).
pub fn texture_summary_async(mod_dirs: Vec<PathBuf>, force: bool, tx: mpsc::Sender<TextureSummary>) {
    std::thread::spawn(move || {
        let _ = tx.send(texture_summary(&mod_dirs, force));
    });
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RollbackResult {
    /// .dds files removed
    pub removed: usize,
    /// source PNG/JPG files restored from backup
    pub restored: usize,
    pub errors: usize,
}

/// Undoes texture optimization:
/// 1. every DDS listed in the manifest is removed and its backed-up source restored;
/// 2. legacy cleanup for DDS made by older versions (before the manifest existed):
///    any `.dds` inside a Textures folder that has a PNG/JPG twin — regardless of the
///    UI/Pawn/Apparel exclusions, which older versions did not have.
pub fn rollback_dds(mod_dirs: &[PathBuf]) -> RollbackResult {
    rollback_with_manifest(mod_dirs, &manifest_path())
}

pub fn rollback_with_manifest(mod_dirs: &[PathBuf], manifest: &Path) -> RollbackResult {
    let mut res = RollbackResult::default();
    let mut failed: Vec<ManifestEntry> = Vec::new();

    for entry in read_manifest(manifest) {
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
                match std::fs::remove_file(path) {
                    Ok(()) => res.removed += 1,
                    Err(_) => res.errors += 1,
                }
            }
        }
    }

    res
}

pub fn rollback_dds_async(mod_dirs: Vec<PathBuf>, tx: mpsc::Sender<RollbackResult>) {
    std::thread::spawn(move || {
        let res = rollback_dds(&mod_dirs);
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

        assert_eq!(optimize_single_texture(&transparent, opts).unwrap(), ConvertOutcome::Bc7);
        let (fourcc, mips) = dds_info(&transparent.with_extension("dds"));
        assert_eq!(fourcc, "DX10", "BC7 needs the DX10 header");
        assert_eq!(mips, 7);

        assert_eq!(optimize_single_texture(&odd, opts).unwrap(), ConvertOutcome::SkippedSize);
        assert!(!odd.with_extension("dds").exists(), "never stretched, left as PNG");
        assert_eq!(optimize_single_texture(&tiny, opts).unwrap(), ConvertOutcome::SkippedSmall);
        assert!(!dir.join("opaque.dds.tmp").exists());

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
            ManifestEntry { dds: ours_fresh.with_extension("dds"), source: ours_fresh.clone(), backup: None, source_fp: source_fingerprint(&ours_fresh) },
            ManifestEntry { dds: ours_stale.with_extension("dds"), source: ours_stale.clone(), backup: None, source_fp: Some("1:1".into()) },
        ];
        let manifest = manifest_index(entries);

        let plan = plan_textures(&[dir.join("Mod")], false, &manifest);
        let mut names: Vec<String> = plan.to_convert.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["b.png", "d.png"], "stale ours + brand new; fresh ours and foreign skipped");
        assert_eq!((plan.up_to_date, plan.foreign_dds), (1, 1));

        let forced = plan_textures(&[dir.join("Mod")], true, &manifest);
        assert_eq!(forced.to_convert.len(), 3, "force re-encodes ours, never the author's DDS");

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
            &[ManifestEntry { dds: dds.clone(), source: source.clone(), backup: Some(backup.clone()), source_fp: None }],
        )
        .unwrap();

        let res = rollback_with_manifest(&[dir.join("Mod")], &manifest);
        assert_eq!(res.restored, 1);
        assert_eq!(res.removed, 2);
        assert_eq!(res.errors, 0);
        assert!(source.is_file(), "source restored");
        assert!(!dds.exists() && !legacy_dds.exists(), "dds removed");
        assert!(!manifest.exists(), "manifest cleared");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
