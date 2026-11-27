//! File helpers shared by several modules.

use std::io;
use std::path::{Path, PathBuf};

/// Writes the file through a temporary one next to it and a rename, so a crash or a full
/// disk mid-write leaves the old file whole instead of a cut one.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A copy that failed, and on which file or folder.
#[derive(Debug)]
pub struct CopyError {
    pub path: PathBuf,
    pub error: io::Error,
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.error)
    }
}

impl From<CopyError> for io::Error {
    fn from(e: CopyError) -> Self {
        io::Error::new(e.error.kind(), e.to_string())
    }
}

/// Copies a folder with everything in it. Links and junctions are followed (a mod can be
/// one, e.g. SteamCMD's link in the Mods folder), except one that leads back into a folder
/// being copied: that would never end.
pub fn copy_dir(from: &Path, to: &Path) -> Result<(), CopyError> {
    copy_dir_inner(from, to, &mut Vec::new())
}

fn copy_dir_inner(from: &Path, to: &Path, open: &mut Vec<PathBuf>) -> Result<(), CopyError> {
    let at = |path: &Path| {
        let path = path.to_path_buf();
        move |error| CopyError { path, error }
    };
    let real = std::fs::canonicalize(from).map_err(at(from))?;
    if open.contains(&real) {
        return Ok(());
    }
    open.push(real);
    std::fs::create_dir_all(to).map_err(at(to))?;
    for entry in std::fs::read_dir(from).map_err(at(from))? {
        let entry = entry.map_err(at(from))?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir_inner(&src, &dst, open)?;
        } else {
            std::fs::copy(&src, &dst).map_err(at(&src))?;
        }
    }
    open.pop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rwmm_fs_util_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temp() {
        let dir = temp("atomic");
        let file = dir.join("state.json");
        write_atomic(&file, "old").unwrap();
        write_atomic(&file, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert!(!dir.join("state.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_is_copied_with_subfolders() {
        let dir = temp("copy");
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("Textures").join("Things")).unwrap();
        std::fs::write(src.join("About.xml"), "about").unwrap();
        std::fs::write(src.join("Textures").join("Things").join("a.png"), "png").unwrap();
        copy_dir(&src, &dir.join("dst")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("dst").join("About.xml")).unwrap(),
            "about"
        );
        assert!(dir.join("dst").join("Textures").join("Things").join("a.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_names_itself_in_the_error() {
        let dir = temp("missing");
        let gone = dir.join("gone");
        let e = copy_dir(&gone, &dir.join("dst")).unwrap_err();
        assert_eq!(e.path, gone);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A junction inside the mod that points back at the mod itself (`mklink /J`, no admin
    /// rights needed) is copied once, not forever.
    #[cfg(windows)]
    #[test]
    fn a_link_back_into_the_folder_does_not_loop() {
        let dir = temp("loop");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("About.xml"), "about").unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(src.join("self"))
            .arg(&src)
            .output()
            .is_ok_and(|o| o.status.success());
        if made {
            copy_dir(&src, &dir.join("dst")).unwrap();
            assert!(dir.join("dst").join("About.xml").exists());
            assert!(!dir.join("dst").join("self").join("self").exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
