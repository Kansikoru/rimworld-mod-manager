use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkshopItemResult {
    pub id: String,
    pub title: String,
    pub preview_url: Option<String>,
    pub tags: Vec<String>,
    pub is_translation: bool,
    pub score: i32,
}

#[derive(Deserialize)]
struct ApiPublishedFileDetailsResponse {
    response: Option<ApiResponseBody>,
}

#[derive(Deserialize)]
struct ApiResponseBody {
    publishedfiledetails: Option<Vec<ApiFileDetail>>,
}

#[derive(Deserialize)]
struct ApiFileDetail {
    publishedfileid: Option<String>,
    result: Option<i32>,
    title: Option<String>,
    preview_url: Option<String>,
    tags: Option<Vec<ApiTag>>,
}

#[derive(Deserialize)]
struct ApiTag {
    tag: Option<String>,
}

static WORKSHOP_ID_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"href="https://steamcommunity\.com/sharedfiles/filedetails/\?id=(\d+)""#).unwrap()
});

pub fn search_steam_workshop(query: &str) -> Result<Vec<WorkshopItemResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let encoded_query = urlencoding_simple(trimmed);
    let browse_url = format!(
        "https://steamcommunity.com/workshop/browse/?appid=294100&searchtext={encoded_query}&browsesort=textsearch&section=readytouseitems"
    );

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build();

    let html_resp = agent
        .get(&browse_url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .set("Accept-Language", "ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7")
        .call()
        .map_err(|e| format!("Ошибка подключения к Steam Workshop: {e}"))?;

    let html = html_resp
        .into_string()
        .map_err(|e| format!("Ошибка чтения ответа Steam Workshop: {e}"))?;

    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    for cap in WORKSHOP_ID_RE.captures_iter(&html) {
        if let Some(m) = cap.get(1) {
            let id_str = m.as_str().to_string();
            if seen.insert(id_str.clone()) {
                ids.push(id_str);
                if ids.len() >= 12 {
                    break;
                }
            }
        }
    }

    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let api_url = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
    let mut form_params: Vec<(String, String)> = Vec::new();
    form_params.push(("itemcount".to_string(), ids.len().to_string()));
    for (i, id) in ids.iter().enumerate() {
        form_params.push((format!("publishedfileids[{i}]"), id.clone()));
    }

    let param_refs: Vec<(&str, &str)> = form_params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let api_resp = agent
        .post(api_url)
        .set("User-Agent", "Mozilla/5.0")
        .send_form(&param_refs);

    if let Ok(resp) = api_resp {
        if let Ok(body) = resp.into_string() {
            if let Ok(parsed) = serde_json::from_str::<ApiPublishedFileDetailsResponse>(&body) {
                if let Some(body) = parsed.response {
                    if let Some(details) = body.publishedfiledetails {
                        let mut results = Vec::new();
                        for item in details {
                            if item.result != Some(1) {
                                continue;
                            }
                            let id = item.publishedfileid.unwrap_or_default();
                            let title = item.title.unwrap_or_else(|| "Без названия".to_string());
                            let preview_url = item.preview_url;
                            let mut tags = Vec::new();
                            let mut is_translation = false;

                            if let Some(tag_list) = item.tags {
                                for t in tag_list {
                                    if let Some(tag_name) = t.tag {
                                        if tag_name.eq_ignore_ascii_case("Translation") {
                                            is_translation = true;
                                        }
                                        tags.push(tag_name);
                                    }
                                }
                            }

                            let title_lower = title.to_lowercase();
                            if title_lower.contains("translation")
                                || title_lower.contains("перевод")
                                || title_lower.contains("[ru]")
                                || title_lower.contains("(ru)")
                                || title_lower.contains("русификатор")
                            {
                                is_translation = true;
                            }

                            if !id.is_empty() && !title.is_empty() {
                                let score = calculate_relevance(&title, trimmed, is_translation);
                                results.push(WorkshopItemResult {
                                    title,
                                    id,
                                    preview_url,
                                    tags,
                                    is_translation,
                                    score,
                                });
                            }
                        }

                        if !results.is_empty() {
                            results.sort_by_key(|b| std::cmp::Reverse(b.score));
                            return Ok(results);
                        }
                    }
                }
            }
        }
    }

    let fallback = ids
        .into_iter()
        .map(|id| WorkshopItemResult {
            title: format!("Steam Workshop Item #{id}"),
            id,
            preview_url: None,
            tags: Vec::new(),
            is_translation: false,
            score: 0,
        })
        .collect();

    Ok(fallback)
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

fn urlencoding_simple(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => {
                out.push('+');
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
