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

// ───────────────────────── Messages ─────────────────────────

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
    Done {
        processed: usize,
        skipped: usize,
        errors: usize,
        deleted_sources: usize,
    },
    /// Optimization was cancelled by user
    Cancelled,
}

// ───────────────────── Texture Scanning ─────────────────────

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

/// Check if a .dds file already exists and is newer than the source image.
pub fn has_newer_dds(source: &Path) -> bool {
    let dds_path = source.with_extension("dds");
    if !dds_path.exists() {
        return false;
    }

    let src_modified = source
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let dds_modified = dds_path
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    dds_modified >= src_modified
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

/// Scan mod directories for texture files (PNG/JPG) that need conversion.
pub fn scan_textures_opt(mod_dirs: &[PathBuf], force: bool) -> Vec<PathBuf> {
    let mut textures: Vec<PathBuf> = Vec::new();

    for mod_dir in mod_dirs {
        for path in walk_mod_textures(mod_dir) {
            if force || !has_newer_dds(&path) {
                textures.push(path);
            }
        }
    }

    textures
}

pub fn scan_textures(mod_dirs: &[PathBuf]) -> Vec<PathBuf> {
    scan_textures_opt(mod_dirs, false)
}

// ─────────────────── Single File Conversion ─────────────────

/// Convert a single PNG/JPG file to DDS BC7 with automatic mipmaps.
pub fn optimize_single_texture(source: &Path) -> Result<(), String> {
    let mut img = image::open(source)
        .map_err(|e| format!("{}: {}", source.display(), e))?
        .flipv();

    let w = img.width();
    let h = img.height();

    fn next_pot(mut n: u32) -> u32 {
        if n == 0 { return 1; }
        n -= 1;
        n |= n >> 1;
        n |= n >> 2;
        n |= n >> 4;
        n |= n >> 8;
        n |= n >> 16;
        n + 1
    }

    let pot_w = next_pot(w);
    let pot_h = next_pot(h);

    if w != pot_w || h != pot_h || w < 4 || h < 4 {
        let new_w = std::cmp::max(4, pot_w);
        let new_h = std::cmp::max(4, pot_h);
        
        // In RimWorld, resizing textures exactly to POT is generally acceptable 
        // for most apparel/items as they scale smoothly.
        img = img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);
    }

    let rgba = img.to_rgba8();

    let dds = image_dds::dds_from_image(
        &rgba,
        image_dds::ImageFormat::BC3RgbaUnorm,
        image_dds::Quality::Normal,
        image_dds::Mipmaps::Disabled,
    )
    .map_err(|e| format!("{}: encode error: {}", source.display(), e))?;

    let mut dds_bytes = Vec::new();
    dds.write(&mut dds_bytes)
        .map_err(|e| format!("{}: buffer write error: {}", source.display(), e))?;

    // Fix DX10 header to legacy DXT5 (128 bytes)
    // image_dds generates a DX10 header by default for BC3RgbaUnorm.
    // RimWorld (and Graphics Settings) strictly rejects DX10 headers for DXT5.
    if dds_bytes.len() >= 148 && &dds_bytes[84..88] == b"DX10" {
        // Change FourCC from DX10 to DXT5
        dds_bytes[84] = b'D';
        dds_bytes[85] = b'X';
        dds_bytes[86] = b'T';
        dds_bytes[87] = b'5';
        
        // Remove the 20-byte DX10 extended header (bytes 128 to 147)
        let mut fixed = Vec::with_capacity(dds_bytes.len() - 20);
        fixed.extend_from_slice(&dds_bytes[0..128]);
        fixed.extend_from_slice(&dds_bytes[148..]);
        dds_bytes = fixed;
    }

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

    Ok(())
}

// ─────────────────── Manifest & source backups ──────────────────

/// One DDS created by the optimizer. `backup` is where the source PNG/JPG was moved, if it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub dds: PathBuf,
    pub source: PathBuf,
    pub backup: Option<PathBuf>,
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
            if dds.is_empty() || source.is_empty() {
                return None;
            }
            Some(ManifestEntry {
                dds: PathBuf::from(dds),
                source: PathBuf::from(source),
                backup: backup.map(PathBuf::from),
            })
        })
        .collect()
}

fn manifest_line(e: &ManifestEntry) -> String {
    format!(
        "{}\t{}\t{}\n",
        e.dds.display(),
        e.source.display(),
        e.backup.as_ref().map(|b| b.display().to_string()).unwrap_or_default()
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

// ─────────────────── Background Worker ──────────────────────

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
    max_threads: usize,
    tx: mpsc::Sender<OptimizationMsg>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let textures = scan_textures_opt(&mod_dirs, force);
        let total = textures.len();

        if total == 0 {
            let _ = tx.send(OptimizationMsg::Done {
                processed: 0,
                skipped: 0,
                errors: 0,
                deleted_sources: 0,
            });
            return;
        }

        use std::sync::atomic::AtomicUsize;
        let done_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));
        let deleted_count = Arc::new(AtomicUsize::new(0));
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

                let conv_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    optimize_single_texture(path)
                }));

                match conv_res {
                    Ok(Ok(())) => {
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
            if let Err(e) = append_manifest(&manifest_path(), &list) {
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
            let proc = done_count.load(Ordering::Relaxed).saturating_sub(error_count.load(Ordering::Relaxed));
            let del = deleted_count.load(Ordering::Relaxed);
            let errs = error_count.load(Ordering::Relaxed);
            let _ = tx.send(OptimizationMsg::Done {
                processed: proc,
                skipped: 0,
                errors: errs,
                deleted_sources: del,
            });
        }
    });
}

pub fn count_textures_preview_opt(mod_dirs: &[PathBuf], force: bool) -> (usize, usize) {
    let mut needs_optimization = 0usize;
    let mut already_optimized = 0usize;

    for mod_dir in mod_dirs {
        for entry in WalkDir::new(mod_dir)
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
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if !is_in_textures_folder(path) || is_in_about_folder(path) {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if ext == "png" || ext == "jpg" || ext == "jpeg" {
                let has_dds = has_newer_dds(path);
                if has_dds && !force {
                    already_optimized += 1;
                } else {
                    needs_optimization += 1;
                    if has_dds {
                        already_optimized += 1;
                    }
                }
            } else if ext == "dds" {
                let has_png = path.with_extension("png").exists();
                let has_jpg = path.with_extension("jpg").exists();
                let has_jpeg = path.with_extension("jpeg").exists();
                if !has_png && !has_jpg && !has_jpeg {
                    already_optimized += 1;
                }
            }
        }
    }

    (needs_optimization, already_optimized)
}

pub fn count_textures_preview(mod_dirs: &[PathBuf]) -> (usize, usize) {
    count_textures_preview_opt(mod_dirs, false)
}

pub fn count_textures_preview_async(
    mod_dirs: Vec<PathBuf>,
    force: bool,
    tx: mpsc::Sender<(usize, usize)>,
) {
    std::thread::spawn(move || {
        let res = count_textures_preview_opt(&mod_dirs, force);
        let _ = tx.send(res);
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
            &[ManifestEntry { dds: dds.clone(), source: source.clone(), backup: Some(backup.clone()) }],
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
