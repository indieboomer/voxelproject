//! Bounded display metadata and connection-scoped cast intentions. Lua stays on the host.
use crate::spell_target::{Target, TargetContext};
use crate::spellbook::{Spell, SpellId, Spellbook, TargetRequirement, Validation};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub id: SpellId,
    pub revision: u32,
    pub name: String,
    pub description: String,
    pub author: String,
    pub target: TargetRequirement,
    pub mana_cost: u32,
    pub cooldown_seconds: f32,
    pub range: f32,
    pub icon: u8,
    pub artwork: crate::spell_art::Recipe,
    pub flavor_quote: String,
}
impl Summary {
    pub fn from_spell(s: &Spell) -> Self {
        Self {
            id: s.id,
            revision: s.revision,
            flavor_quote: crate::spell_flavor::clean(&s.flavor_quote),
            name: s.name.chars().take(80).collect(),
            description: s.description.chars().take(80).collect(),
            author: s.author.chars().take(24).collect(),
            target: s.target,
            mana_cost: s.mana_cost,
            cooldown_seconds: s.cooldown_seconds,
            range: s.range,
            icon: s.icon,
            artwork: s.artwork(),
        }
    }
    pub fn into_spell(self) -> Option<Spell> {
        if self.flavor_quote.len() > crate::spell_flavor::MAX_BYTES
            || self.flavor_quote.chars().any(char::is_control)
            || !self.artwork.supported()
            || self.id == 0
            || self.revision == 0
            || self.name.len() > 320
            || self.description.len() > 320
            || self.author.len() > 96
            || self.mana_cost != crate::crafting::INSTANT_MANA
            || self.cooldown_seconds != 1.5
            || self.range != crate::spell_target::CAST_RANGE
        {
            return None;
        }
        Some(Spell {
            id: self.id,
            revision: self.revision,
            name: self.name,
            description: self.description,
            flavor_quote: self.flavor_quote,
            target: self.target,
            mana_cost: self.mana_cost,
            cooldown_seconds: self.cooldown_seconds,
            range: self.range,
            icon: self.icon,
            artwork: Some(self.artwork),
            allow_guests: true,
            author: self.author,
            source: String::new(),
            original_prompt: String::new(),
            api_version: crate::world_api_gen::VERSION.into(),
            interpretation: None,
            validation: Validation::Ready {
                checked_api: crate::world_api_gen::VERSION.into(),
            },
        })
    }
}
pub fn guest_book(book: &Spellbook) -> Spellbook {
    Spellbook {
        spells: book
            .spells
            .iter()
            .filter(|s| s.ready() && s.allow_guests)
            .cloned()
            .collect(),
        ..Default::default()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub session: u64,
    pub sequence: u64,
    pub spell: SpellId,
    pub revision: u32,
    pub facing: [f32; 3],
    pub target: Option<Target>,
}
pub struct Session {
    pub token: u64,
    last_sequence: u64,
    last_attempt: Option<Instant>,
}
impl Session {
    pub fn new(token: u64) -> Self {
        Self {
            token,
            last_sequence: 0,
            last_attempt: None,
        }
    }
    pub fn fresh(&self, r: &Request) -> bool {
        r.session == self.token && r.sequence > self.last_sequence && r.sequence != 0
    }
    pub fn accept(&mut self, r: &Request, now: Instant) -> Result<(), String> {
        if !self.fresh(r) {
            return Err("Expired or repeated cast request".into());
        }
        self.last_sequence = r.sequence;
        if self
            .last_attempt
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(100))
        {
            return Err("Casting too quickly; try again".into());
        }
        self.last_attempt = Some(now);
        Ok(())
    }
}
#[derive(Default)]
pub struct State {
    pub peers: HashMap<crate::transport::Peer, Session>,
    pub guest_cooldowns: HashMap<(String, SpellId), Instant>,
    pub catalog_revision: u64,
    pub session: Option<u64>,
    pub received_revision: u64,
    pub sequence: u64,
    pub pending: Option<(u64, SpellId)>,
    pub ready_at: HashMap<SpellId, Instant>,
}
pub fn resolve_request(
    r: &Request,
    spell: &Spell,
    world: &crate::voxel::World,
    creatures: &crate::creature::Creatures,
    eye: glam::Vec3,
) -> Result<Option<TargetContext>, String> {
    if !spell.ready() || !spell.allow_guests || spell.id != r.spell || spell.revision != r.revision
    {
        return Err("Spell changed or permission was removed".into());
    }
    let facing = glam::Vec3::from_array(r.facing);
    if !facing.is_finite() || (facing.length_squared() - 1.).abs() > 0.02 {
        return Err("Invalid cast direction".into());
    }
    let context = TargetContext::resolve(world, creatures, eye, facing);
    if context.map(|c| c.target) != r.target || !spell.target.accepts(context.map(|c| c.target)) {
        return Err("Target changed, obstructed or out of range. Aim again.".into());
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    fn fixture() -> (
        Spell,
        crate::voxel::World,
        crate::creature::Creatures,
        Vec3,
        Request,
    ) {
        let m = crate::scripting::Module::load(
            "Stone".into(),
            "Stone".into(),
            include_str!("../modules/target_stone.lua").into(),
        )
        .unwrap();
        let mut book = Spellbook::default();
        let id = book.remember(&m, "Host").unwrap();
        let mut spell = book.get(id).unwrap().clone();
        spell.allow_guests = true;
        let mut world = crate::voxel::World::new(1);
        world
            .chunks
            .insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        world.set_block(8, 25, 12, crate::voxel::BlockType::Soil);
        let creatures = crate::creature::Creatures::new();
        let eye = Vec3::new(8.5, 25.5, 8.5);
        let target = TargetContext::resolve(&world, &creatures, eye, Vec3::Z)
            .unwrap()
            .target;
        let request = Request {
            session: 77,
            sequence: 1,
            spell: id,
            revision: spell.revision,
            facing: Vec3::Z.to_array(),
            target: Some(target),
        };
        (spell, world, creatures, eye, request)
    }
    #[test]
    fn permission_revision_target_and_direction_are_authoritative() {
        let (mut spell, mut world, creatures, eye, mut request) = fixture();
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_ok());
        spell.allow_guests = false;
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_err());
        spell.allow_guests = true;
        request.revision += 1;
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_err());
        request.revision -= 1;
        request.facing = [f32::NAN, 0., 1.];
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_err());
        request.facing = Vec3::Z.to_array();
        world.set_block(8, 25, 10, crate::voxel::BlockType::Stone);
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_err());
        world.set_block(8, 25, 10, crate::voxel::BlockType::Air);
        world.set_block(8, 25, 12, crate::voxel::BlockType::Stone);
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_err());
        world.chunks.clear();
        assert!(resolve_request(&request, &spell, &world, &creatures, eye).is_err());
    }
    #[test]
    fn repeated_reordered_old_session_and_rate_limited_requests_cannot_reexecute() {
        let (_, _, _, _, mut request) = fixture();
        let now = Instant::now();
        let mut session = Session::new(77);
        assert!(session.accept(&request, now).is_ok());
        assert!(session
            .accept(&request, now + Duration::from_secs(2))
            .is_err());
        request.sequence = 3;
        assert!(session
            .accept(&request, now + Duration::from_millis(1))
            .is_err());
        assert!(session
            .accept(&request, now + Duration::from_secs(2))
            .is_err());
        request.sequence = 2;
        assert!(!session.fresh(&request));
        request.sequence = 4;
        assert!(session
            .accept(&request, now + Duration::from_secs(2))
            .is_ok());
        let mut reconnect = Session::new(78);
        assert!(reconnect.accept(&request, now).is_err());
        request.session = 78;
        request.sequence = 1;
        assert!(reconnect.accept(&request, now).is_ok());
    }
    #[test]
    fn catalog_is_bounded_private_and_only_grants_shared_ready_spells() {
        let (mut spell, _, _, _, request) = fixture();
        let mut book = Spellbook::default();
        book.spells.push(spell.clone());
        let mut account = crate::crafting::Account::default();
        guest_book(&book).sync_hotbar(&mut account);
        assert_eq!(account.known_spells, vec![spell.id]);
        account
            .hotbar
            .assign(Some(crate::equipment::Entry::Spell(spell.id)));
        book.spells[0].allow_guests = false;
        guest_book(&book).sync_hotbar(&mut account);
        assert!(account.known_spells.is_empty());
        assert_eq!(account.hotbar.entry(), None);
        spell.name = "🪄".repeat(200);
        spell.description = "🪄".repeat(600);
        spell.flavor_quote = "x".repeat(crate::spell_flavor::MAX_BYTES);
        let summary = Summary::from_spell(&spell);
        let bytes = bincode::serialize(&crate::net::ReliableMsg::SpellCatalog {
            session: 77,
            revision: 1,
            spells: vec![summary.clone(); 64],
        })
        .unwrap();
        assert!(bytes.len() + 64 < crate::transport::MAX_PACKET_BYTES);
        let display = summary.into_spell().unwrap();
        assert!(display.source.is_empty() && display.original_prompt.is_empty());
        assert_eq!(display.artwork(), spell.artwork());
        assert_eq!(display.flavor_quote, spell.flavor_quote);
        let mut invalid = Summary::from_spell(&spell);
        invalid.flavor_quote.push('x');
        assert!(invalid.into_spell().is_none());
        assert_eq!(
            crate::spell_art::render(display.artwork()),
            crate::spell_art::render(spell.artwork())
        );
        let mut invalid = Summary::from_spell(&spell);
        invalid.artwork.version = 255;
        assert!(invalid.into_spell().is_none());
        let bytes = bincode::serialize(&request).unwrap();
        let restored: Request = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.target, request.target);
    }
}
