//! Colour marks of mods: the player gives a mod one of six colours, filters the lists by
//! it and acts on a whole colour at once (a "group"). Kept in `mod_colors.json` in the
//! program's data folder: lowercase package ID -> colour.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ModColor {
    pub const ALL: [ModColor; 6] = [
        ModColor::Red,
        ModColor::Orange,
        ModColor::Yellow,
        ModColor::Green,
        ModColor::Blue,
        ModColor::Purple,
    ];

    /// Readable on the dark and the light theme.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            ModColor::Red => [229, 72, 77],
            ModColor::Orange => [240, 140, 40],
            ModColor::Yellow => [222, 190, 40],
            ModColor::Green => [70, 180, 90],
            ModColor::Blue => [60, 140, 230],
            ModColor::Purple => [160, 100, 220],
        }
    }

    pub fn key(self) -> crate::i18n::TranslationKey {
        use crate::i18n::TranslationKey as K;
        match self {
            ModColor::Red => K::ColorRed,
            ModColor::Orange => K::ColorOrange,
            ModColor::Yellow => K::ColorYellow,
            ModColor::Green => K::ColorGreen,
            ModColor::Blue => K::ColorBlue,
            ModColor::Purple => K::ColorPurple,
        }
    }
}

pub type Marks = HashMap<String, ModColor>;

pub fn marks_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("mod_colors.json")
}

/// Marks in the file (keys in lower case); none when it's missing or unreadable.
pub fn load(path: &Path) -> Marks {
    let marks: Marks = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    marks.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect()
}

pub fn save(path: &Path, marks: &Marks) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(marks).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Gives `ids` the colour `color`, or takes their colour away (`None`).
pub fn set(marks: &mut Marks, ids: &[String], color: Option<ModColor>) {
    for id in ids {
        let key = id.to_lowercase();
        match color {
            Some(c) => marks.insert(key, c),
            None => marks.remove(&key),
        };
    }
}

/// How many of `ids` have each colour, in the order of `ModColor::ALL`; unused colours left out.
pub fn counts<'a>(marks: &Marks, ids: impl Iterator<Item = &'a str>) -> Vec<(ModColor, usize)> {
    let mut n: HashMap<ModColor, usize> = HashMap::new();
    for id in ids {
        if let Some(c) = marks.get(&id.to_lowercase()) {
            *n.entry(*c).or_insert(0) += 1;
        }
    }
    ModColor::ALL
        .iter()
        .filter_map(|c| n.get(c).map(|count| (*c, *count)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_set_counted_stored_and_removed() {
        let dir = std::env::temp_dir().join(format!("rmm_mod_colors_{}", std::process::id()));
        let path = dir.join("mod_colors.json");
        let mut marks = Marks::new();
        set(
            &mut marks,
            &["A.Mod".to_string(), "b.mod".to_string()],
            Some(ModColor::Green),
        );
        set(&mut marks, &["c.mod".to_string()], Some(ModColor::Red));
        set(&mut marks, &["B.MOD".to_string()], Some(ModColor::Red));
        assert_eq!(marks.get("a.mod"), Some(&ModColor::Green), "keys in lower case");

        let ids = ["a.mod", "B.Mod", "c.mod", "d.mod"];
        assert_eq!(
            counts(&marks, ids.iter().copied()),
            vec![(ModColor::Red, 2), (ModColor::Green, 1)],
            "colour order, unused colours left out"
        );

        save(&path, &marks).unwrap();
        assert_eq!(load(&path), marks);
        set(&mut marks, &["a.mod".to_string()], None);
        assert!(!marks.contains_key("a.mod"));
        assert!(load(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
