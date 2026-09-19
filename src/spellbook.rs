//! Persistent, host-owned remembered spells. Separate from legacy module records.
use crate::scripting::Module;
use serde::{Deserialize, Serialize};

pub type SpellId = u64;
pub const MAX_SPELLS: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellType {
    #[default]
    Instant,
    Enchantment,
}
impl SpellType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Instant => "Instant",
            Self::Enchantment => "Enchantment",
        }
    }
}

pub const ENCHANTMENT_TAG: &str = "-- spell_type: enchantment";
pub fn is_enchantment_source(source: &str) -> bool {
    source.lines().any(|line| line.trim() == ENCHANTMENT_TAG)
}
pub fn target_requirement(source: &str) -> TargetRequirement {
    match source
        .lines()
        .find_map(|l| l.trim().strip_prefix("-- spell_target: "))
    {
        Some("creature") => TargetRequirement::Creature,
        Some("block") => TargetRequirement::Block,
        Some("device") => TargetRequirement::Device,
        _ => TargetRequirement::Optional,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRequirement {
    #[default]
    Optional,
    Creature,
    Block,
    Device,
}
impl TargetRequirement {
    pub fn label(self) -> &'static str {
        match self {
            Self::Optional => "Spell-defined",
            Self::Creature => "Creature",
            Self::Block => "Block",
            Self::Device => "Device",
        }
    }
    pub fn accepts(self, target: Option<crate::spell_target::Target>) -> bool {
        use crate::spell_target::Target;
        matches!(
            (self, target),
            (Self::Optional, _)
                | (Self::Creature, Some(Target::Creature { .. }))
                | (Self::Block, Some(Target::Block { .. }))
                | (Self::Device, Some(Target::Block { .. }))
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Validation {
    Ready { checked_api: String },
    Review { reason: String },
}
impl Default for Validation {
    fn default() -> Self {
        Self::Review {
            reason: "Not checked in this session".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spell {
    #[serde(default)]
    pub spell_type: SpellType,
    #[serde(default)]
    pub allow_guests: bool,
    pub id: SpellId,
    pub revision: u32,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub flavor_quote: String,
    pub author: String,
    pub original_prompt: String,
    pub source: String,
    pub api_version: String,
    pub interpretation: Option<serde_json::Value>,
    pub target: TargetRequirement,
    pub mana_cost: u32,
    pub cooldown_seconds: f32,
    pub range: f32,
    pub icon: u8,
    /// Lossless procedural image source; older worlds migrate on revalidation.
    #[serde(default)]
    pub artwork: Option<crate::spell_art::Recipe>,
    #[serde(default)]
    pub validation: Validation,
}
impl Spell {
    pub fn can_run_directly(&self) -> bool {
        self.spell_type == SpellType::Instant
    }
    pub fn accepts_target(
        &self,
        world: &crate::voxel::World,
        target: Option<crate::spell_target::Target>,
    ) -> bool {
        if self.spell_type == SpellType::Enchantment && target.is_none() {
            return false;
        }
        if !self.target.accepts(target) {
            return false;
        }
        if let Some(crate::spell_target::Target::Block { position, .. }) = target {
            let device = world.automation.device_at(position).is_some();
            if self.target == TargetRequirement::Device {
                return device;
            }
            if self.spell_type == SpellType::Enchantment && self.target == TargetRequirement::Block
            {
                return !device;
            }
        }
        true
    }
    pub fn target_label(&self) -> &'static str {
        if self.spell_type == SpellType::Enchantment && self.target == TargetRequirement::Optional {
            "Creature, block or device"
        } else {
            self.target.label()
        }
    }
    pub fn artwork(&self) -> crate::spell_art::Recipe {
        self.artwork.filter(|r| r.supported()).unwrap_or_else(|| {
            crate::spell_art::Recipe::from_source(&self.source, &self.description)
        })
    }
    pub fn ready(&self) -> bool {
        matches!(self.validation, Validation::Ready { .. })
    }
    pub fn compiled(&self) -> Result<Module, String> {
        validate_definition(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Spellbook {
    pub format_version: u32,
    pub next_id: SpellId,
    pub spells: Vec<Spell>,
}
impl Default for Spellbook {
    fn default() -> Self {
        Self {
            format_version: 1,
            next_id: 1,
            spells: Vec::new(),
        }
    }
}
impl Spellbook {
    /// Refresh derived ownership and remove bindings to deleted definitions.
    pub fn sync_hotbar(&self, account: &mut crate::crafting::Account) {
        let ready: Vec<_> = self
            .spells
            .iter()
            .filter(|s| s.ready())
            .map(|s| s.id)
            .collect();
        let changed = ready != account.known_spells;
        account.known_spells = ready;
        if changed {
            account.revision = account.revision.saturating_add(1);
        }
        account.prune_hotbar();
    }
    pub fn get(&self, id: SpellId) -> Option<&Spell> {
        self.spells.iter().find(|s| s.id == id)
    }
    /// The same source/prompt identity used by Remember survives renames and saves.
    pub fn delete_for_module(&mut self, module: &Module) {
        self.spells
            .retain(|s| s.source != module.source || s.original_prompt != module.prompt);
    }

    pub fn remember(&mut self, module: &Module, author: &str) -> Result<SpellId, String> {
        let enchantment = is_enchantment_source(&module.source);
        if module.attachment.is_some() || (!module.is_instant && !enchantment) {
            return Err("Remember an Instant or Enchantment template".into());
        }
        if let Some(existing) = self
            .spells
            .iter()
            .find(|s| s.source == module.source && s.original_prompt == module.prompt)
        {
            return Ok(existing.id);
        }
        let interpretation = module
            .source
            .lines()
            .find_map(|line| line.strip_prefix("-- Intent plan: "))
            .and_then(|line| serde_json::from_str(line).ok());
        let description = module
            .source
            .lines()
            .find_map(|line| line.strip_prefix("-- Interpretation: "))
            .unwrap_or(&module.prompt)
            .chars()
            .take(512)
            .collect();
        // Explicit metadata only; do not guess target requirements from prompt words.
        let target = target_requirement(&module.source);
        let spell = Spell {
            spell_type: if enchantment {
                SpellType::Enchantment
            } else {
                SpellType::Instant
            },
            flavor_quote: crate::spell_flavor::from_source(&module.source),
            artwork: Some(module.artwork),
            allow_guests: false,
            id: 0,
            revision: 1,
            name: module.name.chars().take(80).collect(),
            description,
            author: crate::rule_sharing::caster_account(&module.source)
                .unwrap_or_else(|| author.into()),
            original_prompt: module.prompt.clone(),
            source: module.source.clone(),
            api_version: module
                .api_version()
                .unwrap_or(crate::world_api_gen::VERSION)
                .into(),
            interpretation,
            target,
            mana_cost: if enchantment {
                crate::crafting::RULE_MANA
            } else {
                crate::crafting::INSTANT_MANA
            },
            cooldown_seconds: 1.5,
            range: crate::spell_target::CAST_RANGE,
            icon: match target {
                TargetRequirement::Optional => 0,
                TargetRequirement::Creature => 1,
                TargetRequirement::Block => 2,
                TargetRequirement::Device => 3,
            },
            validation: Validation::default(),
        };
        self.insert(spell)
    }

    fn insert(&mut self, mut spell: Spell) -> Result<SpellId, String> {
        if self.format_version != 1 {
            return Err("This Spellbook format requires a newer game".into());
        }
        if self.spells.len() >= MAX_SPELLS {
            return Err("Spellbook is full (64 spells)".into());
        }
        let largest = self.spells.iter().map(|s| s.id).max().unwrap_or(0);
        let id = self
            .next_id
            .max(largest.checked_add(1).ok_or("Spell IDs exhausted")?)
            .max(1);
        let next = id.checked_add(1).ok_or("Spell IDs exhausted")?;
        spell.id = id;
        spell.validation = check(&spell);
        self.spells.push(spell);
        self.next_id = next;
        Ok(id)
    }

    pub fn duplicate(&mut self, id: SpellId) -> Result<SpellId, String> {
        let mut spell = self.get(id).ok_or("Spell no longer exists")?.clone();
        spell.name = format!("{} copy", spell.name.chars().take(75).collect::<String>());
        spell.revision = 1;
        spell.allow_guests = false;
        self.insert(spell)
    }

    pub fn set_flavor_quote(
        &mut self,
        id: SpellId,
        revision: u32,
        quote: &str,
    ) -> Result<(), String> {
        if self.format_version != 1 {
            return Err("Unsupported Spellbook format".into());
        }
        let spell = self
            .spells
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("Spell no longer exists")?;
        if spell.revision != revision {
            return Err("Spell changed while its quote was being written; try again.".into());
        }
        let quote = crate::spell_flavor::clean(quote);
        if quote.is_empty() {
            return Err("The local model returned an empty quote; try again.".into());
        }
        let revision = spell
            .revision
            .checked_add(1)
            .ok_or("Spell revision exhausted")?;
        spell.flavor_quote = quote;
        spell.revision = revision;
        Ok(())
    }

    pub fn update(
        &mut self,
        id: SpellId,
        name: String,
        target: TargetRequirement,
    ) -> Result<(), String> {
        if self.format_version != 1 {
            return Err("Unsupported Spellbook format".into());
        }
        let name = name.trim();
        if name.is_empty() || name.len() > 320 || name.chars().count() > 80 {
            return Err("Use a name of 1 to 80 characters".into());
        }
        let spell = self
            .spells
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("Spell no longer exists")?;
        if spell.name == name && spell.target == target {
            return Ok(());
        }
        spell.revision = spell
            .revision
            .checked_add(1)
            .ok_or("Spell revision exhausted")?;
        spell.name = name.into();
        spell.target = target;
        self.revalidate();
        Ok(())
    }

    pub fn delete(&mut self, id: SpellId) -> Result<(), String> {
        if self.format_version != 1 {
            return Err("Unsupported Spellbook format".into());
        }
        let index = self
            .spells
            .iter()
            .position(|s| s.id == id)
            .ok_or("Spell no longer exists")?;
        self.spells.remove(index);
        Ok(())
    }

    pub fn revalidate(&mut self) {
        for spell in &mut self.spells {
            spell.flavor_quote = if spell.flavor_quote.is_empty() {
                crate::spell_flavor::from_source(&spell.source)
            } else {
                crate::spell_flavor::clean(&spell.flavor_quote)
            };
        }
        for spell in &mut self.spells {
            if spell.artwork.is_none() {
                spell.artwork = Some(spell.artwork());
            }
        }
        let mut counts = std::collections::HashMap::new();
        for spell in &self.spells {
            *counts.entry(spell.id).or_insert(0usize) += 1;
        }
        for (index, spell) in self.spells.iter_mut().enumerate() {
            spell.validation = if self.format_version != 1 {
                Validation::Review {
                    reason: "Unsupported Spellbook format".into(),
                }
            } else if counts[&spell.id] > 1 || spell.id == 0 {
                Validation::Review {
                    reason: "Invalid or duplicate spell ID; original source retained".into(),
                }
            } else if index >= MAX_SPELLS {
                Validation::Review {
                    reason: "Spellbook capacity exceeded; original source retained".into(),
                }
            } else {
                check(spell)
            };
        }
    }
}

fn check(spell: &Spell) -> Validation {
    match validate_definition(spell) {
        Ok(_) => Validation::Ready {
            checked_api: crate::world_api_gen::VERSION.into(),
        },
        Err(reason) => Validation::Review { reason },
    }
}
fn version(value: &str) -> Option<(u32, u32, u32)> {
    let parts = value
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()
        .ok()?;
    (parts.len() == 3).then(|| (parts[0], parts[1], parts[2]))
}
fn validate_definition(spell: &Spell) -> Result<Module, String> {
    let requested = version(&spell.api_version).ok_or("Unrecognized saved API version")?;
    let current = version(crate::world_api_gen::VERSION).unwrap();
    if requested.0 != current.0 || requested > current {
        return Err("Spell needs an unsupported API version".into());
    }
    if let Some(tag) = crate::world_api_validate::extract_api_version(&spell.source) {
        let tagged = version(tag).ok_or("Unrecognized source API version")?;
        if tagged.0 != current.0 || tagged > current {
            return Err("Source needs an unsupported API version".into());
        }
    }
    if spell.revision == 0
        || spell.name.trim().is_empty()
        || spell.name.len() > 320
        || spell.source.len() > crate::rule_sharing::MAX_SOURCE_BYTES
        || spell.original_prompt.len() > crate::rule_sharing::MAX_PROMPT_BYTES
    {
        return Err("Invalid spell metadata or source size".into());
    }
    let expected_cost = if spell.spell_type == SpellType::Enchantment {
        crate::crafting::RULE_MANA
    } else {
        crate::crafting::INSTANT_MANA
    };
    if spell.mana_cost != expected_cost
        || spell.range != crate::spell_target::CAST_RANGE
        || spell.cooldown_seconds != 1.5
    {
        return Err("Unsupported saved cost, range or cooldown; review required".into());
    }
    let issues = crate::world_api_validate::validate_source(&spell.source);
    if !issues.is_empty() {
        return Err(issues
            .into_iter()
            .map(|i| i.message)
            .collect::<Vec<_>>()
            .join("; "));
    }
    let module = Module::load(
        spell.name.clone(),
        spell.original_prompt.clone(),
        spell.source.clone(),
    )?;
    if spell.spell_type == SpellType::Enchantment {
        if module.is_instant
            || !is_enchantment_source(&spell.source)
            || !spell.source.contains("get_rule_target")
        {
            return Err(
                "Enchantment templates require a persistent rule using api.get_rule_target()"
                    .into(),
            );
        }
    } else if !module.is_instant || is_enchantment_source(&spell.source) {
        return Err("Instant spells must define on_cast, not an enchantment rule".into());
    }
    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flavor_edits_preserve_gameplay_duplicate_and_reject_stale_jobs() {
        let mut book = Spellbook::default();
        let id = book.remember(&module(), "Host").unwrap();
        let source = book.get(id).unwrap().source.clone();
        book.set_flavor_quote(id, 1, "The smallest light remembers how to mend the dark.")
            .unwrap();
        assert!(book.set_flavor_quote(id, 1, "Stale result").is_err());
        let copy = book.duplicate(id).unwrap();
        assert_eq!(
            book.get(copy).unwrap().flavor_quote,
            book.get(id).unwrap().flavor_quote
        );
        assert_eq!(book.get(id).unwrap().source, source);
        assert!(book.get(id).unwrap().ready());
        let mut restored: Spellbook =
            serde_json::from_str(&serde_json::to_string(&book).unwrap()).unwrap();
        restored.revalidate();
        assert_eq!(
            restored.get(id).unwrap().flavor_quote,
            book.get(id).unwrap().flavor_quote
        );
        book.delete(id).unwrap();
        assert!(book.set_flavor_quote(id, 2, "Late result").is_err());
    }
    fn module() -> Module {
        Module::load(
            "Heal".into(),
            "Heal the creature I am aiming at".into(),
            include_str!("../modules/target_heal.lua").into(),
        )
        .unwrap()
    }
    #[test]
    fn hotbar_bindings_survive_reload_and_rename_but_not_deletion() {
        use crate::equipment::Entry;
        let mut book = Spellbook::default();
        let id = book.remember(&module(), "Host").unwrap();
        let mut account = crate::crafting::Account::default();
        book.sync_hotbar(&mut account);
        account.hotbar.select(8);
        account.hotbar.assign(Some(Entry::Spell(id)));
        let saved = serde_json::to_vec(&(book, account)).unwrap();
        let (mut book, mut account): (Spellbook, crate::crafting::Account) =
            serde_json::from_slice(&saved).unwrap();
        book.revalidate();
        book.sync_hotbar(&mut account);
        assert_eq!(account.hotbar.entry(), Some(Entry::Spell(id)));
        assert_eq!(Entry::Spell(id).count(&account), 1);
        book.update(id, "New name".into(), TargetRequirement::Creature)
            .unwrap();
        book.sync_hotbar(&mut account);
        assert_eq!(
            crate::equipment_ui::entry_name(account.hotbar.entry().unwrap(), &book),
            "New name"
        );
        book.spells[0].validation = Validation::Review {
            reason: "Invalid API".into(),
        };
        book.sync_hotbar(&mut account);
        assert_eq!(Entry::Spell(id).count(&account), 0);
        assert_eq!(account.hotbar.entry(), None);
        let revision = account.hotbar.revision;
        book.delete(id).unwrap();
        book.sync_hotbar(&mut account);
        assert_eq!(account.hotbar.entry(), None);
        assert_eq!(account.hotbar.revision, revision);
    }
    #[test]
    fn spell_assignment_requires_authoritative_ownership() {
        use crate::equipment::{accept_hotbar, Entry};
        let mut book = Spellbook::default();
        let id = book.remember(&module(), "Host").unwrap();
        let mut host = crate::crafting::Account::default();
        book.sync_hotbar(&mut host);
        let mut proposed = host.hotbar.clone();
        proposed.assign(Some(Entry::Spell(id)));
        let mut guest = crate::crafting::Account::default();
        assert!(accept_hotbar(&mut guest, &proposed).is_err());
        assert!(accept_hotbar(&mut host, &proposed).is_ok());
        assert!(!crate::equipment::can_mine(
            host.hotbar.entry(),
            crate::voxel::BlockType::Stone,
            &host
        ));
        assert_eq!(Entry::Spell(id).resource(), None);
    }
    #[test]
    fn spellbook_ids_survive_rename_duplicate_delete_and_reload() {
        let module = module();
        let source = module.source.clone();
        let mut book = Spellbook::default();
        let first = book.remember(&module, "Mira").unwrap();
        assert_eq!(book.remember(&module, "Mira").unwrap(), first);
        book.update(first, "Healing Touch".into(), TargetRequirement::Creature)
            .unwrap();
        assert_eq!(book.get(first).unwrap().revision, 2);
        let second = book.duplicate(first).unwrap();
        assert_ne!(first, second);
        assert_eq!(book.get(second).unwrap().revision, 1);
        book.delete(first).unwrap();
        let bytes = serde_json::to_vec(&book).unwrap();
        let mut book: Spellbook = serde_json::from_slice(&bytes).unwrap();
        book.revalidate();
        let spell = book.get(second).unwrap();
        assert!(spell.ready());
        assert_eq!(spell.source, source);
        assert_eq!(spell.author, "Mira");
        assert_eq!(spell.target, TargetRequirement::Creature);
        book.delete(second).unwrap();
        let third = book.remember(&module, "Mira").unwrap();
        assert!(third > second);
        assert!(book.get(first).is_none());
        assert!(book.get(second).is_none());
    }
    #[test]
    fn spellbook_revalidates_saved_status_and_preserves_incompatible_source() {
        let mut book = Spellbook::default();
        let id = book.remember(&module(), "Host").unwrap();
        book.spells[0].source = "function on_cast(api,e) api.not_a_real_method() end".into();
        let original = book.spells[0].source.clone();
        let prompt = book.spells[0].original_prompt.clone();
        let mut book: Spellbook =
            serde_json::from_slice(&serde_json::to_vec(&book).unwrap()).unwrap();
        book.revalidate();
        assert!(!book.get(id).unwrap().ready());
        assert!(book.get(id).unwrap().compiled().is_err());
        assert_eq!(book.get(id).unwrap().source, original);
        assert_eq!(book.get(id).unwrap().original_prompt, prompt);
        book.spells[0].source = "function on_cast(api,e) end".into();
        book.spells[0].api_version = "999.0.0".into();
        book.revalidate();
        assert!(!book.spells[0].ready());
        book.spells[0].api_version = "1.0.0".into();
        book.revalidate();
        assert!(book.spells[0].ready());
        assert_eq!(book.spells[0].api_version, "1.0.0");
        book.spells[0].source = "this is invalid Lua".into();
        book.revalidate();
        assert!(!book.spells[0].ready());
    }
    #[test]
    fn spellbook_duplicate_saved_ids_stay_blocked_after_rename_and_ids_never_wrap() {
        let mut book = Spellbook::default();
        let id = book.remember(&module(), "Host").unwrap();
        book.spells.push(book.spells[0].clone());
        book.revalidate();
        assert!(book.spells.iter().all(|s| !s.ready()));
        book.update(id, "Renamed".into(), TargetRequirement::Creature)
            .unwrap();
        assert!(book.spells.iter().all(|s| !s.ready()));
        book.next_id = u64::MAX;
        assert!(book.duplicate(id).is_err());
        assert_eq!(book.spells.len(), 2);
    }
    #[test]
    fn spellbook_preserves_interpretation_guest_authorship_and_required_target() {
        let source=crate::rule_sharing::annotate(
            "-- Interpretation: Restore target health\n-- Intent plan: {\"effect\":\"custom\"}\n-- spell_target: creature\nfunction on_cast(api,e) api.heal_creature(e.target.id,20) end",
            "Guest","steam:123");
        let module =
            Module::load("Healing".into(), "heal that cow".into(), source.clone()).unwrap();
        let mut book = Spellbook::default();
        let id = book.remember(&module, "Host").unwrap();
        let spell = book.get(id).unwrap();
        assert_eq!(spell.author, "steam:123");
        assert_eq!(spell.source, source);
        assert_eq!(spell.description, "Restore target health");
        assert_eq!(spell.interpretation.as_ref().unwrap()["effect"], "custom");
        assert!(!spell.target.accepts(None));
        assert!(spell
            .target
            .accepts(Some(crate::spell_target::Target::Creature { id: 55 })));
        assert!(!spell
            .target
            .accepts(Some(crate::spell_target::Target::Block {
                position: (0, 0, 0),
                material: crate::voxel::BlockType::Stone
            })));
        let rule = Module::load(
            "Rule".into(),
            "rain".into(),
            "function on_tick(api) end".into(),
        )
        .unwrap();
        assert!(book.remember(&rule, "Host").is_err());
    }
    #[test]
    fn spellbook_save_load_does_not_run_cast_or_rewrite_code() {
        let module = Module::load(
            "Once".into(),
            "give a message".into(),
            "-- api_version: 1.0.0\nfunction on_cast(api,e) error('only when cast') end".into(),
        )
        .unwrap();
        let mut book = Spellbook::default();
        let id = book.remember(&module, "Host").unwrap();
        let mut book: Spellbook =
            serde_json::from_str(&serde_json::to_string(&book).unwrap()).unwrap();
        book.revalidate();
        assert!(book.get(id).unwrap().ready());
        assert_eq!(book.get(id).unwrap().source, module.source);
        assert_eq!(book.get(id).unwrap().api_version, "1.0.0");
    }
}
