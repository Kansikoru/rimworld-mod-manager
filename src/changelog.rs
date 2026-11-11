//! What changed in a Workshop mod: the author's change notes, read from the mod's
//! "Change Notes" page on the Steam website (the public Web API doesn't give them).
//! The page lists the ten latest updates; each note sits in `<p id="<unix time>">`.

use regex::Regex;
use std::sync::LazyLock;
use std::time::Duration;

/// One update of a mod and what its author wrote about it (often nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeNote {
    /// When the update was published (unix seconds).
    pub time: u64,
    pub text: String,
}

pub fn page_url(workshop_id: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/changelog/{workshop_id}")
}

static NOTE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<p id="(\d+)">(.*?)</p>"#).unwrap());
static BREAK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#\d+|#x[0-9a-fA-F]+|[a-zA-Z]+);").unwrap());
static BLANK_LINES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// A note's HTML as plain text: `<br>` is a new line, other tags go, entities are decoded.
fn html_to_text(html: &str) -> String {
    let text = BREAK_RE.replace_all(html, "\n");
    let text = TAG_RE.replace_all(&text, "");
    let text = ENTITY_RE.replace_all(&text, |caps: &regex::Captures| {
        let name = &caps[1];
        let code = if let Some(hex) = name.strip_prefix("#x") {
            u32::from_str_radix(hex, 16).ok()
        } else if let Some(dec) = name.strip_prefix('#') {
            dec.parse().ok()
        } else {
            match name {
                "amp" => Some('&' as u32),
                "lt" => Some('<' as u32),
                "gt" => Some('>' as u32),
                "quot" => Some('"' as u32),
                "apos" => Some('\'' as u32),
                "nbsp" => Some(' ' as u32),
                _ => None,
            }
        };
        code.and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_else(|| caps[0].to_string())
    });
    let text = text.replace('\r', "");
    BLANK_LINES_RE.replace_all(text.trim(), "\n\n").to_string()
}

/// The notes of a change notes page, newest first (the page's own order).
pub fn parse(html: &str) -> Vec<ChangeNote> {
    NOTE_RE
        .captures_iter(html)
        .filter_map(|caps| {
            Some(ChangeNote {
                time: caps[1].parse().ok()?,
                text: html_to_text(&caps[2]),
            })
        })
        .collect()
}

/// Downloads the mod's change notes page (network).
pub fn fetch(workshop_id: &str) -> Result<Vec<ChangeNote>, String> {
    if !crate::modlists::is_workshop_id(workshop_id) {
        return Ok(Vec::new());
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let url = page_url(workshop_id);
    // Steam lets only a few of these pages through in a row from one address and answers
    // the rest with 429; a couple of seconds later it lets the next one in. A client that
    // calls itself Chrome without being one gets 429 every time, so the app gives its own name.
    const TRIES: usize = 3;
    for attempt in 1..=TRIES {
        let answer = agent
            .get(&url)
            .set(
                "User-Agent",
                concat!("rimworld-mod-manager/", env!("CARGO_PKG_VERSION")),
            )
            .set("Accept-Language", "en-US,en;q=0.9")
            .call();
        match answer {
            Ok(page) => return Ok(parse(&page.into_string().map_err(|e| e.to_string())?)),
            Err(ureq::Error::Status(429, _)) if attempt < TRIES => std::thread::sleep(Duration::from_secs(2)),
            Err(ureq::Error::Status(429, _)) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(crate::i18n::text(crate::i18n::TranslationKey::ChangelogTooMany).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_of_a_page() {
        let html = r#"<div class="detailBox workshopAnnouncement noFooter changeLogCtn">
            <div class="changelog headline">Update: 15 Oct @ 5:04am</div>
            <p id="1792065891">v3.0.1<br><br>Faster &amp; smaller.<br />Fixes &quot;A&quot; &#039;B&#039; &lt;C&gt;<br><br><br><br><b>Bold</b> <a href="https://x">link</a></p>
            </div>
            <div class="detailBox workshopAnnouncement noFooter changeLogCtn">
            <p id="1791825433"></p>
            <p class="other">not a note</p>"#;
        let notes = parse(html);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].time, 1792065891);
        assert_eq!(
            notes[0].text,
            "v3.0.1\n\nFaster & smaller.\nFixes \"A\" 'B' <C>\n\nBold link"
        );
        // The author wrote nothing about this update.
        assert_eq!((notes[1].time, notes[1].text.as_str()), (1791825433, ""));
        assert!(parse("<html>Sign in</html>").is_empty());
        assert_eq!(
            page_url("2009463077"),
            "https://steamcommunity.com/sharedfiles/filedetails/changelog/2009463077"
        );
    }

    /// The real page (run by hand: `cargo test live_changelog -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_changelog() {
        let notes = fetch("2009463077").unwrap();
        for n in notes.iter().take(3) {
            println!("{} | {}", n.time, n.text.replace('\n', " / "));
        }
        assert!(!notes.is_empty() && notes.windows(2).all(|w| w[0].time >= w[1].time));
    }
}
