pub fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '\'' | '-'))
        .take(crate::net::MAX_NICKNAME_LEN)
        .collect::<String>()
        .trim()
        .to_string()
}
pub fn fallback() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    let first = ["Sir", "Dame", "Baron", "Wizard", "Captain", "Count"];
    let last = [
        "Wobbleboots",
        "Turnipbeard",
        "Mossbritches",
        "Noodlewand",
        "Picklehelm",
        "Snoreforge",
    ];
    format!("{} {}", first[seed % 6], last[(seed / 7) % 6])
}
pub fn request(url: &str) -> Result<String, String> {
    let url = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let json:serde_json::Value=ureq::post(&url).timeout(std::time::Duration::from_secs(12)).send_json(serde_json::json!({
        "messages":[{"role":"user","content":"Invent one funny, friendly fantasy adventurer name, under 24 characters. Return only the name, no explanation or quotes."}],"temperature":1.0,"max_tokens":32
    })).map_err(|e|e.to_string())?.into_json().map_err(|e|e.to_string())?;
    let raw = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("No name returned")?;
    if raw.lines().count() != 1 || raw.chars().count() > 48 {
        return Err("Invalid name response".into());
    }
    let name = clean(raw);
    if name.is_empty() {
        Err("Empty name response".into())
    } else {
        Ok(name)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn names_are_bounded_and_have_no_control_characters() {
        assert_eq!(super::clean("  Sir\n Turnip!  "), "Sir Turnip");
        assert!(super::clean(&"x".repeat(100)).chars().count() <= crate::net::MAX_NICKNAME_LEN);
        for _ in 0..20 {
            let n = super::fallback();
            assert!(!n.is_empty());
            assert_eq!(n, super::clean(&n));
        }
    }
}
