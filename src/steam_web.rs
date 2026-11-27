//! What every request to the Steam website and Web API shares: one HTTP client (so
//! kept-alive connections are reused across a batch), the API addresses, the form
//! with item IDs, URL encoding of a search text.

use std::sync::LazyLock;
use std::time::Duration;

pub const DETAILS_URL: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
pub const COLLECTION_URL: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";

/// The shared client. A request that must give up sooner sets its own `timeout`.
pub fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> =
        LazyLock::new(|| ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build());
    &AGENT
}

/// The POST form of `GetPublishedFileDetails` (`count_key` "itemcount") or
/// `GetCollectionDetails` ("collectioncount"): the count, then `publishedfileids[i]`.
pub fn ids_form(count_key: &str, ids: &[String]) -> Vec<(String, String)> {
    let mut form = vec![(count_key.to_string(), ids.len().to_string())];
    form.extend(
        ids.iter()
            .enumerate()
            .map(|(i, id)| (format!("publishedfileids[{i}]"), id.clone())),
    );
    form
}

/// The form as `send_form` takes it.
pub fn form_pairs(form: &[(String, String)]) -> Vec<(&str, &str)> {
    form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// A search text for a query string: letters, digits and `-_.~` as is, a space as `+`,
/// every other byte as `%XX`.
pub fn encode(s: &str) -> String {
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

/// A game version tag ("1.6", "1.7"), not "Mod" or another tag.
pub fn is_version_tag(tag: &str) -> bool {
    let mut parts = tag.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_text_is_encoded() {
        assert_eq!(encode("Mya's fufu"), "Mya%27s+fufu");
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode("Мод"), "%D0%9C%D0%BE%D0%B4");
    }

    #[test]
    fn version_tags_only() {
        assert!(is_version_tag("1.6"));
        assert!(is_version_tag("1.10"));
        assert!(!is_version_tag("Mod"));
        assert!(!is_version_tag("1.6.1"));
        assert!(!is_version_tag("1."));
        assert!(!is_version_tag("v1.6"));
    }

    #[test]
    fn ids_form_counts_and_numbers_the_ids() {
        let ids = vec!["10".to_string(), "20".to_string()];
        assert_eq!(
            ids_form("itemcount", &ids),
            vec![
                ("itemcount".to_string(), "2".to_string()),
                ("publishedfileids[0]".to_string(), "10".to_string()),
                ("publishedfileids[1]".to_string(), "20".to_string()),
            ]
        );
    }
}
