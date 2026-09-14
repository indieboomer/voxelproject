//! Pre-flight checks for LLM-generated Lua, run *before* the source is ever
//! loaded into a real Lua VM (see `App::poll_generation`). Catches the most
//! common way a small local model goes wrong -- calling an `api.*` member
//! that doesn't exist (hallucinated or misspelled) -- which would otherwise
//! only surface as an opaque runtime error the first time the rule actually
//! ticks, instead of a specific, actionable message fed straight back into
//! the model's one retry attempt.
//!
//! This is a lightweight text scan against `world_api_gen`'s generated
//! registry (see `world_api/schema.yaml` / `tools/gen_world_api.py`), not a
//! real Lua parser -- it can't catch everything (e.g. a call built
//! dynamically via string concatenation), but it catches the overwhelmingly
//! common case, a literal `api.foo(...)` in the source, cheaply.
//!
//! This module also owns the `-- api_version: X.Y.Z` tagging convention
//! (see `world_api/schema.yaml`'s `persistence.api_version_tagging`).

use crate::world_api_gen::{BLOCK_KINDS, METHOD_NAMES, PROPERTY_NAMES, VERSION};

/// One problem found in a rule's source before it was ever executed.
#[derive(Debug, PartialEq, Eq)]
pub struct ValidationIssue {
    pub message: String,
}

/// Scans `source` for `api.<name>` references and flags any `<name>` that
/// isn't a known World API method or property. One issue per distinct
/// unknown name -- a typo used ten times doesn't produce ten near-identical
/// messages.
pub fn validate_source(source: &str) -> Vec<ValidationIssue> {
    let mut seen = std::collections::BTreeSet::new();
    let mut issues = Vec::new();
    for name in extract_api_references(source) {
        if METHOD_NAMES.contains(&name.as_str()) || PROPERTY_NAMES.contains(&name.as_str()) {
            continue;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        let message = match closest_match(&name, METHOD_NAMES.iter().chain(PROPERTY_NAMES.iter())) {
            Some(s) => format!("unknown World API member `api.{name}` (did you mean `api.{s}`?)"),
            None => format!(
                "unknown World API member `api.{name}` -- see world_api/world_api_compact.md for the full member list"
            ),
        };
        issues.push(ValidationIssue { message });
    }
    issues.extend(validate_replace_block_kinds(source));
    issues.extend(validate_shape_literals(source));
    issues
}

// Shape material and filter are separate positional arguments. Never mistake
// the optional filter for the new block kind (e.g. "ore" is not placeable).
fn validate_shape_literals(source: &str) -> Vec<ValidationIssue> {
    let mut issues=Vec::new();
    for (method,kind_index) in [("fill_box",6),("fill_sphere",4)] {
        for (idx,_) in source.match_indices(&format!("api.{method}")) {
            let Some(args)=call_args_span(source,idx+4+method.len()) else {continue;};
            for (index,is_filter) in [(kind_index,false),(kind_index+1,true)] {
                let Some(literal)=argument_literal(args,index) else {continue;};
                let lower=literal.to_ascii_lowercase();
                let valid=BLOCK_KINDS.contains(&lower.as_str()) || is_filter &&
                    ["any","wood","ore","leaves","plant","solid","liquid"].contains(&lower.as_str());
                if !valid {
                    let role=if is_filter {"filter"} else {"block kind"};
                    issues.push(ValidationIssue{message:format!("api.{method}: unknown {role} \"{literal}\"; use the documented material IDs and block_matches categories")});
                }
            }
        }
    }
    issues
}

fn argument_literal(args: &str, wanted: usize) -> Option<&str> {
    let (mut start,mut index,mut depth)=(0,0,0i32);
    let mut quote=None;let mut escaped=false;
    for (offset,ch) in args.char_indices().chain(std::iter::once((args.len(),','))) {
        if let Some(q)=quote {
            if escaped {escaped=false;} else if ch=='\\' {escaped=true;} else if ch==q {quote=None;}
            continue;
        }
        match ch {
            '\''|'"' => quote=Some(ch),
            '('|'{'|'[' => depth+=1,
            ')'|'}'|']' => depth-=1,
            ',' if depth==0 => {
                if index==wanted {
                    let value=args[start..offset].trim();
                    let q=value.chars().next()?;
                    if !matches!(q,'\''|'"') || value.len()<2 || !value.ends_with(q) {return None;}
                    let inner=&value[1..value.len()-1];
                    // Only simple complete literals, not concatenation or escapes.
                    return (!inner.contains(q) && !inner.contains('\\')).then_some(inner);
                }
                index+=1;start=offset+1;
            }
            _=>{}
        }
    }
    None
}

/// Flags a literal (not a variable/expression) `kind` string argument to
/// `api.replace_block` that isn't a real block id -- the single most likely
/// hallucination given the block roster's own naming (e.g. a model trained
/// on generic "dirt"/"wood" vocabulary might not know this project renamed
/// them to "soil"/"oak_wood"). Best-effort: a non-literal kind (a variable,
/// concatenation, ...) is silently skipped rather than flagged, since this
/// is a text scan, not a real Lua evaluator.
fn validate_replace_block_kinds(source: &str) -> Vec<ValidationIssue> {
    let mut seen = std::collections::BTreeSet::new();
    let mut issues = Vec::new();
    for (idx, _) in source.match_indices("replace_block") {
        let Some(args) = call_args_span(source, idx + "replace_block".len()) else {
            continue;
        };
        let Some(kind) = last_string_literal(args) else {
            continue;
        };
        if BLOCK_KINDS.contains(&kind) || !seen.insert(kind.to_string()) {
            continue;
        }
        let message = match closest_match(kind, BLOCK_KINDS.iter()) {
            Some(s) => format!(
                "api.replace_block(...) is called with kind \"{kind}\", which isn't a real block id (did you mean \"{s}\"?)"
            ),
            None => format!(
                "api.replace_block(...) is called with kind \"{kind}\", which isn't a real block id -- see world_api/world_api_compact.md for the block kind list"
            ),
        };
        issues.push(ValidationIssue { message });
    }
    issues
}

/// The substring strictly between the first `(` at/after `start` and its
/// matching `)`, tracking paren nesting so an argument expression with its
/// own parens (e.g. `x + (y - 1)`) doesn't truncate the scan early. `None`
/// if there's no balanced parenthesized call there at all.
fn call_args_span(source: &str, start: usize) -> Option<&str> {
    let open = source[start..].find('(')? + start;
    let mut depth = 0i32;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open + 1..open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The last `"..."` double-quoted literal in `args` (positionally, a call's
/// `kind` argument is always last), or `None` if there isn't one -- e.g.
/// the kind was passed as a variable rather than a literal.
fn last_string_literal(args: &str) -> Option<&str> {
    let close = args.rfind('"')?;
    let open = args[..close].rfind('"')?;
    Some(&args[open + 1..close])
}

/// Every `api.<identifier>` reference in `source`, in source order,
/// duplicates included (`validate_source` dedupes). Skips a match where
/// "api" is a suffix of a longer identifier (e.g. "myapi.foo"), so only a
/// standalone `api` reference counts.
fn extract_api_references(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, _) in source.match_indices("api.") {
        if idx > 0 {
            let prev_is_ident = source[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if prev_is_ident {
                continue;
            }
        }
        let rest = &source[idx + 4..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            out.push(rest[..end].to_string());
        }
    }
    out
}

/// A cheap "did you mean" suggestion: the closest of `candidates` by edit
/// distance, if it's close enough to plausibly be a typo of a real name
/// rather than a genuinely different (if unsupported) idea.
fn closest_match<'a>(name: &str, candidates: impl Iterator<Item = &'a &'static str>) -> Option<&'static str> {
    const MAX_SUGGESTION_DISTANCE: usize = 3;
    candidates
        .map(|&candidate| (candidate, edit_distance(name, candidate)))
        .filter(|&(_, dist)| dist <= MAX_SUGGESTION_DISTANCE)
        .min_by_key(|&(_, dist)| dist)
        .map(|(candidate, _)| candidate)
}

/// Classic Levenshtein edit distance (insert/delete/substitute), O(n*m).
/// Rule sources and member names are short, so the naive DP table is fine.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Leading-comment prefix marking which World API version a rule's source
/// was written/generated against. Purely informational -- never enforced
/// against the running engine's version, so older untagged rules keep
/// loading exactly as before.
pub const API_VERSION_COMMENT_PREFIX: &str = "-- api_version:";

/// Prepends the current World API version tag to `source`, unless it
/// already starts with one (so re-tagging an already-tagged module is a
/// no-op instead of stacking duplicate comment lines).
pub fn tag_with_api_version(source: &str) -> String {
    if source.trim_start().starts_with(API_VERSION_COMMENT_PREFIX) {
        return source.to_string();
    }
    format!("{API_VERSION_COMMENT_PREFIX} {VERSION}\n{source}")
}

/// Reads the `-- api_version: X.Y.Z` tag from the start of a module's
/// source, if present. `None` for untagged source (e.g. an ad-hoc test
/// fixture, or a rule saved before this tagging convention existed) rather
/// than an error.
pub fn extract_api_version(source: &str) -> Option<&str> {
    let first_line = source.lines().next()?;
    first_line
        .trim_start()
        .strip_prefix(API_VERSION_COMMENT_PREFIX)
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shape_validation_distinguishes_materials_filters_and_expressions() {
        for source in [
            "api.fill_box(0,1,0,2,1,2,'gold_ore','ore')",
            "api.fill_sphere(math.floor(p.x),p.y,p.z,2,choose('x','y'),'solid')",
            "api.fill_box(0,1,0,2,1,2,prefix .. 'ore','any')",
        ] { assert!(super::validate_source(source).is_empty(),"{source}"); }
        assert_eq!(super::validate_source("api.fill_box(0,1,0,2,1,2,'ore')").len(),1);
        assert_eq!(super::validate_source("api.fill_sphere(0,1,0,2,'stone','rocks')").len(),1);
    }
    use super::*;

    #[test]
    fn validate_source_accepts_every_real_api_method_and_property() {
        let source = r#"
            function on_tick(api)
                local p = api.nearest_player(0, 0, 0)
                local t = api.time_of_day
                local w = api.weather
                api.replace_block(0, 0, 0, "stone")
                api.broadcast("hi")
            end
        "#;
        assert_eq!(validate_source(source), Vec::new());
    }

    #[test]
    fn validate_source_flags_a_hallucinated_method_with_a_suggestion() {
        // One character off from the real `replace_block` -- close enough
        // that closest_match should surface it as a suggestion.
        let source = r#"function on_tick(api) api.replace_blocks(0, 0, 0, "stone") end"#;
        let issues = validate_source(source);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].message.contains("api.replace_blocks"));
        assert!(
            issues[0].message.contains("did you mean `api.replace_block`"),
            "{}",
            issues[0].message
        );
    }

    #[test]
    fn validate_source_flags_an_unrelated_name_without_a_forced_suggestion() {
        let source = "function on_tick(api) api.cast_fireball(1, 2, 3) end";
        let issues = validate_source(source);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].message.contains("api.cast_fireball"));
    }

    #[test]
    fn validate_source_deduplicates_repeated_unknown_calls() {
        let source = "function on_tick(api) api.foo() api.foo() api.foo() end";
        assert_eq!(validate_source(source).len(), 1);
    }

    #[test]
    fn validate_source_ignores_a_dotted_call_on_something_that_only_ends_in_api() {
        // "myapi.foo" should not be mistaken for a reference to the World
        // API's `api` table.
        let source = "function on_tick(api) local myapi = {} myapi.foo() end";
        assert_eq!(validate_source(source), Vec::new());
    }

    #[test]
    fn validate_source_flags_a_stale_pre_rename_block_kind_literal() {
        // "dirt"/"wood" were the block ids before the block-system rewrite
        // (see block.rs) renamed them to "soil"/"oak_wood" -- exactly the
        // kind of hallucination a model trained on generic Minecraft-ish
        // vocabulary might produce.
        let source = r#"function on_tick(api) api.replace_block(0, 0, 0, "dirt") end"#;
        let issues = validate_source(source);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].message.contains("\"dirt\""));
        assert!(issues[0].message.contains("isn't a real block id"));
    }

    #[test]
    fn validate_source_accepts_a_real_block_kind_literal() {
        let source = r#"function on_tick(api) api.replace_block(0, 0, 0, "oak_wood") end"#;
        assert_eq!(validate_source(source), Vec::new());
    }

    #[test]
    fn validate_source_does_not_flag_a_non_literal_kind_argument() {
        // The kind comes from a variable, not a literal -- this is a text
        // scan, not a real evaluator, so it must not guess/false-positive.
        let source = r#"
            function on_tick(api)
                local kind = pick_a_kind()
                api.replace_block(0, 0, 0, kind)
            end
        "#;
        assert_eq!(validate_source(source), Vec::new());
    }

    #[test]
    fn tag_with_api_version_prepends_the_current_version_once() {
        let tagged = tag_with_api_version("function on_tick(api) end");
        assert!(tagged.starts_with("-- api_version: "));
        assert_eq!(extract_api_version(&tagged), Some(VERSION));

        let tagged_again = tag_with_api_version(&tagged);
        assert_eq!(tagged, tagged_again, "re-tagging should be a no-op");
    }

    #[test]
    fn extract_api_version_is_none_for_untagged_source() {
        assert_eq!(extract_api_version("function on_tick(api) end"), None);
    }
}
