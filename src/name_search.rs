//! D1, part 2: missing mods with no Workshop link (not in the game's file, not in the Workshop
//! database) are looked up in the Workshop by name. Only an exact title match that is the
//! only one is ticked; everything else is shown for the user to pick, since a similar title
//! can be another upload or a replacement.

use crate::workshop_search::WorkshopItemResult;
use std::collections::HashSet;

/// Results shown per mod at most.
const MAX_MATCHES: usize = 5;

/// A missing mod to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub package_id: String,
    pub name: String,
    /// Workshop items the database lists for this package ID (several uploads, no clear pick).
    pub known: Vec<String>,
}

/// A Workshop item that may be the missing mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameMatch {
    pub id: String,
    pub title: String,
    /// The title is the mod's name (case, spaces, punctuation and version tags aside).
    pub exact: bool,
    /// The database lists this item under the mod's package ID.
    pub known: bool,
    /// Already installed (then it's another mod with the same title, or it needs a rescan).
    pub installed: bool,
}

/// One mod of the search window.
#[derive(Debug, Clone)]
pub struct NameSearchRow {
    pub wanted: Wanted,
    /// None while not searched yet.
    pub matches: Option<Vec<NameMatch>>,
    pub failed: bool,
}

/// The search window: rows, progress, ticks.
#[derive(Debug, Clone, Default)]
pub struct NameSearch {
    /// The set or save the mods come from.
    pub source: String,
    pub rows: Vec<NameSearchRow>,
    pub done: usize,
    pub running: bool,
    pub checked: HashSet<String>,
    /// Sent to Steam already.
    pub subscribed: HashSet<String>,
}

impl NameSearch {
    pub fn new(source: String, wanted: Vec<Wanted>) -> Self {
        Self {
            source,
            rows: wanted
                .into_iter()
                .map(|wanted| NameSearchRow {
                    wanted,
                    matches: None,
                    failed: false,
                })
                .collect(),
            running: true,
            ..Default::default()
        }
    }

    /// Stores a mod's results and ticks the clear pick, if there is one.
    pub fn set_matches(&mut self, row: usize, matches: Vec<NameMatch>) {
        let Some(r) = self.rows.get_mut(row) else { return };
        if let Some(id) = default_pick(&matches) {
            self.checked.insert(id.to_string());
        }
        r.matches = Some(matches);
        self.done += 1;
    }

    pub fn set_failed(&mut self, row: usize) {
        let Some(r) = self.rows.get_mut(row) else { return };
        r.failed = true;
        r.matches = Some(Vec::new());
        self.done += 1;
    }

    /// Mods for which something similar was found.
    pub fn found(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.matches.as_ref().is_some_and(|m| !m.is_empty()))
            .count()
    }
}

/// The name without version tags like "[1.6]", "(1.4-1.5)" or a trailing "v1.1" / "1.6"
/// (a trailing number without a dot stays: "Medieval 2" is a name).
pub fn clean_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find(['[', '(']) {
        let close = if rest[start..].starts_with('[') { ']' } else { ')' };
        let Some(len) = rest[start..].find(close) else { break };
        let inner = &rest[start + 1..start + len];
        out.push_str(&rest[..start]);
        if !is_version_tag(inner) {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    let mut words: Vec<&str> = out.split_whitespace().collect();
    while words.len() > 1 && words.last().is_some_and(|w| w.contains('.') && is_version_tag(w)) {
        words.pop();
    }
    words.join(" ")
}

fn is_version_tag(inner: &str) -> bool {
    let inner = inner.trim();
    inner.chars().any(|c| c.is_ascii_digit())
        && inner
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | ',' | '/' | ' ' | 'v' | 'V' | '+'))
}

/// Letters and digits only, lowercase: "Mya's fufu [1.6]" -> "myasfufu".
pub fn title_key(s: &str) -> String {
    clean_name(s)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(s: &str) -> Vec<String> {
    clean_name(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// The title looks like the mod: one contains the other, or most of the name's words are there.
fn related(title: &str, name: &str) -> bool {
    let (t, n) = (title_key(title), title_key(name));
    if t.is_empty() || n.is_empty() {
        return false;
    }
    if t.contains(&n) || (n.contains(&t) && t.chars().count() >= 4) {
        return true;
    }
    let name_words = words(name);
    if name_words.is_empty() {
        return false;
    }
    let title_words: HashSet<String> = words(title).into_iter().collect();
    let shared = name_words.iter().filter(|w| title_words.contains(*w)).count();
    shared * 2 >= name_words.len() && shared > 0
}

/// The Workshop results that may be the mod, best first: exact titles, then the items the
/// database knows, then the rest in Steam's order. Unrelated results are dropped.
pub fn matches_for(raw: Vec<WorkshopItemResult>, wanted: &Wanted, installed: &HashSet<String>) -> Vec<NameMatch> {
    let name_key = title_key(&wanted.name);
    let mut seen = HashSet::new();
    let mut out: Vec<NameMatch> = raw
        .into_iter()
        .filter(|item| seen.insert(item.id.clone()))
        .filter_map(|item| {
            let known = wanted.known.contains(&item.id);
            let exact = !name_key.is_empty() && title_key(&item.title) == name_key;
            (exact || known || related(&item.title, &wanted.name)).then(|| NameMatch {
                installed: installed.contains(&item.id),
                id: item.id,
                title: item.title,
                exact,
                known,
            })
        })
        .collect();
    // Stable: Steam's order is kept inside each group.
    out.sort_by_key(|m| (!m.exact, !m.known));
    out.truncate(MAX_MATCHES);
    out
}

/// The item ticked by itself: the only exact title that isn't installed.
pub fn default_pick(matches: &[NameMatch]) -> Option<&str> {
    let mut exact = matches.iter().filter(|m| m.exact);
    match (exact.next(), exact.next()) {
        (Some(m), None) if !m.installed => Some(&m.id),
        _ => None,
    }
}

/// The Workshop search page for a name (to look further by hand).
pub fn browse_url(name: &str) -> String {
    let query: String = clean_name(name)
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://steamcommunity.com/workshop/browse/?appid=294100&searchtext={query}&browsesort=textsearch")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str) -> WorkshopItemResult {
        WorkshopItemResult {
            id: id.into(),
            title: title.into(),
            preview_url: None,
            tags: Vec::new(),
            is_translation: false,
            score: 0,
        }
    }

    fn wanted(name: &str, known: &[&str]) -> Wanted {
        Wanted {
            package_id: "some.mod".into(),
            name: name.into(),
            known: known.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn version_tags_are_ignored() {
        assert_eq!(clean_name("NewRatkinPlus[1.6]"), "NewRatkinPlus");
        assert_eq!(clean_name("[1.5-1.6] Mod (v1.2)  name"), "Mod name");
        assert_eq!(clean_name("Mod (Continued)"), "Mod (Continued)");
        assert_eq!(clean_name("Dev In Your Language RU 1.6"), "Dev In Your Language RU");
        assert_eq!(clean_name("米莉拉角色拓展 v1.1"), "米莉拉角色拓展");
        assert_eq!(
            clean_name("Vanilla Factions Expanded - Medieval 2"),
            "Vanilla Factions Expanded - Medieval 2"
        );
        assert_eq!(clean_name("1.6"), "1.6");
        assert_eq!(title_key("Mya's fufu [1.6]"), "myasfufu");
        assert_eq!(title_key("虫娘 Insect Girls"), "虫娘insectgirls");
    }

    #[test]
    fn exact_title_first_and_ticked() {
        let raw = vec![
            item("1", "Wolfein Race Facial Animation"),
            item("2", "Wolfein Race [1.6]"),
            item("3", "Totally different"),
        ];
        let m = matches_for(raw, &wanted("Wolfein Race", &[]), &HashSet::new());
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["2", "1"]);
        assert!(m[0].exact && !m[1].exact);
        assert_eq!(default_pick(&m), Some("2"));
    }

    #[test]
    fn two_exact_titles_are_not_ticked() {
        let raw = vec![item("818773962", "HugsLib"), item("3513789221", "HugsLib")];
        let m = matches_for(raw, &wanted("HugsLib", &["3513789221", "818773962"]), &HashSet::new());
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|m| m.exact && m.known));
        assert_eq!(default_pick(&m), None, "no guessing between uploads");
    }

    #[test]
    fn installed_item_is_not_ticked() {
        let raw = vec![item("5", "Adaptive TPS")];
        let installed: HashSet<String> = ["5".to_string()].into();
        let m = matches_for(raw, &wanted("Adaptive TPS", &[]), &installed);
        assert!(m[0].installed);
        assert_eq!(default_pick(&m), None);
    }

    #[test]
    fn similar_titles_are_offered_unrelated_dropped() {
        let raw = vec![
            item("1", "Combat Extended Guns"),
            item("2", "Vanilla Expanded Framework"),
            item("3", "Combat Extended (Continued)"),
        ];
        let m = matches_for(raw, &wanted("Combat Extended", &[]), &HashSet::new());
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        assert_eq!(default_pick(&m), None);
    }

    #[test]
    fn search_progress() {
        let mut s = NameSearch::new("Save".into(), vec![wanted("A mod", &[]), wanted("B mod", &[])]);
        s.set_matches(
            0,
            vec![NameMatch {
                id: "7".into(),
                title: "A mod".into(),
                exact: true,
                known: false,
                installed: false,
            }],
        );
        s.set_failed(1);
        assert_eq!((s.done, s.found()), (2, 1));
        assert!(s.checked.contains("7"));
    }

    /// Searches the Workshop for the mods of the user's saves that neither the save nor the
    /// saved Workshop ID index can link (run by hand:
    /// `cargo test --release live_name_search -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_name_search() {
        let index = crate::workshop_db::load().unwrap_or_default();
        let saves_dir = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap())
            .join(r"AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Saves");
        let mut seen = HashSet::new();
        let (mut total, mut ticked, mut offered) = (0, 0, 0);
        for save in crate::saves::list_saves(&saves_dir) {
            for id in &save.ids {
                let lower = id.to_lowercase();
                if lower.starts_with("ludeon.rimworld")
                    || save.steam_id(id).is_some()
                    || index.lookup(id, save.name_of(id)).is_some()
                    || !seen.insert(lower)
                {
                    continue;
                }
                let w = wanted(save.name_of(id).unwrap_or(id), &[]);
                let raw = crate::workshop_search::search_steam_workshop(&clean_name(&w.name)).unwrap_or_default();
                let m = matches_for(raw, &w, &HashSet::new());
                total += 1;
                if default_pick(&m).is_some() {
                    ticked += 1;
                } else if !m.is_empty() {
                    offered += 1;
                }
                let first = m
                    .first()
                    .map(|m| format!("{} [{}]{}", m.title, m.id, if m.exact { " exact" } else { "" }));
                println!("{} -> {} found, first: {:?}", w.name, m.len(), first);
            }
        }
        println!(
            "{total} mods: ticked {ticked}, offered {offered}, nothing {}",
            total - ticked - offered
        );
    }

    #[test]
    fn browse_url_encodes_the_name() {
        assert_eq!(
            browse_url("Mya's fufu [1.6]"),
            "https://steamcommunity.com/workshop/browse/?appid=294100&searchtext=Mya%27s+fufu&browsesort=textsearch"
        );
    }
}
