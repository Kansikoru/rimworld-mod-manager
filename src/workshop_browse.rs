//! The Workshop tab: RimWorld's Steam Workshop browsed inside the manager. The browse page
//! (search, sort, game version, page) gives the item IDs; `GetPublishedFileDetails` (no key
//! needed) gives titles, descriptions, previews, subscriber counts and version tags.

use regex::Regex;
use std::sync::LazyLock;
use std::time::Duration;

const BROWSE_URL: &str = "https://steamcommunity.com/workshop/browse/";
const DETAILS_URL: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
pub const PER_PAGE: usize = 30;

/// How the catalog is sorted (a search is always by relevance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowseSort {
    #[default]
    TrendWeek,
    TrendMonth,
    MostSubscribed,
    Recent,
    Updated,
}

impl BrowseSort {
    pub const ALL: [BrowseSort; 5] = [
        BrowseSort::TrendWeek,
        BrowseSort::TrendMonth,
        BrowseSort::MostSubscribed,
        BrowseSort::Recent,
        BrowseSort::Updated,
    ];

    /// `browsesort` and the trend period in days (0 = none).
    fn param(self) -> (&'static str, u32) {
        match self {
            BrowseSort::TrendWeek => ("trend", 7),
            BrowseSort::TrendMonth => ("trend", 30),
            BrowseSort::MostSubscribed => ("totaluniquesubscribers", 0),
            BrowseSort::Recent => ("mostrecent", 0),
            BrowseSort::Updated => ("lastupdated", 0),
        }
    }
}

/// What to show: one page of a search or of the sorted catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowseQuery {
    pub text: String,
    pub sort: BrowseSort,
    /// Only items tagged with this game version ("1.6"...).
    pub version: Option<String>,
    /// From 1.
    pub page: u32,
}

/// A Workshop item as the tab shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkshopItem {
    pub id: String,
    pub title: String,
    /// Plain text (BBCode removed).
    pub description: String,
    pub preview_url: Option<String>,
    pub subscriptions: u64,
    pub file_size: u64,
    pub time_updated: u64,
    /// Game versions from the tags: "1.6", "1.7"...
    pub versions: Vec<String>,
    /// Other tags ("Mod", "Translation"...).
    pub tags: Vec<String>,
}

fn encode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

pub fn browse_url(q: &BrowseQuery) -> String {
    let mut url = format!("{BROWSE_URL}?appid=294100&section=readytouseitems&numperpage={PER_PAGE}");
    let text = q.text.trim();
    if text.is_empty() {
        let (sort, days) = q.sort.param();
        url.push_str(&format!("&browsesort={sort}&actualsort={sort}"));
        if days > 0 {
            url.push_str(&format!("&days={days}"));
        }
    } else {
        url.push_str(&format!("&searchtext={}&browsesort=textsearch", encode(text)));
    }
    if let Some(v) = &q.version {
        url.push_str(&format!("&requiredtags%5B%5D={}", encode(v)));
    }
    url.push_str(&format!("&p={}", q.page.max(1)));
    url
}

/// The page keeps its items in embedded JSON (`\"publishedfileid\":\"123\"`); older pages
/// had links. IDs come in page order, each once.
pub fn parse_browse_ids(html: &str) -> Vec<String> {
    static JSON_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\\"publishedfileid\\":\\"(\d+)\\""#).unwrap());
    static HREF_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"href="https://steamcommunity\.com/sharedfiles/filedetails/\?id=(\d+)""#).unwrap()
    });
    let mut seen = std::collections::HashSet::new();
    let mut ids: Vec<String> = JSON_RE
        .captures_iter(html)
        .map(|c| c[1].to_string())
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if ids.is_empty() {
        ids = HREF_RE
            .captures_iter(html)
            .map(|c| c[1].to_string())
            .filter(|id| seen.insert(id.clone()))
            .collect();
    }
    ids.truncate(PER_PAGE);
    ids
}

/// BBCode -> plain text: tags dropped, `[url=x]text[/url]` keeps the text, images and
/// empty lines in a row removed.
pub fn bbcode_to_text(s: &str) -> String {
    static IMG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)\[img\].*?\[/img\]").unwrap());
    static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[/?[a-zA-Z0-9*]+(=[^\]]*)?\]").unwrap());
    static BLANK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n[ \t]*(\n[ \t]*){2,}").unwrap());
    let s = s.replace("\r\n", "\n");
    let s = IMG_RE.replace_all(&s, "");
    let s = TAG_RE.replace_all(&s, "");
    let s = BLANK_RE.replace_all(&s, "\n\n");
    s.trim().to_string()
}

fn is_version_tag(tag: &str) -> bool {
    let mut parts = tag.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
    )
}

/// Reads a `GetPublishedFileDetails` answer, in the order of `ids`; hidden or removed items
/// are left out.
pub fn parse_items(json: &str, ids: &[String]) -> Result<Vec<WorkshopItem>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = value["response"]["publishedfiledetails"]
        .as_array()
        .ok_or("no publishedfiledetails")?;
    let mut by_id = std::collections::HashMap::new();
    for item in list {
        if item["result"].as_i64() != Some(1) {
            continue;
        }
        let Some(id) = item["publishedfileid"].as_str() else {
            continue;
        };
        let number = |key: &str| {
            item[key]
                .as_u64()
                .or_else(|| item[key].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0)
        };
        let (versions, tags): (Vec<String>, Vec<String>) = item["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .filter_map(|t| t["tag"].as_str())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
            .partition(|t| is_version_tag(t));
        by_id.insert(
            id.to_string(),
            WorkshopItem {
                id: id.to_string(),
                title: item["title"].as_str().unwrap_or_default().trim().to_string(),
                description: bbcode_to_text(item["description"].as_str().unwrap_or_default()),
                preview_url: item["preview_url"]
                    .as_str()
                    .filter(|u| u.starts_with("https://"))
                    .map(str::to_string),
                subscriptions: number("subscriptions"),
                file_size: number("file_size"),
                time_updated: number("time_updated"),
                versions,
                tags,
            },
        );
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// One page: the browse page for the IDs, then their details.
pub fn fetch_page(q: &BrowseQuery) -> Result<Vec<WorkshopItem>, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let html = agent
        .get(&browse_url(q))
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let ids = parse_browse_ids(&html);
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut form: Vec<(String, String)> = vec![("itemcount".into(), ids.len().to_string())];
    form.extend(
        ids.iter()
            .enumerate()
            .map(|(i, id)| (format!("publishedfileids[{i}]"), id.clone())),
    );
    let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let json = agent
        .post(DETAILS_URL)
        .send_form(&pairs)
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse_items(&json, &ids)
}

/// The preview at a given width (Steam's image server scales it), for thumbnails.
pub fn sized_preview(url: &str, width: u32) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}imw={width}&ima=fit&impolicy=Letterbox&letterbox=false")
}

/// Downloads a preview and makes an RGBA image no wider than `max_width`.
pub fn load_preview(url: &str, max_width: u32) -> Option<image::RgbaImage> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let mut bytes = Vec::new();
    use std::io::Read;
    agent
        .get(url)
        .set("Accept", "image/png,image/jpeg,image/gif")
        .call()
        .ok()?
        .into_reader()
        .take(20 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let img = if img.width() > max_width {
        img.resize(max_width, u32::MAX, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Some(img.to_rgba8())
}

/// "1.2 MB", "340 KB".
pub fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (MB * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// "1 234 567" -> "1.2M", "45 300" -> "45.3K".
pub fn short_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_for_catalog_and_search() {
        let q = BrowseQuery {
            version: Some("1.6".into()),
            page: 2,
            ..Default::default()
        };
        assert_eq!(
            browse_url(&q),
            "https://steamcommunity.com/workshop/browse/?appid=294100&section=readytouseitems&numperpage=30&browsesort=trend&actualsort=trend&days=7&requiredtags%5B%5D=1.6&p=2"
        );
        let q = BrowseQuery {
            text: " Hugs Lib ".into(),
            sort: BrowseSort::Recent,
            page: 0,
            ..Default::default()
        };
        assert_eq!(
            browse_url(&q),
            "https://steamcommunity.com/workshop/browse/?appid=294100&section=readytouseitems&numperpage=30&searchtext=Hugs+Lib&browsesort=textsearch&p=1"
        );
    }

    #[test]
    fn ids_from_the_page() {
        let html = r#"x \"publishedfileid\":\"111\",\"title\":\"A\" ... \"publishedfileid\":\"222\" \"publishedfileid\":\"111\"
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=999">"#;
        assert_eq!(parse_browse_ids(html), ["111", "222"]);
        let old = r#"<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=5"></a><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=6">"#;
        assert_eq!(parse_browse_ids(old), ["5", "6"]);
    }

    #[test]
    fn details_in_page_order() {
        let json = r#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"2","result":1,"title":"Two","description":"[h1]Hi[/h1]\r\n[url=https://x]link[/url]","preview_url":"https://img/2.png","subscriptions":1500,"file_size":"2048","time_updated":100,"tags":[{"tag":"Mod"},{"tag":"1.6"}]},
            {"publishedfileid":"1","result":1,"title":" One ","subscriptions":3},
            {"publishedfileid":"3","result":9}
        ]}}"#;
        let ids = ["1".to_string(), "2".to_string(), "3".to_string()];
        let items = parse_items(json, &ids).unwrap();
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["1", "2"]);
        assert_eq!(items[0].title, "One");
        assert_eq!(items[1].description, "Hi\nlink");
        assert_eq!((items[1].file_size, items[1].subscriptions), (2048, 1500));
        assert_eq!(items[1].versions, ["1.6"]);
        assert_eq!(items[1].tags, ["Mod"]);
    }

    #[test]
    fn bbcode_is_removed() {
        assert_eq!(
            bbcode_to_text("[b]Bold[/b] [img]https://i/x.png[/img]text\n\n\n\n[list][*]one[/list]"),
            "Bold text\n\none"
        );
    }

    #[test]
    fn numbers_for_people() {
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(5 * 1024 * 1024 + 300_000), "5.3 MB");
        assert_eq!(short_count(45_300), "45.3K");
        assert_eq!(short_count(1_234_567), "1.2M");
        assert_eq!(short_count(999), "999");
        assert_eq!(
            sized_preview("https://img/a.png", 320),
            "https://img/a.png?imw=320&ima=fit&impolicy=Letterbox&letterbox=false"
        );
    }

    /// The real catalog (run by hand: `cargo test --release live_workshop_browse -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_workshop_browse() {
        let items = fetch_page(&BrowseQuery {
            version: Some("1.6".into()),
            page: 1,
            ..Default::default()
        })
        .unwrap();
        println!("{} items", items.len());
        for i in items.iter().take(5) {
            println!(
                "{} | {} | {} subs | {} | {:?} | {:?}",
                i.id,
                i.title,
                i.subscriptions,
                human_size(i.file_size),
                i.versions,
                i.preview_url
            );
        }
        assert!(items.len() >= 20);
        let url = items.iter().find_map(|i| i.preview_url.clone()).unwrap();
        let img = load_preview(&sized_preview(&url, 320), 320).expect("preview");
        println!("preview {}x{}", img.width(), img.height());
        let found = fetch_page(&BrowseQuery {
            text: "HugsLib".into(),
            page: 1,
            ..Default::default()
        })
        .unwrap();
        assert!(found.iter().any(|i| i.id == "818773962"));
    }
}
