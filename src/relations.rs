//! Mods related to the selected one, highlighted in both lists: its translations and add-ons
//! (or, for a translation or an add-on, the mod it belongs to), the mods it needs and the mods
//! that need it. Helps to find a translation or an add-on and its mod by hand, even when they
//! declare no dependency.

use crate::models::ModInfo;
use std::collections::HashMap;

/// What a highlighted mod is to the selected one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// A translation of the selected mod.
    Translation,
    /// An add-on of the selected mod: its name carries the mod's name ("Wolfein Race Gene Patch").
    Addon,
    /// The mod the selected translation or add-on belongs to.
    Original,
    /// A mod the selected one needs.
    Dependency,
    /// A mod that needs the selected one.
    Dependent,
}

/// Names shorter than this aren't matched inside other names ("UI", "Core"...).
const MIN_NAME: usize = 5;

fn has_id(list: &[String], id: &str) -> bool {
    list.iter().any(|x| x.eq_ignore_ascii_case(id))
}

/// A translation of `original`: says so (`parent_package_id`), loads after it, or carries its
/// name ("Vanilla Expanded Framework Russian Language Pack").
fn translates(tr: &ModInfo, original: &ModInfo) -> bool {
    if !tr.is_translation_mod || original.is_translation_mod {
        return false;
    }
    let id = &original.package_id;
    tr.parent_package_id
        .as_deref()
        .is_some_and(|p| p.eq_ignore_ascii_case(id))
        || has_id(&tr.load_after, id)
        || has_id(&tr.dependencies, id)
        || (original.name_lower.chars().count() >= MIN_NAME && tr.name_lower.contains(&original.name_lower))
}

/// An add-on of `base`: another (non-translation) mod whose name carries the base's name.
/// Not by loadAfter: hundreds of mods load after Harmony.
fn extends(addon: &ModInfo, base: &ModInfo) -> bool {
    !addon.is_translation_mod
        && !base.is_translation_mod
        && !addon.package_id.eq_ignore_ascii_case(&base.package_id)
        && base.name_lower.chars().count() >= MIN_NAME
        && addon.name_lower.contains(&base.name_lower)
}

/// Lowercase package ID -> how the mod relates to `selected`. A stronger relation wins
/// (translation over dependency); the selected mod itself isn't in the map.
pub fn related_mods(selected: &ModInfo, all: &[ModInfo]) -> HashMap<String, Relation> {
    let selected_key = selected.package_id.to_lowercase();
    let mut out: HashMap<String, Relation> = HashMap::new();
    let mut put = |m: &ModInfo, r: Relation| {
        let key = m.package_id.to_lowercase();
        if key != selected_key {
            out.entry(key).or_insert(r);
        }
    };

    if selected.is_translation_mod {
        // The mod it translates: the declared parent first, else the longest name it contains.
        let declared: Vec<&ModInfo> = all.iter().filter(|m| translates(selected, m)).collect();
        let original = declared
            .iter()
            .find(|m| {
                selected
                    .parent_package_id
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(&m.package_id))
            })
            .or_else(|| declared.iter().max_by_key(|m| m.name_lower.chars().count()));
        if let Some(m) = original {
            put(m, Relation::Original);
        }
    } else {
        for m in all.iter().filter(|m| translates(m, selected)) {
            put(m, Relation::Translation);
        }
        for m in all.iter().filter(|m| extends(m, selected)) {
            put(m, Relation::Addon);
        }
        // An add-on itself: the mod whose name it carries (the longest such name).
        if let Some(base) = all
            .iter()
            .filter(|m| extends(selected, m))
            .max_by_key(|m| m.name_lower.chars().count())
        {
            put(base, Relation::Original);
        }
    }
    for m in all {
        if has_id(&selected.dependencies, &m.package_id) {
            put(m, Relation::Dependency);
        }
    }
    for m in all {
        if has_id(&m.dependencies, &selected.package_id) {
            put(m, Relation::Dependent);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, name: &str) -> ModInfo {
        ModInfo {
            package_id: id.into(),
            name: name.into(),
            name_lower: name.to_lowercase(),
            ..Default::default()
        }
    }

    fn tr(id: &str, name: &str) -> ModInfo {
        ModInfo {
            is_translation_mod: true,
            ..m(id, name)
        }
    }

    #[test]
    fn a_mod_shows_its_translations_dependencies_and_dependents() {
        let mut vef = m(
            "oskarpotocki.vanillafactionsexpanded.core",
            "Vanilla Expanded Framework",
        );
        vef.dependencies = vec!["brrainz.harmony".into()];
        let harmony = m("brrainz.harmony", "Harmony");
        let by_name = tr("ow.ru.core", "Vanilla Expanded Framework Russian Language Pack");
        let mut by_parent = tr("x.ru", "VEF RU");
        by_parent.parent_package_id = Some("OskarPotocki.VanillaFactionsExpanded.Core".into());
        let mut needs_vef = m("vre.android", "Vanilla Races Expanded - Android");
        needs_vef.dependencies = vec!["oskarpotocki.vanillafactionsexpanded.core".into()];
        let unrelated = tr("other.ru", "Some Other Mod RU");
        let all = vec![vef.clone(), harmony, by_name, by_parent, needs_vef, unrelated];

        let rel = related_mods(&vef, &all);
        assert_eq!(rel.get("ow.ru.core"), Some(&Relation::Translation));
        assert_eq!(rel.get("x.ru"), Some(&Relation::Translation));
        assert_eq!(rel.get("brrainz.harmony"), Some(&Relation::Dependency));
        assert_eq!(rel.get("vre.android"), Some(&Relation::Dependent));
        assert!(!rel.contains_key("other.ru"));
        assert!(!rel.contains_key("oskarpotocki.vanillafactionsexpanded.core"));
    }

    #[test]
    fn a_translation_shows_the_mod_it_translates() {
        let vef = m("vef.core", "Vanilla Expanded Framework");
        let vanilla = m("vanilla", "Vanilla");
        let pack = tr("ow.ru.core", "Vanilla Expanded Framework Russian Language Pack");
        let all = vec![vef, vanilla, pack.clone()];
        let rel = related_mods(&pack, &all);
        assert_eq!(rel.get("vef.core"), Some(&Relation::Original), "the longest name wins");
        assert!(!rel.contains_key("vanilla"));
    }

    #[test]
    fn add_ons_and_their_mod() {
        let race = m("melondove.wolfeinrace", "Wolfein Race");
        let fa = m("melondove.wolfeinrace.fa", "Wolfein Race Facial Animation");
        let genes = m("ancot.wolfeinracegenepatch", "Wolfein Race Gene Patch");
        let ru = tr("nanamy.wolfeinraceru", "Wolfein Race (Russian language)");
        let other = m("other", "Rimworld Race Pack");
        let all = vec![race.clone(), fa.clone(), genes, ru, other];

        let rel = related_mods(&race, &all);
        assert_eq!(rel.get("melondove.wolfeinrace.fa"), Some(&Relation::Addon));
        assert_eq!(rel.get("ancot.wolfeinracegenepatch"), Some(&Relation::Addon));
        assert_eq!(rel.get("nanamy.wolfeinraceru"), Some(&Relation::Translation));
        assert!(!rel.contains_key("other"));

        assert_eq!(
            related_mods(&fa, &all).get("melondove.wolfeinrace"),
            Some(&Relation::Original)
        );
    }

    #[test]
    fn a_translation_that_loads_after_its_mod() {
        let achtung = m("brrainz.achtung", "Achtung!");
        let mut ru = tr("ru.pashka.brrainz.achtung", "Achtung! Russian Language");
        ru.load_after = vec!["Brrainz.Achtung".into()];
        let all = vec![achtung.clone(), ru.clone()];
        assert_eq!(
            related_mods(&ru, &all).get("brrainz.achtung"),
            Some(&Relation::Original)
        );
        assert_eq!(
            related_mods(&achtung, &all).get("ru.pashka.brrainz.achtung"),
            Some(&Relation::Translation)
        );
    }
}
