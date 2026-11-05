//! Tags of mod sets ("Modpacks" tab): "stable", "test" and the like, plus the user's own.
//! Kept by set name in the program's data folder (`modlist_tags.json`), not in the game's
//! ModLists files, which the game rewrites.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Built-in tags, stored by these keys and shown translated.
pub const BUILT_IN: [&str; 4] = ["stable", "test", "performance", "roleplay"];

/// Longest own tag, in characters.
pub const MAX_LEN: usize = 24;

pub type Tags = HashMap<String, Vec<String>>;

pub fn tags_path() -> PathBuf {
    crate::game_install::manager_data_dir().join("modlist_tags.json")
}

pub fn load(path: &Path) -> Tags {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, tags: &Tags) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(tags).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// An own tag as typed: trimmed, inner spaces collapsed, cut to `MAX_LEN`; `None` when empty.
pub fn clean(raw: &str) -> Option<String> {
    let tag: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let tag: String = tag.chars().take(MAX_LEN).collect();
    let tag = tag.trim().to_string();
    (!tag.is_empty()).then_some(tag)
}

/// Turns `tag` on or off for a set; letter case does not matter. A set without tags
/// leaves the map.
pub fn toggle(tags: &mut Tags, set: &str, tag: &str) {
    let list = tags.entry(set.to_string()).or_default();
    if let Some(pos) = list.iter().position(|t| t.eq_ignore_ascii_case(tag)) {
        list.remove(pos);
    } else {
        list.push(tag.to_string());
    }
    if list.is_empty() {
        tags.remove(set);
    }
}

pub fn has(tags: &Tags, set: &str, tag: &str) -> bool {
    tags.get(set)
        .is_some_and(|l| l.iter().any(|t| t.eq_ignore_ascii_case(tag)))
}

/// Every tag in use with the number of sets, built-in ones first, then own ones by name.
/// `sets`: the names of the sets that exist (tags of deleted files are not counted).
pub fn in_use<'a>(tags: &Tags, sets: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for set in sets {
        for tag in tags.get(set).into_iter().flatten() {
            match counts.iter_mut().find(|(t, _)| t.eq_ignore_ascii_case(tag)) {
                Some((_, n)) => *n += 1,
                None => counts.push((tag.clone(), 1)),
            }
        }
    }
    let rank = |t: &str| BUILT_IN.iter().position(|b| *b == t).unwrap_or(BUILT_IN.len());
    counts.sort_by(|(a, _), (b, _)| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });
    counts
}

/// The tags follow a renamed set.
pub fn rename(tags: &mut Tags, from: &str, to: &str) {
    if let Some(list) = tags.remove(from) {
        tags.insert(to.to_string(), list);
    }
}

/// A copied set gets the same tags.
pub fn copy(tags: &mut Tags, from: &str, to: &str) {
    if let Some(list) = tags.get(from).cloned() {
        tags.insert(to.to_string(), list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_rename_copy_and_count() {
        let mut tags = Tags::new();
        toggle(&mut tags, "Run", "stable");
        toggle(&mut tags, "Run", "Medieval");
        toggle(&mut tags, "Test run", "test");
        toggle(&mut tags, "Test run", "medieval");
        assert!(has(&tags, "Run", "medieval"), "letter case does not matter");

        toggle(&mut tags, "Run", "MEDIEVAL");
        assert!(!has(&tags, "Run", "Medieval"));
        toggle(&mut tags, "Run", "Medieval");

        copy(&mut tags, "Run", "Run (copy)");
        rename(&mut tags, "Test run", "Trial");
        assert!(has(&tags, "Trial", "test") && !tags.contains_key("Test run"));

        let sets = ["Run", "Run (copy)", "Trial"];
        let used = in_use(&tags, sets.iter().copied());
        assert_eq!(
            used,
            vec![
                ("stable".to_string(), 2),
                ("test".to_string(), 1),
                ("Medieval".to_string(), 3)
            ]
        );

        toggle(&mut tags, "Trial", "test");
        toggle(&mut tags, "Trial", "medieval");
        assert!(!tags.contains_key("Trial"), "a set without tags leaves the map");
    }

    #[test]
    fn own_tags_are_cleaned() {
        assert_eq!(clean("  late   game  "), Some("late game".to_string()));
        assert_eq!(clean("   "), None);
        assert_eq!(clean(&"x".repeat(40)).unwrap().chars().count(), MAX_LEN);
    }
}
