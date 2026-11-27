use std::time::Duration;

use crate::workshop_browse;

use crate::i18n::tr;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkshopItemResult {
    pub id: String,
    pub title: String,
    pub preview_url: Option<String>,
    pub tags: Vec<String>,
    pub is_translation: bool,
    pub score: i32,
}

/// How long one request of a text search may take.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(8);

/// A found item with its relevance to the query; translations are told by tag or title.
fn found_item(
    id: String,
    title: String,
    preview_url: Option<String>,
    tags: Vec<String>,
    query: &str,
) -> WorkshopItemResult {
    // Any of the languages on offer ("日本語", "spolszczenie", "Übersetzung"...), not only ru/en.
    let is_translation =
        tags.iter().any(|t| t.eq_ignore_ascii_case("Translation")) || crate::matcher::is_translation_title(&title);
    let score = calculate_relevance(&title, query, is_translation);
    WorkshopItemResult {
        id,
        title,
        preview_url,
        tags,
        is_translation,
        score,
    }
}

/// How many results a search keeps (the website's first matches, the client's too).
const MAX_RESULTS: usize = 12;

/// Searches the Workshop by text: the website, and with the setting on (`route` has the
/// game's Steam DLL) the Steam client when the site refuses or finds nothing. Where the
/// site shows an age check instead of results, it "finds nothing" for every query.
pub fn search_steam_workshop(
    query: &str,
    route: &crate::workshop_browse::Route,
) -> Result<Vec<WorkshopItemResult>, String> {
    let Some(dll) = route.steam_dll.as_deref() else {
        return search_site(query);
    };
    if route.prefer_steam {
        return search_client(dll, query).or_else(|_| search_site(query));
    }
    match search_site(query) {
        Ok(items) if !items.is_empty() => Ok(items),
        site => search_client(dll, query).or(site),
    }
}

/// The same search through the Steam client, from the account the user is signed in to.
fn search_client(dll: &std::path::Path, query: &str) -> Result<Vec<WorkshopItemResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let page = crate::steam::UgcPageQuery {
        query_type: crate::steam::UGC_RANKED_BY_TEXT_SEARCH,
        matching: crate::steam::UGC_MATCHING_READY_TO_USE,
        trend_days: 0,
        text: trimmed,
        required_tags: Vec::new(),
    };
    let mut results: Vec<WorkshopItemResult> = crate::steam::ugc_query_pages(dll, &page, 1..=1)?
        .into_iter()
        .filter(|item| !item.title.is_empty())
        .take(MAX_RESULTS)
        .map(|item| found_item(item.id.to_string(), item.title, item.preview_url, item.tags, trimmed))
        .collect();
    results.sort_by_key(|r| std::cmp::Reverse(r.score));
    Ok(results)
}

/// The website's search page gives the IDs (the same parser as the Workshop tab, so a new
/// page layout breaks both or neither), the Web API their titles and tags. A 5xx or a
/// dropped connection is tried again; a translation search over a whole modpack makes many
/// of these, so each request gives up sooner than the tab's.
fn search_site(query: &str) -> Result<Vec<WorkshopItemResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let browse_url = format!(
        "https://steamcommunity.com/workshop/browse/?appid=294100&searchtext={}&browsesort=textsearch&section=readytouseitems",
        crate::steam_web::encode(trimmed)
    );
    let html = workshop_browse::with_retries(|| {
        workshop_browse::attempt(
            crate::steam_web::agent()
                .get(&browse_url)
                .timeout(SEARCH_TIMEOUT)
                .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .set("Accept-Language", "ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7")
                .call(),
        )
    })
    .map_err(|e| tr!("Нет связи со Steam Workshop: {}", "Can't reach the Steam Workshop: {}", "无法连接 Steam 创意工坊：{}", e))?;

    let mut ids = workshop_browse::parse_browse_ids(&html);
    ids.truncate(MAX_RESULTS);
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // Without the details the IDs alone still lead to the items.
    let Ok(details) = workshop_browse::fetch_details(&ids, SEARCH_TIMEOUT) else {
        return Ok(ids
            .into_iter()
            .map(|id| WorkshopItemResult {
                title: format!("Steam Workshop Item #{id}"),
                id,
                preview_url: None,
                tags: Vec::new(),
                is_translation: false,
                score: 0,
            })
            .collect());
    };
    let mut results: Vec<WorkshopItemResult> = details
        .into_iter()
        .filter(|item| !item.id.is_empty() && !item.title.is_empty())
        .map(|item| {
            let tags = item.tags.into_iter().chain(item.versions).collect();
            found_item(item.id, item.title, item.preview_url, tags, trimmed)
        })
        .collect();
    results.sort_by_key(|r| std::cmp::Reverse(r.score));
    Ok(results)
}

fn calculate_relevance(title: &str, query: &str, is_trans: bool) -> i32 {
    let title_lower = title.to_lowercase();
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    let mut score = 0;
    if is_trans {
        score += 50;
    }
    for word in &query_words {
        if word.len() >= 2 && title_lower.contains(word) {
            score += 20;
        }
    }
    if title_lower.contains(&query_lower) {
        score += 100;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_items_know_translations_and_relevance() {
        let by_tag = found_item(
            "1".into(),
            "HugsLib RU".into(),
            None,
            vec!["Translation".into()],
            "HugsLib",
        );
        let by_title = found_item("2".into(), "HugsLib (translation)".into(), None, vec![], "HugsLib");
        let plain = found_item("3".into(), "HugsLib".into(), None, vec!["Mod".into()], "HugsLib");
        assert!(by_tag.is_translation && by_title.is_translation && !plain.is_translation);
        assert!(by_tag.score > plain.score && plain.score > 0);
    }

    /// The real website and Web API (run by hand:
    /// `cargo test live_site_search -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_site_search() {
        let found = search_site("HugsLib").unwrap();
        for item in &found {
            println!(
                "{} | {} | {} | {}",
                item.id, item.title, item.score, item.is_translation
            );
        }
        assert!(found.len() <= MAX_RESULTS && found.iter().any(|i| i.id == "818773962"));
        assert!(found.iter().all(|i| !i.title.starts_with("Steam Workshop Item #")));
    }

    /// Through the real Steam client (run by hand:
    /// `cargo test live_search_through_steam_client -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_search_through_steam_client() {
        let dll = crate::steam::detect_steam_paths()
            .steam_dll_path
            .expect("steam_api64.dll");
        let found = search_client(&dll, "HugsLib").unwrap();
        for item in &found {
            println!("{} | {} | {} | {:?}", item.id, item.title, item.score, item.tags);
        }
        assert!(found.len() <= MAX_RESULTS && found.iter().any(|i| i.id == "818773962"));
        // With the setting on and the site answering, the site's results are used.
        let route = crate::workshop_browse::Route {
            steam_dll: Some(dll),
            prefer_steam: false,
        };
        assert!(!search_steam_workshop("HugsLib", &route).unwrap().is_empty());
    }
}
