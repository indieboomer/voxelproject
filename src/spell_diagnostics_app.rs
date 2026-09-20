//! Local diagnostic records live alongside player comments in the saved chat export.
//! They are not broadcast or drawn as chat bubbles.
use super::*;
use serde_json::{json, Value};

pub(super) fn unix_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis()
}

fn source_key(source: &str) -> String {
    // Stable correlation label, not a security hash.
    let hash = source.bytes().fold(0xcbf29ce484222325u64,
        |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3));
    format!("{hash:016x}")
}

fn record(sequence: usize, stage: &str, data: Value) -> String {
    format!("[spell-diagnostic] {}", json!({
        "schema": 1, "sequence": sequence, "unix_ms": unix_ms(), "stage": stage, "data": data,
    }))
}

impl App {
    pub(super) fn spell_diagnostic(&mut self, stage: &str, data: Value) -> usize {
        let sequence = self.chat_transcript.len() + 1;
        self.chat_transcript.push(record(sequence, stage, data));
        sequence
    }

    pub(super) fn diagnose_source(&mut self, name: &str, prompt: &str, source: &str) -> String {
        let key = source_key(source);
        if self.diagnostic_sources.insert(source.to_owned()) {
            self.spell_diagnostic("source", json!({
                "code_id": key, "name": name, "prompt": prompt, "lua": source,
            }));
        }
        key
    }

    pub(super) fn diagnose_saved_cast(&mut self, id: u64, route: &str) -> usize {
        let spell = self.scripting.spellbook.get(id).cloned();
        let code = spell.as_ref().map(|s| self.diagnose_source(&s.name, &s.original_prompt, &s.source));
        let context = self.aimed_spell_context();
        self.spell_diagnostic("cast_attempt", json!({
            "route": route, "spell_id": id, "code_id": code,
            "name": spell.as_ref().map(|s| &s.name), "revision": spell.as_ref().map(|s| s.revision),
            "required_target": spell.as_ref().map(|s| s.target.label()),
            "target": context.map(|c| c.target), "position": self.player.position.to_array(),
            "mana_before": self.player.crafting.mana, "health": self.player.health,
            "time_of_day": self.time_of_day, "weather": format!("{:?}", self.weather.current),
        }))
    }

    pub(super) fn diagnose_cast_effects(&mut self, outcome: &TickOutcome) {
        self.spell_diagnostic("cast_effects", json!({
            "block_edit_count": outcome.block_edits.len(),
            "block_edits_sample": outcome.block_edits.iter().take(32).collect::<Vec<_>>(),
            "player_effects": outcome.player_effects.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
            "warnings": outcome.warnings, "errors": outcome.crashes, "broadcasts": outcome.broadcasts,
            "time_of_day_after": self.time_of_day, "weather_after": format!("{:?}", self.weather.current),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_code_and_comments_remain_single_line_and_recoverable() {
        let source = "function on_cast(api, event)\n  error(\"failed\")\nend";
        let line = record(7, "source", json!({"lua": source, "comment": "didn't heal\nsecond line"}));
        assert_eq!(line.lines().count(), 1);
        let data: Value = serde_json::from_str(line.strip_prefix("[spell-diagnostic] ").unwrap()).unwrap();
        assert_eq!(data["data"]["lua"], source);
        assert_eq!(data["sequence"], 7);
        assert_eq!(source_key(source), source_key(source));
        assert_ne!(source_key(source), source_key("function on_cast() end"));
    }
}
