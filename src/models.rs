use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROTECTED_PACKAGE_IDS: &[&str] = &[
    "ludeon.rimworld", // Only Core is absolutely mandatory
];

pub fn is_protected_package_id(package_id: &str) -> bool {
    PROTECTED_PACKAGE_IDS
        .iter()
        .any(|&p| p.eq_ignore_ascii_case(package_id))
}

pub const OPTIMIZER_BLACKLIST: &[&str] = &[
    "brrainz.harmony",
    "unlimitedhugs.hugslib",
    "oskarpotocki.vanillafactionsexpanded.core",
    "ceteam.combatextended",
    "rwmt.multiplayer",
    "rwmt.multiplayercompatibility",
    "smashphil.vehicleframework",
];

pub fn is_optimizer_blacklisted(package_id: &str) -> bool {
    OPTIMIZER_BLACKLIST.iter().any(|&p| p.eq_ignore_ascii_case(package_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ModOrigin {
    Workshop,
    #[default]
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModInfo {
    pub package_id: String,
    pub name: String,
    #[serde(default)]
    pub name_lower: String,
    pub author: String,
    pub description: String,
    pub dir_path: PathBuf,
    pub preview_path: Option<PathBuf>,
    pub supported_versions: Vec<String>,
    pub load_after: Vec<String>,
    pub force_load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub force_load_before: Vec<String>,
    pub dependencies: Vec<String>,
    pub origin: ModOrigin,
    pub languages: Vec<String>,
    pub is_translation_mod: bool,
    pub parent_package_id: Option<String>,
    pub steam_item_id: Option<String>,
    pub has_defs: bool,
    pub has_assemblies: bool,
    /// The mod ships something besides About/ and Languages/
    /// (Patches, Textures, Sounds, AssetBundles, Shaders...). Such a mod is never a pure translation.
    #[serde(default)]
    pub has_other_content: bool,
    /// Has a Patches/ folder.
    #[serde(default)]
    pub has_patches: bool,
    /// Has a Textures/ folder.
    #[serde(default)]
    pub has_textures: bool,
    /// Counts of def types found in Defs/ (races, buildings, weapons...).
    #[serde(default)]
    pub def_stats: crate::categories::DefStats,
    /// Categories computed from the content (or set by the user), strongest first.
    #[serde(default)]
    pub categories: Vec<crate::categories::ModCategory>,
    /// `incompatibleWith` (+ the block for the installed game version).
    #[serde(default)]
    pub incompatible_with: Vec<String>,
    /// `alternativePackageIds`: lowercase dependency id -> IDs that satisfy it as well.
    #[serde(default)]
    pub dependency_alternatives: std::collections::HashMap<String, Vec<String>>,
    /// Raw `*ByVersion` blocks from About.xml (resolved into the lists above after scanning).
    #[serde(default)]
    pub by_version: VersionedLists,
}

/// `modDependenciesByVersion`, `loadAfterByVersion`, ... keyed by game version ("1.6").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VersionedLists {
    #[serde(default)]
    pub dependencies: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub load_after: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub load_before: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub incompatible_with: std::collections::HashMap<String, Vec<String>>,
}

impl ModInfo {
    pub fn is_protected(&self) -> bool {
        is_protected_package_id(&self.package_id)
    }

    pub fn has_russian(&self) -> bool {
        self.languages.iter().any(|l| {
            let lower = l.to_lowercase();
            lower.contains("russian") || lower.contains("русск") || lower == "ru"
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TranslationTargetLang {
    #[default]
    Ru,
    En,
    Zh,
    De,
    Fr,
    Ja,
    Ko,
    Es,
    PtBr,
    Pl,
    It,
    Tr,
    Uk,
    Cs,
    All,
}

/// How a language shows up in mod folders and Workshop titles (languages without
/// hand-written rules below).
struct LangSpec {
    /// Lower-case pieces of RimWorld `Languages` folder names ("japanese", "日本語").
    folders: &'static [&'static str],
    /// Lower-case words in titles ("japanese", "日本語", "tłumaczenie").
    words: &'static [&'static str],
    /// Short codes, counted only as tags: "[jp]", "(jp)", "_jp", "... jp" at the end.
    codes: &'static [&'static str],
    /// Letters only this language uses (kana, Hangul, Ukrainian і ї є ґ).
    script: Option<fn(&str) -> bool>,
}

impl LangSpec {
    fn title_matches(&self, title: &str) -> bool {
        let lower = title.to_lowercase();
        self.words.iter().any(|w| lower.contains(w))
            || self.codes.iter().any(|c| has_code_tag(&lower, c))
            || self.script.is_some_and(|f| f(title))
    }

    fn folder_matches(&self, folder: &str) -> bool {
        let lower = folder.to_lowercase();
        self.folders.iter().any(|f| lower.contains(f))
    }
}

/// "[jp]", "(jp)", "_jp" or " jp" at the end: a code used as a tag, not a word inside a sentence.
fn has_code_tag(lower: &str, code: &str) -> bool {
    lower.contains(&format!("[{code}]"))
        || lower.contains(&format!("({code})"))
        || lower.contains(&format!("_{code}"))
        || lower.ends_with(&format!(" {code}"))
}

pub fn contains_kana(s: &str) -> bool {
    s.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c))
}

pub fn contains_hangul(s: &str) -> bool {
    s.chars().any(|c| {
        ('\u{AC00}'..='\u{D7AF}').contains(&c)
            || ('\u{1100}'..='\u{11FF}').contains(&c)
            || ('\u{3130}'..='\u{318F}').contains(&c)
    })
}

/// Letters Ukrainian has and Russian doesn't.
pub fn contains_ukrainian_letters(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c, 'і' | 'ї' | 'є' | 'ґ' | 'І' | 'Ї' | 'Є' | 'Ґ'))
}

fn has_uk_marker(title: &str) -> bool {
    let lower = title.to_lowercase();
    contains_ukrainian_letters(title)
        || lower.contains("ukrain")
        || lower.contains("україн")
        || has_code_tag(&lower, "ua")
}

impl TranslationTargetLang {
    /// Languages offered for translation search, most common on the Workshop first.
    pub const POPULAR: [TranslationTargetLang; 13] = [
        Self::Ru,
        Self::Zh,
        Self::Ja,
        Self::Ko,
        Self::De,
        Self::Fr,
        Self::Es,
        Self::PtBr,
        Self::Pl,
        Self::It,
        Self::Tr,
        Self::Uk,
        Self::Cs,
    ];

    /// The language's own name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ru => "Русский",
            Self::En => "English",
            Self::Zh => "中文",
            Self::De => "Deutsch",
            Self::Fr => "Français",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
            Self::Es => "Español",
            Self::PtBr => "Português (BR)",
            Self::Pl => "Polski",
            Self::It => "Italiano",
            Self::Tr => "Türkçe",
            Self::Uk => "Українська",
            Self::Cs => "Čeština",
            Self::All => "All",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Ru => "RU",
            Self::En => "EN",
            Self::Zh => "ZH",
            Self::De => "DE",
            Self::Fr => "FR",
            Self::Ja => "JA",
            Self::Ko => "KO",
            Self::Es => "ES",
            Self::PtBr => "PT-BR",
            Self::Pl => "PL",
            Self::It => "IT",
            Self::Tr => "TR",
            Self::Uk => "UK",
            Self::Cs => "CS",
            Self::All => "",
        }
    }

    /// English name, added to Workshop searches ("Dubs Bad Hygiene Japanese").
    pub fn search_suffix(&self) -> &'static str {
        match self {
            Self::Ru => "Russian",
            Self::En => "English",
            Self::Zh => "Chinese",
            Self::De => "German",
            Self::Fr => "French",
            Self::Ja => "Japanese",
            Self::Ko => "Korean",
            Self::Es => "Spanish",
            Self::PtBr => "Portuguese",
            Self::Pl => "Polish",
            Self::It => "Italian",
            Self::Tr => "Turkish",
            Self::Uk => "Ukrainian",
            Self::Cs => "Czech",
            Self::All => "",
        }
    }

    fn spec(&self) -> Option<LangSpec> {
        let s = match self {
            Self::Ja => LangSpec {
                folders: &["japanese", "日本語"],
                words: &["japanese", "日本語"],
                codes: &["jp", "ja", "jpn"],
                script: Some(contains_kana),
            },
            Self::Ko => LangSpec {
                folders: &["korean", "한국어"],
                words: &["korean", "한국어", "한글"],
                codes: &["kr", "ko", "kor"],
                script: Some(contains_hangul),
            },
            Self::Es => LangSpec {
                folders: &["spanish", "español"],
                words: &[
                    "spanish",
                    "español",
                    "espanol",
                    "castellano",
                    "traducción",
                    "traduccion",
                ],
                codes: &["es", "esp", "spa"],
                script: None,
            },
            Self::PtBr => LangSpec {
                folders: &["portuguese", "português"],
                words: &[
                    "portuguese",
                    "português",
                    "portugues",
                    "brasileiro",
                    "tradução",
                    "traducao",
                ],
                codes: &["pt", "br", "ptbr", "pt-br"],
                script: None,
            },
            Self::Pl => LangSpec {
                folders: &["polish", "polski"],
                words: &["polish", "polski", "spolszczenie", "tłumaczenie", "tlumaczenie"],
                codes: &["pl", "pol"],
                script: None,
            },
            Self::It => LangSpec {
                folders: &["italian", "italiano"],
                words: &["italian", "italiano", "traduzione"],
                codes: &["it", "ita"],
                script: None,
            },
            Self::Tr => LangSpec {
                folders: &["turkish", "türkçe"],
                words: &["turkish", "türkçe", "turkce", "çeviri", "ceviri"],
                codes: &["tr", "tur"],
                script: None,
            },
            Self::Uk => LangSpec {
                folders: &["ukrainian", "українськ"],
                words: &["ukrainian", "україн"],
                codes: &["ua", "ukr"],
                script: Some(contains_ukrainian_letters),
            },
            Self::Cs => LangSpec {
                folders: &["czech", "čeština"],
                words: &["czech", "čeština", "cestina", "česk", "překlad", "preklad"],
                codes: &["cz", "cs", "cze"],
                script: None,
            },
            _ => return None,
        };
        Some(s)
    }

    /// The mod itself ships this language (a folder in its Languages): no translation needed.
    pub fn is_built_into(&self, m: &ModInfo) -> bool {
        if let Some(spec) = self.spec() {
            return m.languages.iter().any(|l| spec.folder_matches(l));
        }
        let words: &[&str] = match self {
            Self::Ru => &["russian", "русский"],
            Self::En => &["english"],
            Self::Zh => &["chinese"],
            Self::De => &["german", "deutsch"],
            Self::Fr => &["french", "français"],
            _ => return false,
        };
        m.languages.iter().any(|l| {
            let lower = l.to_lowercase();
            words.iter().any(|w| lower.contains(w))
        })
    }

    /// Checks if a local mod on disk matches this target language.
    pub fn matches_mod_info(&self, m: &ModInfo) -> bool {
        if let Some(spec) = self.spec() {
            return m.languages.iter().any(|l| spec.folder_matches(l)) || spec.title_matches(&m.name);
        }
        match self {
            Self::All => true,
            Self::Ru => {
                m.has_russian()
                    || contains_cyrillic(&m.name)
                    || has_ru_tag_or_keyword(&m.name)
                    || m.package_id.to_lowercase().contains(".ru")
                    || m.package_id.to_lowercase().starts_with("ru.")
            }
            Self::En => {
                m.languages.iter().any(|l| {
                    let l_low = l.to_lowercase();
                    l_low.contains("english") || l_low == "en"
                }) || m.name.to_lowercase().contains("[en]")
            }
            Self::Zh => {
                m.languages.iter().any(|l| {
                    let l_low = l.to_lowercase();
                    l_low.contains("chinese")
                        || l_low.contains("simplified")
                        || l_low.contains("traditional")
                        || l_low == "zh"
                }) || contains_chinese_chars(&m.name)
                    || has_zh_tag_or_keyword(&m.name)
                    || m.package_id.to_lowercase().contains(".zh")
                    || m.package_id.to_lowercase().starts_with("zh.")
                    || m.package_id.to_lowercase().contains(".chs")
                    || m.package_id.to_lowercase().ends_with("chs")
            }
            Self::De => {
                m.languages.iter().any(|l| {
                    let l_low = l.to_lowercase();
                    l_low.contains("german") || l_low.contains("deutsch") || l_low == "de"
                }) || m.name.to_lowercase().contains("[de]")
                    || m.name.to_lowercase().contains("deutsch")
            }
            Self::Fr => {
                m.languages.iter().any(|l| {
                    let l_low = l.to_lowercase();
                    l_low.contains("french") || l_low.contains("français") || l_low == "fr"
                }) || m.name.to_lowercase().contains("[fr]")
                    || m.name.to_lowercase().contains("francais")
                    || m.name.to_lowercase().contains("traduction")
            }
            // Languages with a LangSpec are handled above.
            _ => false,
        }
    }

    /// Checks if a Workshop item matches this target language based on title and tags.
    pub fn matches_workshop_item(&self, title: &str, tags: &[String]) -> bool {
        if let Some(spec) = self.spec() {
            let tag_name = self.search_suffix();
            return spec.title_matches(title) || tags.iter().any(|t| t.eq_ignore_ascii_case(tag_name));
        }
        let title_low = title.to_lowercase();

        let is_cyr = contains_cyrillic(title);
        let has_ru = is_cyr || has_ru_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("Russian"));

        let is_zh_chars = contains_chinese_chars(title);
        let has_zh = is_zh_chars
            || has_zh_tag_or_keyword(title)
            || tags.iter().any(|t| {
                t.eq_ignore_ascii_case("Chinese")
                    || t.eq_ignore_ascii_case("Traditional Chinese")
                    || t.eq_ignore_ascii_case("Simplified Chinese")
            });

        let has_fr = has_fr_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("French"));

        let has_de = has_de_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("German"));

        match self {
            Self::Ru => {
                // Must have positive Russian marker
                if !has_ru {
                    return false;
                }
                // Cyrillic alone isn't Russian: a Ukrainian translation without a Russian marker is not it.
                if has_uk_marker(title)
                    && !title_low.contains("russian")
                    && !title_low.contains("рус")
                    && !title_low.contains("[ru]")
                {
                    return false;
                }
                // If it contains strong Chinese/French/German markers without Cyrillic/Russian, reject
                if (has_zh || has_fr || has_de)
                    && !is_cyr
                    && !title_low.contains("russian")
                    && !title_low.contains("[ru]")
                {
                    return false;
                }
                true
            }
            // Japanese titles use Chinese characters too; kana means Japanese.
            Self::Zh => has_zh && !has_ru && !contains_kana(title),
            Self::Fr => has_fr && !has_ru && !has_zh,
            Self::De => has_de && !has_ru && !has_zh,
            Self::En => {
                (title_low.contains("english") || title_low.contains("[en]"))
                    && !has_ru
                    && !has_zh
                    && !has_fr
                    && !has_de
            }
            Self::All => true,
            // Languages with a LangSpec are handled above.
            _ => false,
        }
    }
}

pub fn contains_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

pub fn contains_chinese_chars(s: &str) -> bool {
    s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
}

/// The text has one of `words` as a whole word ("Rus Dubs Bad Hygiene", "Allow Tool - CN").
fn has_word(lower: &str, words: &[&str]) -> bool {
    lower.split(|c: char| !c.is_alphanumeric()).any(|w| words.contains(&w))
}

pub fn has_ru_tag_or_keyword(s: &str) -> bool {
    let lower = s.to_lowercase();
    has_word(&lower, &["ru", "rus"])
        || lower.contains("[ru]")
        || lower.contains("(ru)")
        || lower.contains("_ru")
        || lower.ends_with(" ru")
        || lower.contains(" ru ")
        || lower.contains(".ru")
        || lower.contains("[rus]")
        || lower.contains("[рус]")
        || lower.contains("russian")
        || lower.contains("русификатор")
        || lower.contains("перевод")
        || lower.contains("русский")
}

pub fn is_package_id_translation(id: &str) -> bool {
    let id_lower = id.to_lowercase();
    id_lower.starts_with("zh.")
        || id_lower.starts_with("rwzh.")
        || id_lower.starts_with("ru.")
        || id_lower.starts_with("chs.")
        || id_lower.ends_with(".zh")
        || id_lower.ends_with(".zh.hc")
        || id_lower.ends_with(".ru")
        || id_lower.ends_with(".chs")
        || id_lower.ends_with(".cht")
        || id_lower.ends_with(".translation")
        || id_lower.ends_with(".trans")
        || id_lower.ends_with(".locale")
        || id_lower.contains(".chs.")
        || id_lower.contains(".zh.")
        || id_lower.contains(".ru.")
        || id_lower.contains(".translation.")
}

pub fn is_mod_name_translation_tagged(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    name_lower.ends_with("_zh")
        || name_lower.ends_with("_ru")
        || name_lower.ends_with(" zh")
        || name_lower.ends_with(" ru")
        || name_lower.ends_with(" chs")
        || name_lower.ends_with(" cht")
        || name_lower.contains("[zh]")
        || name_lower.contains("[ru]")
        || name_lower.contains("[cn]")
        || name_lower.contains("[chs]")
        || name_lower.contains("[cht]")
        || name_lower.contains("[rus]")
        || name_lower.contains("[рус]")
        || name_lower.contains("(zh)")
        || name_lower.contains("(ru)")
        || name_lower.contains("(cn)")
}

pub fn has_translation_keyword(s: &str) -> bool {
    let lower = s.to_lowercase();
    let trans_keywords = [
        "zh-pack",
        "汉化",
        "繁體中文",
        "简体中文",
        "translation",
        "translate",
        "localization",
        "русификатор",
        "перевод",
        "russian language",
        "russian translation",
        "chinese pack",
        "chinese translation",
    ];
    trans_keywords.iter().any(|&kw| lower.contains(kw))
}

pub fn has_explicit_translation_title_marker(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    name_lower.contains("zh-pack")
        || name_lower.contains("汉化")
        || name_lower.contains("русификатор")
        || name_lower.contains("translation")
}

pub fn is_translation_indicator(package_id: &str, name: &str, is_translation: bool) -> bool {
    let id_lower = package_id.to_lowercase();
    let name_lower = name.to_lowercase();

    is_translation
        || is_package_id_translation(&id_lower)
        || is_mod_name_translation_tagged(&name_lower)
        || has_translation_keyword(&id_lower)
        || has_translation_keyword(&name_lower)
        || has_ru_tag_or_keyword(&name_lower)
        || has_zh_tag_or_keyword(&name_lower)
}

pub fn has_zh_tag_or_keyword(s: &str) -> bool {
    let lower = s.to_lowercase();
    has_word(&lower, &["zh", "cn", "chs", "cht"])
        || lower.contains("[zh]")
        || lower.contains("(zh)")
        || lower.contains("_zh")
        || lower.ends_with(" zh")
        || lower.contains(" zh ")
        || lower.contains("[cn]")
        || lower.contains("[chs]")
        || lower.contains("[cht]")
        || lower.contains("zh-pack")
        || lower.contains("chinese")
        || lower.contains("汉化")
        || lower.contains("繁體")
        || lower.contains("简体")
        || lower.contains("中文")
}

pub fn has_fr_tag_or_keyword(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("[fr]")
        || lower.contains("(fr)")
        || lower.contains("_fr")
        || lower.ends_with(" fr")
        || lower.contains("french")
        || lower.contains("français")
        || lower.contains("francais")
        || lower.contains("traduction")
}

pub fn has_de_tag_or_keyword(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("[de]")
        || lower.contains("(de)")
        || lower.contains("_de")
        || lower.ends_with(" de")
        || lower.contains("german")
        || lower.contains("deutsch")
        || lower.contains("deutsche")
        || lower.contains("übersetzung")
        || lower.contains("uebersetzung")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommunityRule {
    #[serde(default)]
    pub tier: Option<u8>,
    #[serde(default)]
    pub load_before: Vec<String>,
    #[serde(default)]
    pub load_after: Vec<String>,
    #[serde(default)]
    pub force_load_before: Vec<String>,
    #[serde(default)]
    pub force_load_after: Vec<String>,
    #[serde(default)]
    pub incompatible: Vec<String>,
}
