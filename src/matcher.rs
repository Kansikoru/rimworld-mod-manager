use crate::models::{is_protected_package_id, ModInfo};
use std::collections::HashMap;

/// Main classification and linking method for RimWorld mods.
/// Evaluates folder structure, packageId patterns, title keywords, and loadAfter dependencies
/// to accurately distinguish base/content mods from translation mods.
pub fn classify_and_link_translations(mods: &mut [ModInfo]) {
    // 1. Build map of package_id -> index and cleaned_name -> package_id
    let mut id_to_index = HashMap::new();
    let mut name_to_id = HashMap::new();

    for (i, m) in mods.iter().enumerate() {
        let id_lower = m.package_id.to_lowercase();
        id_to_index.insert(id_lower.clone(), i);
        name_to_id.insert(clean_name(&m.name), id_lower);
    }

    // 2. Identify translation mods
    for m in mods.iter_mut() {
        if is_protected_package_id(&m.package_id) {
            m.is_translation_mod = false;
            m.parent_package_id = None;
            continue;
        }

        let is_trans = detect_is_translation(m);
        if is_trans {
            m.is_translation_mod = true;
            m.parent_package_id = find_parent_package_id(m, &id_to_index, &name_to_id);
        } else {
            m.is_translation_mod = false;
            m.parent_package_id = None;
        }
    }
}

/// Determines whether a mod is a pure translation/localization mod.
///
/// The result is used to offer deleting the mod, so the rules are strict:
/// a false "not a translation" is harmless, a false "translation" deletes a working mod.
/// 1. A mod with Defs/, Patches/, Textures/, Sounds/, AssetBundles/... is never a translation.
/// 2. A translation always ships a Languages/ folder.
/// 3. Without DLLs (only About/ + Languages/) it is a translation.
/// 4. With DLLs it is a translation only if the title says so explicitly
///    (translation, русификатор, 汉化, zh-pack) or the package ID is translation-tagged and has a keyword.
pub fn detect_is_translation(m: &ModInfo) -> bool {
    if m.has_defs || m.has_other_content {
        return false;
    }
    if m.languages.is_empty() {
        return false;
    }
    if !m.has_assemblies {
        return true;
    }

    let name_lower = m.name.to_lowercase();
    let id_lower = m.package_id.to_lowercase();
    crate::models::has_explicit_translation_title_marker(&name_lower)
        || (crate::models::is_package_id_translation(&id_lower)
            && crate::models::has_translation_keyword(&name_lower))
}

/// Finds the parent / mother mod that this translation mod translates.
pub fn find_parent_package_id(
    m: &ModInfo,
    id_to_index: &HashMap<String, usize>,
    name_to_id: &HashMap<String, String>,
) -> Option<String> {
    let id_lower = m.package_id.to_lowercase();

    // 0. Check modDependencies (excluding protected IDs)
    for dep in &m.dependencies {
        let dep_lower = dep.to_lowercase();
        if !is_protected_package_id(&dep_lower)
            && dep_lower != id_lower
            && id_to_index.contains_key(&dep_lower)
        {
            return Some(dep.clone());
        }
    }

    // 1. Check load_after & force_load_after
    for target in m.load_after.iter().chain(m.force_load_after.iter()) {
        let target_lower = target.to_lowercase();
        if !is_protected_package_id(&target_lower)
            && target_lower != id_lower
            && id_to_index.contains_key(&target_lower)
        {
            return Some(target.clone());
        }
    }

    // 2. Strip package ID suffixes (e.g. "latta.devl10n.zh" -> "latta.devl10n")
    for suffix in &[
        ".zh.hc",
        ".zh",
        ".ru",
        ".chs",
        ".cht",
        ".translation",
        "_zh",
        "_ru",
        ".trans",
        ".locale",
    ] {
        if let Some(stripped) = id_lower.strip_suffix(suffix) {
            if id_to_index.contains_key(stripped) && stripped != id_lower {
                return Some(stripped.to_string());
            }
        }
    }

    // 3. Strip package ID prefixes (e.g. "zh.rh2bcdfirstaid" -> "rh2.bcd.first.aid" via stripped check)
    for prefix in &["zh.", "rwzh.", "ru.", "chs."] {
        if let Some(stripped) = id_lower.strip_prefix(prefix) {
            // Direct stripped match
            if id_to_index.contains_key(stripped) && stripped != id_lower {
                return Some(stripped.to_string());
            }
            // Dot-ignoring match: compare alphanumeric
            let stripped_clean = clean_name(stripped);
            for existing_id in id_to_index.keys() {
                if existing_id != &id_lower && clean_name(existing_id) == stripped_clean {
                    return Some(existing_id.clone());
                }
            }
        }
    }

    // 4. Strip name suffixes (e.g. "Down For Me_zh" -> "Down For Me")
    let cleaned_name = extract_base_mod_name(&m.name);
    if let Some(parent_id) = name_to_id.get(&cleaned_name) {
        if parent_id != &id_lower {
            return Some(parent_id.clone());
        }
    }

    None
}

/// Normalizes a name by keeping only alphanumeric characters in lowercase.
pub fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

static BRACKET_TAG_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\[(ru|zh|cn|chs|cht|fr|de)\]|\((ru|zh|cn)\)").unwrap()
});

/// Extracts the candidate base mod title by stripping translation tags and suffixes.
pub fn extract_base_mod_name(name: &str) -> String {
    let mut s = BRACKET_TAG_RE.replace_all(name, "").into_owned();

    // Strip suffixes like _zh, _ru, CHS, Translation
    let lower = s.to_lowercase();
    for suffix in &[
        "_zh",
        "_ru",
        " chs",
        " cht",
        " zh",
        " ru",
        " translation",
        " перевод",
        " русификатор",
    ] {
        if let Some(idx) = lower.rfind(suffix) {
            s.truncate(idx);
            break;
        }
    }

    clean_name(&s)
}

static GENERIC_MOD_WORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "and",
    "or",
    "of",
    "for",
    "in",
    "on",
    "at",
    "to",
    "mod",
    "edition",
    "version",
    "pack",
    "framework",
    "expanded",
    "core",
    "repack",
    "updated",
    "continued",
    "unofficial",
    "addon",
];

static BRACKET_STRIP_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\[[^\]]*\]|\([^\)]*\)").unwrap());

/// Extracts core meaningful keywords from a mod name, ignoring stop words and bracket tags.
pub fn extract_key_tokens(name: &str) -> Vec<String> {
    let stripped = BRACKET_STRIP_RE.replace_all(name, " ");
    let words: Vec<String> = stripped
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();

    let meaningful: Vec<String> = words
        .iter()
        .filter(|w| w.len() >= 2 && !GENERIC_MOD_WORDS.contains(&w.as_str()))
        .cloned()
        .collect();

    if meaningful.is_empty() {
        words.into_iter().filter(|w| w.len() >= 2).collect()
    } else {
        meaningful
    }
}

/// Checks if a candidate translation title matches the base mod's title using token overlap.
pub fn matches_base_mod_title(base_mod_name: &str, candidate_title: &str) -> (bool, i32) {
    let base_lower = base_mod_name.trim().to_lowercase();
    let cand_lower = candidate_title.trim().to_lowercase();

    // Direct whole phrase match
    if cand_lower.contains(&base_lower) {
        return (true, 100);
    }

    let base_tokens = extract_key_tokens(base_mod_name);
    if base_tokens.is_empty() {
        return (false, 0);
    }

    let cand_tokens: std::collections::HashSet<String> = candidate_title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();

    let mut matched_count = 0;
    for t in &base_tokens {
        if cand_tokens.contains(t)
            || (t.len() >= 4 && cand_tokens.iter().any(|cw| cw.contains(t.as_str())))
        {
            matched_count += 1;
        }
    }

    let total = base_tokens.len();
    let match_ok = if total == 1 {
        matched_count == 1
    } else if total == 2 {
        matched_count == 2
    } else {
        (matched_count as f32 / total as f32) >= 0.65
    };

    if !match_ok {
        return (false, 0);
    }

    let mut score = 50;
    if matched_count == total {
        score += 30;
    }
    if !base_tokens.is_empty() && cand_lower.contains(&base_tokens[0]) {
        score += 20;
    }

    (true, score)
}

/// Checks whether the candidate Workshop item is simply the base mod itself.
pub fn is_candidate_base_mod_itself(
    candidate_title: &str,
    candidate_steam_id: &str,
    base_name: &str,
    base_steam_id: Option<&str>,
) -> bool {
    if let Some(base_sid) = base_steam_id {
        if !candidate_steam_id.is_empty() && base_sid == candidate_steam_id {
            return true;
        }
    }
    let cand_trim = candidate_title.trim();
    let base_trim = base_name.trim();
    if cand_trim.eq_ignore_ascii_case(base_trim) {
        return true;
    }
    false
}

/// Checks whether a title contains indicators of being a translation mod.
pub fn is_translation_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    crate::models::has_translation_keyword(&lower)
        || crate::models::is_mod_name_translation_tagged(&lower)
        || crate::models::has_ru_tag_or_keyword(&lower)
        || crate::models::has_zh_tag_or_keyword(&lower)
        || crate::models::has_de_tag_or_keyword(&lower)
        || crate::models::has_fr_tag_or_keyword(&lower)
        || lower.contains("übersetzung")
        || lower.contains("traduction")
        || crate::models::contains_cyrillic(title)
        // A language marker of any language on offer ("日本語", "[PL]", "Português").
        || crate::models::TranslationTargetLang::POPULAR
            .iter()
            .any(|lang| lang.matches_workshop_item(title, &[]))
}

/// Stricter name check for the modpack-wide search: the whole mod name is in the title
/// ("Vanilla Expanded Framework" in "Vanilla Expanded Framework - Russian"), or every
/// meaningful word of it is. Partial overlaps like "Alpha Beta Gamma" vs "Alpha Delta" don't count.
pub fn title_names_mod(base_mod_name: &str, candidate_title: &str) -> bool {
    let base = clean_name(base_mod_name);
    if base.len() >= 3 && clean_name(candidate_title).contains(&base) {
        return true;
    }
    // Every word counts here ("Expanded", "Framework" tell mods apart), only function words are skipped.
    const SKIP: &[&str] = &["the", "a", "an", "and", "or", "of", "for", "in", "on", "at", "to", "by"];
    let tokens: Vec<String> = BRACKET_STRIP_RE
        .replace_all(base_mod_name, " ")
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty() && !SKIP.contains(&w.as_str()))
        .collect();
    if tokens.is_empty() {
        return false;
    }
    let cand: std::collections::HashSet<String> = candidate_title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    tokens.iter().all(|t| cand.contains(t))
}

/// Translations of one mod for the modpack-wide search: the usual language and translation
/// checks, a strict name match, and nothing the user is already subscribed to.
pub fn translations_for_mod(
    items: Vec<crate::workshop_search::WorkshopItemResult>,
    base_name: &str,
    base_steam_id: Option<&str>,
    target_lang: crate::models::TranslationTargetLang,
    installed_steam_ids: &std::collections::HashSet<String>,
) -> Vec<crate::workshop_search::WorkshopItemResult> {
    filter_and_rank_workshop_translations(items, base_name, base_steam_id, target_lang)
        .into_iter()
        .filter(|item| title_names_mod(base_name, &item.title) && !installed_steam_ids.contains(&item.id))
        .collect()
}

/// Strictly filters and ranks Workshop translation items against a base mod and target language.
pub fn filter_and_rank_workshop_translations(
    items: Vec<crate::workshop_search::WorkshopItemResult>,
    base_name: &str,
    base_steam_id: Option<&str>,
    target_lang: crate::models::TranslationTargetLang,
) -> Vec<crate::workshop_search::WorkshopItemResult> {
    let mut valid_items = Vec::new();

    for mut item in items {
        // 1. Must not be the base mod itself
        if is_candidate_base_mod_itself(&item.title, &item.id, base_name, base_steam_id) {
            continue;
        }

        // 2. Must match the target language
        if !target_lang.matches_workshop_item(&item.title, &item.tags) {
            continue;
        }

        // 3. Must match the base mod title
        let (matches_name, mut score) = matches_base_mod_title(base_name, &item.title);
        if !matches_name {
            continue;
        }

        // 4. Must have translation indicators
        if !item.is_translation && !is_translation_title(&item.title) {
            continue;
        }

        // Add bonus score
        if item
            .tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case("Translation"))
        {
            score += 15;
        }
        let suffix = target_lang.search_suffix().to_lowercase();
        if !suffix.is_empty() && item.title.to_lowercase().contains(&suffix) {
            score += 15;
        }
        if crate::models::contains_cyrillic(&item.title)
            && target_lang == crate::models::TranslationTargetLang::Ru
        {
            score += 15;
        }

        item.score = score;
        valid_items.push(item);
    }

    // Sort descending by relevance score
    valid_items.sort_by_key(|b| std::cmp::Reverse(b.score));

    valid_items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModInfo, ModOrigin};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_test_mod(id: &str, name: &str) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            author: "Author".to_string(),
            description: "Desc".to_string(),
            dir_path: PathBuf::new(),
            preview_path: None,
            supported_versions: vec![],
            load_after: vec![],
            force_load_after: vec![],
            load_before: vec![],
            force_load_before: vec![],
            dependencies: vec![],
            origin: ModOrigin::Local,
            languages: vec![],
            is_translation_mod: false,
            parent_package_id: None,
            steam_item_id: None,
            has_defs: false,
            has_assemblies: false,
            has_other_content: false,
            ..Default::default()
        }
    }

    fn with_langs(mut m: ModInfo) -> ModInfo {
        m.languages = vec!["Russian".to_string()];
        m
    }

    #[test]
    fn test_detect_is_translation() {
        let m = with_langs(make_test_mod("foo.bar", "Foo Bar"));
        assert!(detect_is_translation(&m), "Pure language mod");

        let mut m = make_test_mod("foo.bar", "Foo Bar");
        m.has_defs = true;
        m.has_assemblies = true;
        assert!(!detect_is_translation(&m), "Normal content mod");

        let m = make_test_mod("foo.bar.ru", "Foo Bar 汉化");
        assert!(!detect_is_translation(&m), "No Languages folder -> never a translation");

        let m = with_langs(make_test_mod("foo.bar.ru", "Foo Bar"));
        assert!(detect_is_translation(&m), "Package ID suffix .ru, only Languages");

        let mut m = with_langs(make_test_mod("foo.bar.ru", "Foo Bar"));
        m.has_assemblies = true;
        assert!(!detect_is_translation(&m), "Package ID suffix .ru WITH assemblies (no kw)");

        m.name = "Foo Bar русификатор".to_string();
        assert!(detect_is_translation(&m), "Package ID suffix .ru WITH assemblies (with kw)");

        let m = with_langs(make_test_mod("foo.bar", "Foo Bar 汉化"));
        assert!(detect_is_translation(&m), "Keyword in name 汉化");

        let mut m = with_langs(make_test_mod("foo.bar.trans", "Foo Bar translation"));
        m.has_assemblies = true;
        assert!(detect_is_translation(&m), "Mod with DLL but translation in name");
    }

    #[test]
    fn content_mods_are_never_translations() {
        let mut patch_mod = with_langs(make_test_mod("someone.compatpatch", "Some Compatibility Patch"));
        patch_mod.has_other_content = true; // Patches/
        assert!(!detect_is_translation(&patch_mod), "Patches + Languages is a patch mod");

        let mut retexture = with_langs(make_test_mod("someone.retexture.ru", "Retexture [RU]"));
        retexture.has_other_content = true; // Textures/
        assert!(!detect_is_translation(&retexture), "Textures + Languages is a retexture");

        let mut defs_mod = with_langs(make_test_mod("someone.translation.tools", "Translation Tools"));
        defs_mod.has_defs = true;
        assert!(!detect_is_translation(&defs_mod), "Defs make it a content mod despite the name");
    }

    #[test]
    fn test_find_parent_package_id() {
        let mut id_to_index = HashMap::new();
        let mut name_to_id = HashMap::new();
        
        id_to_index.insert("foo.bar".to_string(), 0);
        name_to_id.insert("foobar".to_string(), "foo.bar".to_string());

        let mut m = make_test_mod("foo.bar.ru", "Foo Bar Ru");
        m.load_after = vec!["foo.bar".to_string()];
        assert_eq!(find_parent_package_id(&m, &id_to_index, &name_to_id), Some("foo.bar".to_string()), "Via loadAfter");

        let m = make_test_mod("foo.bar.ru", "Foo Bar Ru");
        assert_eq!(find_parent_package_id(&m, &id_to_index, &name_to_id), Some("foo.bar".to_string()), "Via suffix stripping");

        let m = make_test_mod("zh.foo.bar", "Foo Bar ZH");
        assert_eq!(find_parent_package_id(&m, &id_to_index, &name_to_id), Some("foo.bar".to_string()), "Via prefix stripping");

        let m = make_test_mod("unknown.mod.ru", "Unknown Mod");
        assert_eq!(find_parent_package_id(&m, &id_to_index, &name_to_id), None, "No match found");
    }

    fn item(id: &str, title: &str, tags: &[&str]) -> crate::workshop_search::WorkshopItemResult {
        crate::workshop_search::WorkshopItemResult {
            id: id.into(),
            title: title.into(),
            preview_url: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            is_translation: tags.contains(&"Translation"),
            score: 0,
        }
    }

    #[test]
    fn modpack_search_shows_only_this_mod_in_this_language() {
        use crate::models::TranslationTargetLang as L;
        let found = vec![
            item("1", "Vanilla Expanded Framework - Russian", &["Translation"]),
            item("2", "Vanilla Expanded Framework", &["Mod"]), // the mod itself
            item("3", "Vanilla Expanded Framework - German", &["Translation"]),
            item("4", "Vanilla Furniture Expanded [RU]", &["Translation"]), // another mod
            item("5", "Vanilla Expanded Framework (Русский перевод)", &[]),
            item("6", "VEF Russian", &["Translation"]), // abbreviation: not the mod name
            item("7", "Vanilla Expanded Framework RU", &["Translation"]), // already subscribed
            item("8", "Rus Vanilla Expanded Framework", &["Translation"]),
        ];
        let installed: std::collections::HashSet<String> = ["7".to_string()].into_iter().collect();
        let ids: Vec<String> = translations_for_mod(found, "Vanilla Expanded Framework", Some("2"), L::Ru, &installed)
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids.len(), 3, "{ids:?}");
        for id in ["1", "5", "8"] {
            assert!(ids.contains(&id.to_string()), "{id} missing: {ids:?}");
        }
    }

    /// Live check against the Steam Workshop: `cargo test live_modpack_search -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_modpack_search() {
        use crate::models::TranslationTargetLang as L;
        let none = std::collections::HashSet::new();
        for (name, lang) in [
            ("Dubs Bad Hygiene", L::Ru),
            ("Vanilla Expanded Framework", L::Ru),
            ("Humanoid Alien Races", L::Ru),
            ("RimHUD", L::De),
            ("Allow Tool", L::Zh),
        ] {
            let raw = crate::workshop_search::search_steam_workshop(&format!("{name} {}", lang.search_suffix())).unwrap();
            let total = raw.len();
            let kept = translations_for_mod(raw, name, None, lang, &none);
            println!("{name} [{}]: {total} found, kept {}", lang.label(), kept.len());
            for k in kept {
                println!("    {}  ({})", k.title, k.id);
            }
        }
    }

    #[test]
    fn languages_are_told_apart() {
        use crate::models::TranslationTargetLang as L;
        let is = |lang: L, title: &str| lang.matches_workshop_item(title, &[]);
        // Japanese uses Chinese characters too: kana decides.
        assert!(is(L::Ja, "Dubs Bad Hygiene 日本語化パッチ"));
        assert!(!is(L::Zh, "Dubs Bad Hygiene 日本語化パッチ"));
        assert!(is(L::Zh, "Dubs Bad Hygiene 简体中文汉化"));
        // Cyrillic alone is not Russian.
        assert!(is(L::Uk, "Dubs Bad Hygiene - Українська локалізація"));
        assert!(!is(L::Ru, "Dubs Bad Hygiene - Українська локалізація"));
        assert!(is(L::Ru, "Dubs Bad Hygiene - Русский перевод"));
        // Short codes count only as tags, not as English words.
        assert!(is(L::It, "Dubs Bad Hygiene [IT]"));
        assert!(!is(L::It, "Make it work: Dubs Bad Hygiene"));
        assert!(is(L::PtBr, "Dubs Bad Hygiene - Tradução PT-BR"));
        assert!(is(L::Pl, "Dubs Bad Hygiene - spolszczenie"));
        assert!(is(L::Ko, "Dubs Bad Hygiene 한국어 번역"));
        assert!(!is(L::Ko, "Dubs Bad Hygiene Russian"));
    }

    #[test]
    fn new_languages_find_their_translations() {
        use crate::models::TranslationTargetLang as L;
        let found = vec![
            item("1", "Vanilla Expanded Framework - Japanese Translation", &["Translation"]),
            item("2", "Vanilla Expanded Framework 日本語化", &[]),
            item("3", "Vanilla Expanded Framework - Russian", &["Translation"]),
        ];
        let none = std::collections::HashSet::new();
        let ids: Vec<String> = translations_for_mod(found, "Vanilla Expanded Framework", None, L::Ja, &none)
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(!ids.contains(&"3".to_string()));
    }

    #[test]
    fn title_must_name_the_whole_mod() {
        assert!(title_names_mod("Dubs Bad Hygiene", "Dubs Bad Hygiene [RU]"));
        assert!(title_names_mod("Dubs Bad Hygiene", "Перевод Dubs Bad Hygiene"));
        assert!(title_names_mod("Hygiene: Dubs Bad", "Dubs Bad Hygiene - Russian"));
        assert!(!title_names_mod("Dubs Bad Hygiene", "Dubs Mint Menus [RU]"));
        assert!(!title_names_mod("Alpha Beta Gamma", "Alpha Delta Epsilon [RU]"));
    }

    #[test]
    fn built_in_language_needs_no_translation() {
        use crate::models::TranslationTargetLang as L;
        let mut m = make_test_mod("a.b", "A B");
        m.languages = vec!["Russian (Русский)".into(), "German".into()];
        assert!(L::Ru.is_built_into(&m));
        assert!(L::De.is_built_into(&m));
        assert!(!L::Zh.is_built_into(&m));
    }

    #[test]
    fn test_matches_base_mod_title() {
        let (matches, score) = matches_base_mod_title("My Mod", "My Mod [RU]");
        assert!(matches);
        assert!(score >= 100);

        let (matches, score) = matches_base_mod_title("Super Weapon", "Weapon Super [ZH]");
        assert!(matches);
        assert!(score >= 80);

        let (matches, score) = matches_base_mod_title("Alpha Beta Gamma", "Alpha Delta Epsilon [RU]");
        assert!(!matches);
        assert_eq!(score, 0);
    }
}
