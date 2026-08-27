use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

const SYSTEM_PROMPT_TEMPLATE: &str = include_str!("../prompts/system_prompt.txt");
const NIGHT_HUNT_EXAMPLE: &str = include_str!("../modules/night_hunt.lua");
const REDSTONE_EXAMPLE: &str = include_str!("../modules/redstone_healing.lua");
const STORM_SUMMONER_EXAMPLE: &str = include_str!("../modules/storm_summoner.lua");

struct ChatMessage {
    role: &'static str,
    content: String,
}

impl ChatMessage {
    fn system(content: String) -> Self {
        Self {
            role: "system",
            content,
        }
    }
    fn user(content: String) -> Self {
        Self {
            role: "user",
            content,
        }
    }
    fn assistant(content: String) -> Self {
        Self {
            role: "assistant",
            content,
        }
    }
}

/// Talks to a locally running `llama-server` (llama.cpp) over its
/// OpenAI-compatible chat completions endpoint. Which GGUF model actually
/// answers is whatever the user pointed llama-server at -- this client
/// doesn't care, matching the README's "configurable GGUF coding model".
pub struct LlmClient {
    base_url: String,
}

impl LlmClient {
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }

    fn system_prompt() -> String {
        SYSTEM_PROMPT_TEMPLATE
            .replace("{NIGHT_HUNT_EXAMPLE}", NIGHT_HUNT_EXAMPLE)
            .replace("{REDSTONE_EXAMPLE}", REDSTONE_EXAMPLE)
            .replace("{STORM_SUMMONER_EXAMPLE}", STORM_SUMMONER_EXAMPLE)
    }

    /// Kicks off a background request generating a brand-new rule module
    /// from a natural-language description. Poll the returned handle once
    /// per frame; never blocks the caller.
    pub fn generate(&self, user_request: &str) -> PendingGeneration {
        let messages = vec![
            ChatMessage::system(Self::system_prompt()),
            ChatMessage::user(user_request.to_string()),
        ];
        self.spawn_request(messages)
    }

    /// Kicks off a background retry: gives the model its previous (invalid)
    /// output plus the validation error so it can correct itself. Used for
    /// exactly one retry -- see `App::poll_generation`.
    pub fn retry(&self, user_request: &str, broken_code: &str, error: &str) -> PendingGeneration {
        let messages = vec![
            ChatMessage::system(Self::system_prompt()),
            ChatMessage::user(user_request.to_string()),
            ChatMessage::assistant(broken_code.to_string()),
            ChatMessage::user(format!(
                "That code failed validation with this error:\n{error}\n\nFix it and output only the corrected Lua source, no explanation."
            )),
        ];
        self.spawn_request(messages)
    }

    fn spawn_request(&self, messages: Vec<ChatMessage>) -> PendingGeneration {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = request_completion(&url, messages);
            let _ = tx.send(result);
        });
        PendingGeneration { receiver: rx }
    }
}

/// A generation request in flight on a background thread. `ureq` is
/// blocking and `mlua`'s types aren't `Send`, so the HTTP call happens on
/// its own thread and only the resulting Lua source text (or error string)
/// crosses back over the channel; validation still happens on the main
/// thread, where the Lua VM actually gets created.
pub struct PendingGeneration {
    receiver: Receiver<Result<String, String>>,
}

impl PendingGeneration {
    pub fn poll(&self) -> Option<Result<String, String>> {
        self.receiver.try_recv().ok()
    }
}

fn request_completion(url: &str, messages: Vec<ChatMessage>) -> Result<String, String> {
    let body = serde_json::json!({
        "messages": messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect::<Vec<_>>(),
        "temperature": 0.2,
        "max_tokens": 800,
    });

    let response = ureq::post(url)
        .timeout(Duration::from_secs(120))
        .send_json(body)
        .map_err(|e| format!("request to {url} failed: {e} (is llama-server running?)"))?;

    let json: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("couldn't parse response as JSON: {e}"))?;

    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("unexpected response shape: {json}"))?;

    Ok(extract_lua(content))
}

/// Strips a ```lua fenced block if the model wrapped its answer in one,
/// otherwise returns the trimmed text as-is.
fn extract_lua(text: &str) -> String {
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        let after = after.strip_prefix("lua").unwrap_or(after);
        let after = after.strip_prefix('\n').unwrap_or(after);
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
        return after.trim().to_string();
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_lua_strips_fenced_code_blocks() {
        let text = "Here you go:\n```lua\nfunction on_tick(api) end\n```\nHope that helps!";
        assert_eq!(extract_lua(text), "function on_tick(api) end");
    }

    #[test]
    fn extract_lua_passes_through_plain_code() {
        let text = "function on_tick(api) end";
        assert_eq!(extract_lua(text), "function on_tick(api) end");
    }

    /// Exercises the real end-to-end pipeline: a live llama-server generates
    /// Lua for a rule, and our real sandbox (`Module::load`) validates it.
    /// Requires `llama-server` running on 127.0.0.1:8090 -- not run by
    /// default `cargo test`; run with `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_produces_a_valid_module() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let pending = client.generate(
            "During the day, chickens flee from the nearest player if it gets within 6 blocks.",
        );

        let deadline = Instant::now() + Duration::from_secs(90);
        let result = loop {
            if let Some(r) = pending.poll() {
                break r;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for llama-server");
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        let code = result.expect("generation request should succeed");
        println!("--- generated Lua ---\n{code}\n----------------------");
        let module = Module::load("test_rule".to_string(), "test".to_string(), code)
            .expect("generated module should pass validation");
        assert!(!module.name.is_empty());
    }
}
