use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

const SYSTEM_PROMPT_TEMPLATE: &str = include_str!("../prompts/system_prompt.txt");
/// Generated from world_api/schema.yaml by tools/gen_world_api.py -- see
/// that file for the single authoritative World API description this is
/// derived from. Regenerating it and rebuilding is enough to update what
/// the model is told; nothing here needs hand-editing when the API changes.
const WORLD_API_COMPACT: &str = include_str!("../world_api/world_api_compact.md");
const NIGHT_HUNT_EXAMPLE: &str = include_str!("../modules/night_hunt.lua");
const REDSTONE_EXAMPLE: &str = include_str!("../modules/redstone_healing.lua");
const STORM_SUMMONER_EXAMPLE: &str = include_str!("../modules/storm_summoner.lua");
const JUMP_RAIN_EXAMPLE: &str = include_str!("../modules/jump_rain.lua");
const ADD_STONE_EXAMPLE: &str = include_str!("../modules/add_stone.lua");

/// These example files also load as real starter rules (`ScriptHost::scan_dir`),
/// so they carry the same `-- api_version: X.Y.Z` leading comment every
/// other rule does. Strip it before showing the model a few-shot example --
/// versioning a rule's source is an engine-side bookkeeping concern (see
/// `world_api_validate::tag_with_api_version`), not something the model
/// should ever need to write itself or copy from an example.
fn strip_api_version_tag(source: &str) -> &str {
    let trimmed = source.trim_start();
    if trimmed.starts_with(crate::world_api_validate::API_VERSION_COMMENT_PREFIX) {
        trimmed.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        source
    }
}

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
            .replace("{WORLD_API_COMPACT}", WORLD_API_COMPACT)
            .replace("{NIGHT_HUNT_EXAMPLE}", strip_api_version_tag(NIGHT_HUNT_EXAMPLE))
            .replace("{REDSTONE_EXAMPLE}", strip_api_version_tag(REDSTONE_EXAMPLE))
            .replace(
                "{STORM_SUMMONER_EXAMPLE}",
                strip_api_version_tag(STORM_SUMMONER_EXAMPLE),
            )
            .replace("{JUMP_RAIN_EXAMPLE}", strip_api_version_tag(JUMP_RAIN_EXAMPLE))
            .replace("{ADD_STONE_EXAMPLE}", strip_api_version_tag(ADD_STONE_EXAMPLE))
    }

    /// One line prepended to the user's own request, telling the model
    /// which of the two module contracts to write (see `PromptKind`)
    /// instead of leaving it to guess. `App::start_generation` picks `kind`
    /// via the cheap deterministic `classify_prompt` heuristic; if the
    /// model's output doesn't end up matching, `App::poll_generation` asks
    /// for one corrective retry, same as any other validation failure.
    fn directive_for(kind: PromptKind) -> &'static str {
        match kind {
            PromptKind::Rule => {
                "This is a persistent RULE request: define `function on_tick(api)`."
            }
            PromptKind::Instant => {
                "This is a one-time INSTANT SPELL request: define `function on_cast(api, event)` \
                 instead of on_tick. Its whole effect must happen once, immediately -- no \
                 condition-checking, no waiting for a future tick."
            }
        }
    }

    fn build_user_turn(user_request: &str, kind: PromptKind) -> String {
        format!("{}\n\nRequest: {user_request}", Self::directive_for(kind))
    }

    /// Kicks off a background request generating a brand-new module (a
    /// continuous rule or an instant spell, per `kind`) from a
    /// natural-language description. Poll the returned handle once per
    /// frame; never blocks the caller.
    pub fn generate(&self, user_request: &str, kind: PromptKind) -> PendingGeneration {
        let messages = vec![
            ChatMessage::system(Self::system_prompt()),
            ChatMessage::user(Self::build_user_turn(user_request, kind)),
        ];
        self.spawn_request(messages)
    }

    /// Kicks off a background retry: gives the model its previous (invalid,
    /// or wrong-contract) output plus an error/correction message so it can
    /// fix itself. Used for exactly one retry -- see `App::poll_generation`.
    pub fn retry(
        &self,
        user_request: &str,
        kind: PromptKind,
        broken_code: &str,
        error: &str,
    ) -> PendingGeneration {
        let messages = vec![
            ChatMessage::system(Self::system_prompt()),
            ChatMessage::user(Self::build_user_turn(user_request, kind)),
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

/// Which of the two module contracts a prompt should generate -- see
/// world_api/schema.yaml's `events.on_tick`/`events.on_cast`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// A continuous rule (`on_tick`): reacts to ongoing world/player state.
    Rule,
    /// A one-time instant spell (`on_cast`): a single immediate action.
    Instant,
}

/// Conditional/temporal phrasing that marks a prompt as describing ongoing
/// behavior rather than a one-time action -- checked first (and wins over
/// any instant-leaning word below) since "when it jumps, add 10 stone" is a
/// rule that happens to contain an instant-looking verb, not the reverse.
/// Multi-word so a substring check is enough; single "if"/"when" would
/// false-positive on unrelated words.
const RULE_PHRASES: &[&str] = &[
    "when ", "whenever ", " if ", " while ", " during ", "every time",
    "each time", "as long as", "at night", "at day", "at dawn", "at dusk",
    "at sunset", "at sunrise",
];

/// Imperative verbs that open a one-shot command ("add 100 stone", "spawn
/// a dozen chickens", "give me some wood") -- checked only against the
/// prompt's first word, matching how an imperative English sentence reads,
/// so a rule like "spawning should stop at night" isn't misread as one.
const INSTANT_FIRST_WORDS: &[&str] = &[
    "add", "give", "grant", "spawn", "summon", "heal", "clear", "remove",
    "delete", "kill", "fill", "place", "drop", "make", "set", "teleport",
    "cast", "poison", "cure", "damage", "boost",
];

/// Cheap, deterministic heuristic guessing whether `prompt` describes an
/// ongoing RULE or a one-time INSTANT SPELL -- the same kind of
/// keyword-based approach `derive_rule_name` already uses, not real
/// language understanding. Used only to bias which contract the model is
/// asked to write (see `LlmClient::directive_for`); a wrong guess isn't
/// fatal, since `App::poll_generation` accepts whatever contract the model
/// actually produces after at most one corrective retry. Defaults to
/// `Rule` for anything ambiguous, matching this project's behavior before
/// instant spells existed (every prompt used to become a rule).
pub fn classify_prompt(prompt: &str) -> PromptKind {
    let lower = format!(" {} ", prompt.trim().to_ascii_lowercase());
    if RULE_PHRASES.iter().any(|p| lower.contains(p)) {
        return PromptKind::Rule;
    }
    let first_word = lower
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric());
    if INSTANT_FIRST_WORDS.contains(&first_word) {
        return PromptKind::Instant;
    }
    PromptKind::Rule
}

/// Low-content English words dropped when deriving a short rule name from a
/// prompt -- articles, prepositions, pronouns, and filler verbs that don't
/// describe *what* the rule actually does. "player"/"players" is included
/// since nearly every rule mentions one, so it's rarely the distinguishing
/// word.
const NAME_STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "to", "of", "in", "on",
    "at", "by", "for", "with", "and", "or", "but", "if", "then", "when", "while", "that", "this",
    "these", "those", "it", "its", "their", "they", "he", "she", "him", "her", "has", "have",
    "had", "will", "would", "should", "could", "can", "do", "does", "did", "not", "no", "so",
    "than", "as", "from", "into", "onto", "out", "up", "down", "near", "within", "during", "gets",
    "get", "got", "give", "gives", "given", "start", "starts", "started", "begin", "begins",
    "cause", "causes", "caused", "make", "makes", "made", "one", "two", "three", "four", "five",
    "six", "seven", "eight", "nine", "ten", "some", "any", "all", "each", "every", "also", "just",
    "only", "very", "really", "much", "many", "more", "most", "less", "least", "around", "about",
    "player", "players",
];

/// Shortest word considered meaningful enough to anchor a name -- drops
/// short filler nouns ("day", "one", ...) without having to enumerate every
/// one of them by hand.
const NAME_MIN_WORD_LEN: usize = 4;
/// Per-word cap, so one long word can't blow out an otherwise-short name.
const NAME_MAX_WORD_LEN: usize = 12;
const NAME_WORD_COUNT: usize = 2;

/// Derives a short, snake_case name from a natural-language rule prompt for
/// use as the module's default display name (e.g. "chickens flee from the
/// nearest player" -> "chickens_flee"). This is a plain keyword heuristic,
/// not real language understanding -- it picks the first couple of
/// sufficiently long, non-filler words in the order they appear, which
/// tends to track the subject/action of the sentence well enough to beat a
/// generic "rule_3" without depending on the (unreliable, see
/// `live_generation_produces_a_valid_module`) local model for yet another
/// thing to get right. Returns `None` if no word survives filtering (e.g.
/// an all-stopword or all-punctuation prompt), so the caller can fall back
/// to a numbered name.
pub fn derive_rule_name(prompt: &str) -> Option<String> {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| {
            w.len() >= NAME_MIN_WORD_LEN
                && !NAME_STOPWORDS.contains(&w.as_str())
                && w.chars().any(|c| c.is_alphabetic())
        })
        .map(|w| w.chars().take(NAME_MAX_WORD_LEN).collect::<String>())
        .take(NAME_WORD_COUNT)
        .collect();

    if words.is_empty() {
        None
    } else {
        Some(words.join("_"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_embeds_the_world_api_doc_and_strips_example_version_tags() {
        let prompt = LlmClient::system_prompt();
        assert!(
            prompt.contains("api.replace_block"),
            "expected the generated World API doc to be spliced into the system prompt"
        );
        assert!(
            !prompt.contains("api_version:"),
            "the model should never see the engine-internal api_version tag \
             in a few-shot example -- it's added automatically after generation, \
             not something the model should write or copy"
        );
        assert!(
            prompt.contains("function on_tick(api)"),
            "expected the example modules' actual content to still be present after stripping the tag"
        );
        assert!(
            prompt.contains("function on_cast(api, event)"),
            "expected the add_stone.lua instant-spell example to be spliced into the system prompt"
        );
    }

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

    #[test]
    fn derive_rule_name_picks_the_first_meaningful_words_in_order() {
        assert_eq!(
            derive_rule_name(
                "During the day, chickens flee from the nearest player if it gets within 6 blocks."
            ),
            Some("chickens_flee".to_string())
        );
        assert_eq!(
            derive_rule_name("when a player jumps, it starts raining"),
            Some("jumps_raining".to_string())
        );
        assert_eq!(
            derive_rule_name("breaking a crystal block while sprinting summons a storm"),
            Some("breaking_crystal".to_string())
        );
    }

    #[test]
    fn derive_rule_name_drops_stopwords_and_short_filler_words() {
        // "three" is a spelled-out number (filtered) and "in"/"a" are
        // stopwords -- the name should skip straight to the real content.
        assert_eq!(
            derive_rule_name("three creature deaths in one location spawn a red stone"),
            Some("creature_deaths".to_string())
        );
    }

    #[test]
    fn derive_rule_name_falls_back_to_none_for_an_all_filler_prompt() {
        assert_eq!(derive_rule_name("if it is on the a"), None);
        assert_eq!(derive_rule_name("   "), None);
    }

    #[test]
    fn derive_rule_name_uses_a_single_word_when_only_one_survives() {
        assert_eq!(derive_rule_name("make it rain"), Some("rain".to_string()));
    }

    #[test]
    fn derive_rule_name_caps_an_overly_long_word() {
        let long_word = "a".repeat(30);
        let name = derive_rule_name(&format!("the {long_word} block explodes")).unwrap();
        assert!(
            name.split('_').all(|w| w.len() <= NAME_MAX_WORD_LEN),
            "expected every word capped at {NAME_MAX_WORD_LEN} chars: {name}"
        );
    }

    #[test]
    fn classify_prompt_recognizes_conditional_and_temporal_rules() {
        assert_eq!(
            classify_prompt(
                "During the day, chickens flee from the nearest player if it gets within 6 blocks."
            ),
            PromptKind::Rule
        );
        assert_eq!(
            classify_prompt("At night, sheep hunt players carrying a crystal."),
            PromptKind::Rule
        );
        assert_eq!(
            classify_prompt("When a player jumps, it starts raining"),
            PromptKind::Rule
        );
        assert_eq!(
            classify_prompt("During rain, soil near trees becomes mud that slows players."),
            PromptKind::Rule
        );
    }

    #[test]
    fn classify_prompt_recognizes_imperative_one_shot_actions_as_instant() {
        assert_eq!(classify_prompt("add 100 stone"), PromptKind::Instant);
        assert_eq!(
            classify_prompt("spawn 12 chickens around me"),
            PromptKind::Instant
        );
        assert_eq!(
            classify_prompt("give me 50 wood to my inventory"),
            PromptKind::Instant
        );
        assert_eq!(classify_prompt("heal me"), PromptKind::Instant);
        assert_eq!(classify_prompt("make it rain"), PromptKind::Instant);
        assert_eq!(classify_prompt("kill all nearby sheep"), PromptKind::Instant);
        assert_eq!(
            classify_prompt("poison the nearest player"),
            PromptKind::Instant
        );
        assert_eq!(classify_prompt("cure my poison"), PromptKind::Instant);
        assert_eq!(
            classify_prompt("boost my jump height"),
            PromptKind::Instant
        );
    }

    #[test]
    fn classify_prompt_prefers_rule_when_a_conditional_phrase_and_an_instant_verb_both_appear() {
        // "make" opens the sentence, which alone would read as an instant
        // command, but the "when" clause means this is really describing
        // ongoing behavior -- conditional phrasing must win.
        assert_eq!(
            classify_prompt("make it rain whenever a player jumps"),
            PromptKind::Rule
        );
    }

    #[test]
    fn classify_prompt_defaults_to_rule_for_an_ambiguous_prompt() {
        assert_eq!(
            classify_prompt("sheep are afraid of red light"),
            PromptKind::Rule
        );
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
        let prompt = "During the day, chickens flee from the nearest player if it gets within 6 blocks.";
        let pending = client.generate(prompt, classify_prompt(prompt));

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
        assert!(
            !module.is_instant,
            "a conditional prompt like this one should generate a RULE (on_tick), not a spell"
        );
    }

    /// Same pipeline as `live_generation_produces_a_valid_module`, but for
    /// an INSTANT SPELL request -- confirms classify_prompt's guess plus
    /// the directive it drives actually gets the model to write on_cast
    /// instead of on_tick for a real one-shot prompt, not just in the
    /// (mocked/hand-written) unit tests. Same manual-run requirement.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_produces_a_valid_instant_spell_module() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "add 100 stone";
        let kind = classify_prompt(prompt);
        assert_eq!(kind, PromptKind::Instant, "sanity check on the classifier itself");
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_spell".to_string(), "test".to_string(), code)
            .expect("generated module should pass validation");
        assert!(
            module.is_instant,
            "expected \"add 100 stone\" to generate an INSTANT SPELL (on_cast), not a rule"
        );
    }

    /// Same pipeline again, checking the model actually reaches for the new
    /// 1.2.0 player-health/poison World API (not just that it produces
    /// *some* valid module) when the prompt calls for it.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_uses_the_poison_api_for_a_poison_prompt() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "poison the nearest player";
        let kind = classify_prompt(prompt);
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_poison".to_string(), "test".to_string(), code.clone())
            .expect("generated module should pass validation");
        assert!(
            code.contains("set_poisoned"),
            "expected \"poison the nearest player\" to call api.set_poisoned somewhere: {code}"
        );
        assert!(!module.name.is_empty());
    }

    /// Same pipeline once more, for the 1.3.0 stone_golem creature kind.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_uses_the_stone_golem_kind_for_a_summon_prompt() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "summon a stone golem near me";
        let kind = classify_prompt(prompt);
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_golem".to_string(), "test".to_string(), code.clone())
            .expect("generated module should pass validation");
        assert!(
            code.contains("stone_golem"),
            "expected \"summon a stone golem near me\" to reference the stone_golem kind: {code}"
        );
        assert!(!module.name.is_empty());
    }

    /// Same pipeline once more, for the 1.7.0 wolf creature kind.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_uses_the_wolf_kind_for_a_summon_prompt() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "summon a wolf near me";
        let kind = classify_prompt(prompt);
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_wolf".to_string(), "test".to_string(), code.clone())
            .expect("generated module should pass validation");
        assert!(
            code.contains("wolf"),
            "expected \"summon a wolf near me\" to reference the wolf kind: {code}"
        );
        assert!(!module.name.is_empty());
    }

    /// Same pipeline once more, for the 1.8.0 stinger creature kind.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_uses_the_stinger_kind_for_a_summon_prompt() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "summon a stinger near me";
        let kind = classify_prompt(prompt);
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_stinger".to_string(), "test".to_string(), code.clone())
            .expect("generated module should pass validation");
        assert!(
            code.contains("stinger"),
            "expected \"summon a stinger near me\" to reference the stinger kind: {code}"
        );
        assert!(!module.name.is_empty());
    }

    /// Same pipeline once more, for the 1.9.0 goblin creature kind.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_uses_the_goblin_kind_for_a_summon_prompt() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "summon a goblin near me";
        let kind = classify_prompt(prompt);
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_goblin".to_string(), "test".to_string(), code.clone())
            .expect("generated module should pass validation");
        assert!(
            code.contains("goblin"),
            "expected \"summon a goblin near me\" to reference the goblin kind: {code}"
        );
        assert!(!module.name.is_empty());
    }

    /// Same pipeline once more, for the 1.5.0 five-weather system.
    #[test]
    #[ignore = "requires a running llama-server on 127.0.0.1:8090"]
    fn live_generation_uses_the_new_weather_names() {
        use crate::scripting::Module;
        use std::time::{Duration, Instant};

        let client = LlmClient::new("http://127.0.0.1:8090".to_string());
        let prompt = "when it's stormy, spawn a sheep near every player";
        let kind = classify_prompt(prompt);
        let pending = client.generate(prompt, kind);

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
        let module = Module::load("test_weather".to_string(), "test".to_string(), code.clone())
            .expect("generated module should pass validation");
        assert!(
            code.contains("\"storm\""),
            "expected a storm-conditioned rule to check api.weather == \"storm\": {code}"
        );
        assert!(!module.is_instant, "a \"when X\" rule should be on_tick, not on_cast");
    }
}
