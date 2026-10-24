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
    OPTIMIZER_BLACKLIST
        .iter()
        .any(|&p| p.eq_ignore_ascii_case(package_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModOrigin {
    Workshop,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    All,
}

impl TranslationTargetLang {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ru => "🇷🇺 Русский",
            Self::En => "🇬🇧 English",
            Self::Zh => "🇨🇳 中文",
            Self::De => "🇩🇪 Deutsch",
            Self::Fr => "🇫🇷 Français",
            Self::All => "🌐 Все",
        }
    }

    pub fn search_suffix(&self) -> &'static str {
        match self {
            Self::Ru => "Russian",
            Self::En => "English",
            Self::Zh => "Chinese",
            Self::De => "German",
            Self::Fr => "French",
            Self::All => "",
        }
    }

    /// Checks if a local mod on disk matches this target language.
    pub fn matches_mod_info(&self, m: &ModInfo) -> bool {
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
        }
    }

    /// Checks if a Workshop item matches this target language based on title and tags.
    pub fn matches_workshop_item(&self, title: &str, tags: &[String]) -> bool {
        let title_low = title.to_lowercase();

        let is_cyr = contains_cyrillic(title);
        let has_ru = is_cyr
            || has_ru_tag_or_keyword(title)
            || tags.iter().any(|t| t.eq_ignore_ascii_case("Russian"));

        let is_zh_chars = contains_chinese_chars(title);
        let has_zh = is_zh_chars
            || has_zh_tag_or_keyword(title)
            || tags.iter().any(|t| {
                t.eq_ignore_ascii_case("Chinese")
                    || t.eq_ignore_ascii_case("Traditional Chinese")
                    || t.eq_ignore_ascii_case("Simplified Chinese")
            });

        let has_fr =
            has_fr_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("French"));

        let has_de =
            has_de_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("German"));

        match self {
            Self::Ru => {
                // Must have positive Russian marker
                if !has_ru {
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
            Self::Zh => has_zh && !has_ru,
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
        }
    }
}

pub fn contains_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

pub fn contains_chinese_chars(s: &str) -> bool {
    s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
}

pub fn has_ru_tag_or_keyword(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("[ru]")
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
    lower.contains("[zh]")
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

/// Detects the visual language badge for an item.
pub fn detect_language_badge(title: &str, tags: &[String]) -> &'static str {
    let title_low = title.to_lowercase();
    if contains_cyrillic(title)
        || has_ru_tag_or_keyword(title)
        || tags.iter().any(|t| t.eq_ignore_ascii_case("Russian"))
    {
        "🇷🇺 RU"
    } else if contains_chinese_chars(title)
        || has_zh_tag_or_keyword(title)
        || tags.iter().any(|t| {
            t.eq_ignore_ascii_case("Chinese")
                || t.eq_ignore_ascii_case("Traditional Chinese")
                || t.eq_ignore_ascii_case("Simplified Chinese")
        })
    {
        "🇨🇳 ZH"
    } else if has_fr_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("French"))
    {
        "🇫🇷 FR"
    } else if has_de_tag_or_keyword(title) || tags.iter().any(|t| t.eq_ignore_ascii_case("German"))
    {
        "🇩🇪 DE"
    } else if title_low.contains("english")
        || title_low.contains("[en]")
        || tags.iter().any(|t| t.eq_ignore_ascii_case("English"))
    {
        "🇬🇧 EN"
    } else {
        "🌐"
    }
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
