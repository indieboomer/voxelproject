//! Private callback state. Only commit can mutate the authoritative world.
use super::*;
use crate::creature::CreatureDraft;
use std::collections::HashMap;

pub(super) struct CallbackTransaction<'a> {
    pub world: &'a World,
    pub creatures: RefCell<CreatureDraft>,
    pub weather: RefCell<WeatherState>,
    pub time: RefCell<f32>,
    pub blocks: RefCell<Vec<(i32, i32, i32, BlockType)>>,
    pub deaths: RefCell<Vec<DeathEvent>>,
    pub broadcasts: RefCell<Vec<String>>,
    pub effects: RefCell<Vec<PlayerEffect>>,
    pub spawn_seed: Cell<u64>,
    pub attack_policies: RefCell<Vec<crate::creature::AttackPolicy>>,
    pub policy_owner: Option<(u64, u64)>,
    prior_blocks: HashMap<(i32, i32, i32), BlockType>,
    base_players: Vec<PlayerSnapshot>,
    resources: HashMap<PlayerId, [u32; COLLECTIBLE_BLOCKS.len()]>,
}

impl<'a> CallbackTransaction<'a> {
    pub fn new(input: &TickInput<'a>, spawn_seed: u64) -> Self {
        let mut players = input.players.to_vec();
        for effect in input.player_effects.iter() {
            apply_player_preview(&mut players, effect);
        }
        let mut resources = HashMap::new();
        for player in input.players {
            let mut counts = if player.id == HOST_PLAYER_ID { input.host_resources } else { player.resources };
            for (i, block) in COLLECTIBLE_BLOCKS.iter().enumerate() {
                counts[i] = resource_balance(counts[i], player.id, *block, input.player_effects);
            }
            resources.insert(player.id, counts);
        }
        Self {
            world: input.world,
            creatures: RefCell::new(CreatureDraft::new(input.creatures)),
            weather: RefCell::new(input.weather.clone()),
            time: RefCell::new(*input.time_of_day),
            blocks: RefCell::new(Vec::new()),
            deaths: RefCell::new(Vec::new()),
            broadcasts: RefCell::new(Vec::new()),
            effects: RefCell::new(Vec::new()),
            spawn_seed: Cell::new(spawn_seed),
            attack_policies: RefCell::new(Vec::new()),
            policy_owner: None,
            prior_blocks: input
                .block_edits
                .iter()
                .map(|&(x, y, z, block)| ((x, y, z), block))
                .collect(),
            base_players: players,
            resources,
        }
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType {
        self.blocks
            .borrow()
            .iter()
            .rev()
            .find(|&&(bx, by, bz, _)| (bx, by, bz) == (x, y, z))
            .map(|entry| entry.3)
            .or_else(|| self.prior_blocks.get(&(x, y, z)).copied())
            .unwrap_or_else(|| self.world.get_block(x, y, z))
    }

    pub fn native_scan_cost(&self) -> u32 {
        1 + self.creatures.borrow().snapshot.len() as u32
            + self.base_players.len() as u32
            + self.effects.borrow().len() as u32
            + self.blocks.borrow().len() as u32
            + self.attack_policies.borrow().len() as u32
    }

    pub fn players(&self) -> Vec<PlayerSnapshot> {
        let mut players = self.base_players.clone();
        for effect in self.effects.borrow().iter() {
            apply_player_preview(&mut players, effect);
        }
        players
    }

    pub fn resource_count(&self, player_id: PlayerId, index: usize) -> Option<u32> {
        Some(resource_balance(self.resources.get(&player_id)?[index], player_id,
            COLLECTIBLE_BLOCKS[index], &self.effects.borrow()))
    }
    pub fn inventory(&self, player_id: PlayerId) -> Option<crate::crafting::Account> {
        let p = self.players().into_iter().find(|p|p.id==player_id)?;
        let mut account = crate::crafting::Account {mana:p.finances.mana,elements:p.finances.elements,gear:p.finances.items,..Default::default()};
        account.resources=*self.resources.get(&player_id)?;
        for effect in self.effects.borrow().iter() {
            match *effect {
                PlayerEffect::Inventory {player_id:id,resources,..} if id==player_id => account.resources=resources,
                PlayerEffect::GiveItem {player_id:id,block,amount} if id==player_id => {
                    if let Some(i)=COLLECTIBLE_BLOCKS.iter().position(|b|*b==block) {account.resources[i]=account.resources[i].saturating_add(amount);}
                }
                PlayerEffect::TakeItem {player_id:id,block,amount} if id==player_id => {
                    if let Some(i)=COLLECTIBLE_BLOCKS.iter().position(|b|*b==block) {account.resources[i]=account.resources[i].saturating_sub(amount);}
                }
                _=>{}
            }
        }
        Some(account)
    }
    pub fn stage_inventory(&self, player_id: PlayerId, account: &crate::crafting::Account) {
        self.effects.borrow_mut().push(PlayerEffect::Inventory {player_id,balances:InventoryBalances::from_account(account),resources:account.resources});
    }

    pub fn commit(self, input: &mut TickInput, spawn_seed: &Cell<u64>, emit_deaths: bool) {
        if let Some(owner) = self.policy_owner {
            let policies = self.attack_policies.into_inner();
            if policies.is_empty() { input.creatures.attack_policies.remove(&owner); }
            else { input.creatures.attack_policies.insert(owner, policies); }
        }
        self.creatures.into_inner().commit(input.creatures);
        *input.weather = self.weather.into_inner();
        *input.time_of_day = self.time.into_inner();
        input.block_edits.extend(self.blocks.into_inner());
        input.player_effects.extend(self.effects.into_inner());
        if emit_deaths {
            input.death_events.extend(self.deaths.into_inner());
        }
        for message in self.broadcasts.into_inner() {
            log::info!("[rule] {message}");
            input.broadcasts.push(message);
        }
        spawn_seed.set(self.spawn_seed.get());
    }
}

fn resource_balance(mut balance: u32, owner: PlayerId, block: BlockType, effects: &[PlayerEffect]) -> u32 {
    for effect in effects {
        match *effect {
            PlayerEffect::Inventory {player_id, resources, ..} if player_id==owner => {
                if let Some(i)=COLLECTIBLE_BLOCKS.iter().position(|b|*b==block) {balance=resources[i];}
            }
            PlayerEffect::GiveItem {
                player_id,
                block: b,
                amount,
            } if b == block && player_id == owner => {
                balance = balance.saturating_add(amount);
            }
            PlayerEffect::TakeItem {
                player_id,
                block: b,
                amount,
            } if b == block && player_id == owner => {
                balance = balance.saturating_sub(amount);
            }
            _ => {}
        }
    }
    balance
}

fn apply_player_preview(players: &mut [PlayerSnapshot], effect: &PlayerEffect) {
    for player in players {
        match *effect {
            PlayerEffect::Inventory {player_id,balances,..} if player_id==player.id => player.finances=balances,
            PlayerEffect::Health { player_id, delta } if player_id == player.id => {
                player.health = (player.health + delta).clamp(0.0, crate::player::MAX_HEALTH);
            }
            PlayerEffect::Poisoned {
                player_id,
                poisoned,
            } if player_id == player.id => player.poisoned = poisoned,
            PlayerEffect::SpeedMultiplier {
                player_id,
                multiplier,
            } if player_id == player.id => player.speed_multiplier = multiplier,
            PlayerEffect::JumpMultiplier {
                player_id,
                multiplier,
            } if player_id == player.id => player.jump_multiplier = multiplier,
            PlayerEffect::Teleport { player_id, pos } if player_id == player.id => {
                player.pos = pos;
                player.velocity = Vec3::ZERO;
            }
            _ => {}
        }
    }
}
