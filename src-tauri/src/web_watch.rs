//! The web watch (feature `web-watch`): fetches a page, reads the text of
//! the first element with a given id or class, and returns it (and, read as
//! a number, the first number in it). A Trigger (`src/automation.rs`)
//! checks it on its own interval and fires when the value meets its
//! condition.

use std::time::Duration;

use dom_query::Document;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::job_kinds::{http_get, HttpError, Url};

/// Largest page read.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
/// Longest text kept as the value.
const MAX_VALUE_CHARS: usize = 1000;
/// Longest a single check may take.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
/// Shortest interval between two checks of one watch, so a web site is not
/// asked too often.
pub const MIN_EVERY_SECONDS: u64 = 30;

/// Whether the value is the element's text or the first number in it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadAs {
    #[default]
    Text,
    Number,
}

/// What to watch, as saved in jobs.json.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebWatch {
    /// http:// or https:// address of the page.
    pub url: String,
    pub find: FindBy,
    /// The id or class name, without # or a dot.
    pub name: String,
    #[serde(default)]
    pub read_as: ReadAs,
}

impl WebWatch {
    pub fn check(&self) -> Result<(), String> {
        if self.url.len() > 2000 {
            return Err("the address is over 2000 characters".into());
        }
        Url::parse(&self.url)?;
        check_name(self.find, &self.name)
    }

    /// `#name` or `.name`, as people write it.
    pub fn shown(&self) -> String {
        match self.find {
            FindBy::Id => format!("#{}", self.name),
            FindBy::Class => format!(".{}", self.name),
        }
    }

    /// Fetches the page and reads the value: `{"text": ...}`, plus
    /// `"number"` when read as a number. Errors are sentences for people.
    pub async fn read(&self) -> Result<Value, String> {
        let (_, reply) = http_get(&self.url, MAX_PAGE_BYTES).await.map_err(|e| match e {
            HttpError::Unreachable(why) => format!("could not reach {} ({why})", self.url),
            HttpError::Other(why) => format!("{}: {why}", self.url),
        })?;
        if !(200..300).contains(&reply.status) {
            return Err(format!("{} answered HTTP {}", self.url, reply.status));
        }
        let page = String::from_utf8_lossy(&reply.body);
        let Some(text) = find_text(&page, self.find, &self.name)? else {
            return Err(format!("{} is not on {}", self.shown(), self.url));
        };
        let text: String = text.chars().take(MAX_VALUE_CHARS).collect();
        match self.read_as {
            ReadAs::Text => Ok(json!({ "text": text })),
            ReadAs::Number => match read_number(&text) {
                Some(number) => Ok(json!({ "text": text, "number": number })),
                None => Err(format!("{} has no number in it: {text}", self.shown())),
            },
        }
    }
}

/// How the element is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindBy {
    Id,
    Class,
}

/// Longest id or class name accepted.
const MAX_NAME: usize = 200;

/// Checks an id or class name. A class is one word (an element's class
/// attribute is a list of words separated by spaces).
pub fn check_name(by: FindBy, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the id or class name is empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("the id or class name is over {MAX_NAME} characters"));
    }
    if name.chars().any(char::is_control) {
        return Err("the id or class name contains a control character".into());
    }
    if by == FindBy::Class && name.chars().any(char::is_whitespace) {
        return Err("a class name is one word, without spaces".into());
    }
    Ok(())
}

/// An attribute selector for the name: `[id="name"]` matches the whole id,
/// `[class~="name"]` one word of the class list. Quotes and backslashes are
/// escaped, so any checked name gives a valid selector.
fn selector(by: FindBy, name: &str) -> String {
    let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
    match by {
        FindBy::Id => format!("[id=\"{quoted}\"]"),
        FindBy::Class => format!("[class~=\"{quoted}\"]"),
    }
}

/// The text of the first matching element, including the text of elements
/// inside it, with runs of white space (line breaks too) turned into one
/// space. `None` when nothing matches.
pub fn find_text(html: &str, by: FindBy, name: &str) -> Result<Option<String>, String> {
    check_name(by, name)?;
    let doc = Document::from(html);
    let Some(found) = doc.try_select(&selector(by, name)) else {
        return Ok(None);
    };
    let text = found.first().text();
    Ok(Some(text.split_whitespace().collect::<Vec<_>>().join(" ")))
}

/// The first number in `text`. A minus sign directly before it, or before a
/// currency sign in front of it, counts ("-£5", "£-5"). Commas between
/// groups of three digits are thousands separators ("1,234,567.89"). The
/// European style "1.234,56" is read as 1.234: the comma ends the number.
pub fn read_number(text: &str) -> Option<f64> {
    let chars: Vec<char> = text.chars().collect();
    let start = chars.iter().position(|c| c.is_ascii_digit())?;
    let minus = |c: char| c == '-' || c == '\u{2212}';
    let currency = |c: char| matches!(c, '$' | '£' | '€' | '¥');
    let negative = match start.checked_sub(1).map(|i| chars[i]) {
        Some(c) if minus(c) => true,
        Some(c) if currency(c) => start.checked_sub(2).is_some_and(|i| minus(chars[i])),
        _ => false,
    };
    let mut digits = String::new();
    let mut seen_point = false;
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            digits.push(c);
        } else if c == '.' && !seen_point && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            seen_point = true;
            digits.push('.');
        } else if c == ',' && !seen_point && thousands_group(&chars[i + 1..]) {
            // A thousands separator: skipped.
        } else {
            break;
        }
        i += 1;
    }
    let value: f64 = digits.parse().ok()?;
    let value = if negative { -value } else { value };
    value.is_finite().then_some(value)
}

/// Exactly three digits follow, then no further digit.
fn thousands_group(rest: &[char]) -> bool {
    rest.len() >= 3 && rest[..3].iter().all(char::is_ascii_digit) && !rest.get(3).is_some_and(char::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kim's probe page, as served on 16 November 2026.
    const PROBE: &str = "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>probe</title></head><body>\n<p><span id=\"probe-token\">8a199ec20bbbddb426dcb9b38a3bdae8\n2026-11-16 11:40:01</span></p>\n</body></html>";

    #[test]
    fn finds_the_probe_token_by_id() {
        assert_eq!(
            find_text(PROBE, FindBy::Id, "probe-token").unwrap().as_deref(),
            Some("8a199ec20bbbddb426dcb9b38a3bdae8 2026-11-16 11:40:01"),
            "the line break becomes one space"
        );
        assert_eq!(find_text(PROBE, FindBy::Id, "probe").unwrap(), None, "an id must match whole");
        assert_eq!(find_text(PROBE, FindBy::Class, "probe-token").unwrap(), None, "that is an id, not a class");
    }

    #[test]
    fn finds_by_class_word_and_takes_the_first_match() {
        let page = r#"<div class="card price  big">£1,234.<b>56</b></div><div class="price">£9</div>"#;
        assert_eq!(find_text(page, FindBy::Class, "price").unwrap().as_deref(), Some("£1,234.56"), "text inside <b> is included");
        assert_eq!(find_text(page, FindBy::Class, "big").unwrap().as_deref(), Some("£1,234.56"));
        assert_eq!(find_text(page, FindBy::Class, "pri").unwrap(), None, "a class must match a whole word");
        assert_eq!(find_text("<p class=\"a\"></p>", FindBy::Class, "a").unwrap().as_deref(), Some(""), "found but empty");
    }

    #[test]
    fn awkward_names_and_pages_never_panic() {
        assert!(find_text(PROBE, FindBy::Id, "").is_err());
        assert!(find_text(PROBE, FindBy::Class, "two words").is_err());
        assert!(find_text(PROBE, FindBy::Id, &"x".repeat(201)).is_err());
        assert!(find_text(PROBE, FindBy::Id, "a\nb").is_err());
        let odd = r#"<i id='say "hi"'>1</i><i id="back\slash">2</i><i id="1st:item[0]">3</i>"#;
        assert_eq!(find_text(odd, FindBy::Id, "say \"hi\"").unwrap().as_deref(), Some("1"));
        assert_eq!(find_text(odd, FindBy::Id, "back\\slash").unwrap().as_deref(), Some("2"));
        assert_eq!(find_text(odd, FindBy::Id, "1st:item[0]").unwrap().as_deref(), Some("3"));
        for junk in ["", "<<<", "<p id=x>unclosed", "\u{0}\u{1}", "<svg><p id=\"x\">in svg</p></svg>"] {
            let _ = find_text(junk, FindBy::Id, "x").unwrap();
        }
    }

    #[test]
    fn reads_numbers_with_currency_and_thousands() {
        let cases: [(&str, Option<f64>); 16] = [
            ("£1,234.56", Some(1234.56)),
            ("$ 99", Some(99.0)),
            ("€0.99", Some(0.99)),
            ("¥12,345,678", Some(12345678.0)),
            ("-£5.50", Some(-5.5)),
            ("£-5.50", Some(-5.5)),
            ("\u{2212}3", Some(-3.0)),
            ("Price: 42 items", Some(42.0)),
            ("BTC 97,500.25 USD (+1.2%)", Some(97500.25)),
            ("1,23", Some(1.0)),
            ("1,2345", Some(1.0)),
            ("1.234,56", Some(1.234)),
            ("version 2.", Some(2.0)),
            ("pre-order 5", Some(5.0)),
            ("no digits here", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(read_number(text), want, "{text:?}");
        }
        assert_eq!(read_number("8a199ec2 2026-11-16"), Some(8.0), "the probe token starts with a digit");
        assert_eq!(read_number("SKU-5"), Some(-5.0), "known limit: a hyphen right before the digits is a minus");
    }
}
