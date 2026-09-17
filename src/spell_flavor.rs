//! Original, cosmetic flavor text. Bounded for saves, card layout and catalogs.
use serde::Deserialize;
pub const MAX_BYTES: usize = 96;
pub const INSTRUCTIONS:&str="Write flavor_quote: one original, poetic fantasy sentence about the spell, like a fragment of a folktale or a whispered legend. 6-14 words, at most 96 UTF-8 bytes. Use imagery, personification or a tiny implied story, not a literal summary of what the spell does. Evoke its subject and atmosphere; do not explain mechanics, costs or controls. No heading, attribution, quotation marks, markup, or copied quotation. Tone examples (invent a new line): stone gift -> Even mountains part with their memories.; healing -> The wound forgot its sorrow beneath her hand.; moonlit sheep -> The moon counted its dreams and found a flock. Do not change gameplay to fit the story.";

pub fn clean(value: &str) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text = text.as_str();
    for (open, close) in [("\"", "\""), ("'", "'"), ("“", "”")] {
        if text.len() >= open.len() + close.len() && text.starts_with(open) && text.ends_with(close)
        {
            text = &text[open.len()..text.len() - close.len()];
            break;
        }
    }
    let mut end = text.len().min(MAX_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end < text.len() {
        if let Some(space) = text[..end].rfind(' ') {
            end = space;
        }
    }
    text[..end]
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}
pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value.as_str().map(clean).unwrap_or_default())
}
pub fn from_source(source: &str) -> String {
    source
        .lines()
        .find_map(|s| s.strip_prefix("-- Intent plan: "))
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("flavor_quote").and_then(|v| v.as_str()).map(clean))
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the local llama-server; tests quote generation for an existing spell"]
    fn live_flavor_quote_for_existing_spell() {
        let quote = crate::llm::request_flavor_quote(
            crate::net::DEFAULT_LLM_URL,
            "Moonlit Flock: summon three sheep beneath the moon.",
        )
        .unwrap();
        assert!(!quote.is_empty() && quote.len() <= MAX_BYTES);
        println!("Existing spell quote: {quote}");
    }
    #[test]
    fn quotes_are_single_line_utf8_bounded_and_legacy_sources_are_empty() {
        assert_eq!(
            clean("The moon whispered, 'Come home.'"),
            "The moon whispered, 'Come home.'"
        );
        assert_eq!(
            clean("  \"The earth\n remembers.\"  "),
            "The earth remembers."
        );
        let quote = clean(&"Żółw 🌙 ".repeat(40));
        assert!(quote.len() <= MAX_BYTES);
        assert!(!quote.contains('\n'));
        assert!(from_source("function on_cast(api,event) end").is_empty());
        assert_eq!(
            from_source(r#"-- Intent plan: {"flavor_quote":"Stone remembers the mountain."}"#),
            "Stone remembers the mountain."
        );
    }
}
