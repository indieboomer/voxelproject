//! Bounded, atomic installation snapshots over the existing reliable channel.
use crate::automation::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub revision: u64,
    pub index: u32,
    pub count: u32,
    pub tick: u64,
    pub devices: Vec<Device>,
}
pub fn chunks(state: &State, revision: u64) -> Vec<Chunk> {
    let devices: Vec<_> = state.devices.values().cloned().collect();
    let count = devices.len().max(1).div_ceil(4) as u32;
    (0..count)
        .map(|index| Chunk {
            revision,
            index,
            count,
            tick: state.tick,
            devices: devices
                .iter()
                .skip(index as usize * 4)
                .take(4)
                .cloned()
                .collect(),
        })
        .collect()
}
#[derive(Default)]
pub struct Transfer {
    complete: u64,
    pending: u64,
    count: u32,
    tick: u64,
    chunks: BTreeMap<u32, Vec<Device>>,
}
impl Transfer {
    pub fn has_snapshot(&self)->bool {self.complete>0}
    pub fn accept(
        &mut self,
        chunk: Chunk,
        recipes: &crate::crafting::Registry,
    ) -> Result<Option<State>, String> {
        if chunk.revision <= self.complete || chunk.revision < self.pending {
            return Ok(None);
        }
        if chunk.count == 0
            || chunk.count > 32
            || chunk.index >= chunk.count
            || chunk.devices.len() > 4
        {
            return Err("Invalid automation transfer".into());
        }
        if chunk.revision > self.pending {
            self.pending = chunk.revision;
            self.count = chunk.count;
            self.tick = chunk.tick;
            self.chunks.clear();
        }
        if self.count != chunk.count || self.tick != chunk.tick {
            return Err("Mixed automation snapshot".into());
        }
        self.chunks.entry(chunk.index).or_insert(chunk.devices);
        if self.chunks.len() != self.count as usize {
            return Ok(None);
        }
        let mut state = State {
            tick: self.tick,
            ..Default::default()
        };
        for devices in self.chunks.values() {
            for d in devices {
                if state.devices.insert(d.cell, d.clone()).is_some() {
                    return Err("Duplicate device".into());
                }
            }
        }
        state.validate(balance(), recipes)?;
        self.complete = self.pending;
        self.chunks.clear();
        Ok(Some(state))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_catalog_chests_fit_packets_and_replicate_large_counts() {
        let mut state=State::default();
        for x in 0..4 {
            let p=(x,30,0);let mut chest=Device::new(Kind::Chest,p,0);
            for block in crate::voxel::COLLECTIBLE_BLOCKS {chest.items.insert(format!("resource:{}",block.id()),2_000_000);}
            state.devices.insert(p,chest);
        }
        let recipes=crate::crafting::Registry::load().unwrap();
        let mut transfer=Transfer::default();let mut received=None;
        for chunk in chunks(&state,1) {
            let bytes=crate::net::encode(&crate::net::Packet::Reliable{id:1,msg:crate::net::ReliableMsg::AutomationState(chunk)});
            assert!(bytes.len()<crate::transport::MAX_PACKET_BYTES);
            let crate::net::Packet::Reliable {msg:crate::net::ReliableMsg::AutomationState(chunk),..}=crate::net::decode(&bytes).unwrap() else {panic!("snapshot")};
            received=transfer.accept(chunk,&recipes).unwrap().or(received);
        }
        assert_eq!(received.unwrap(),state);
    }
    #[test]
    fn clients_commit_complete_snapshots_despite_reordering_and_duplicates() {
        let recipes = crate::crafting::Registry::load().unwrap();
        let mut state = State::default();
        for (x,kind) in Kind::ALL.into_iter().enumerate() {
            let p=(x as i32,30,0);
            let mut d=Device::new(kind,p,0);
            if kind.sustained() {d.powered_ticks=5;d.activity=Activity::Working;d.mana=12;}
            state.devices.insert(p,d);
        }
        let mut receiver = Transfer::default();
        let packets = chunks(&state, 1);
        let mut result = None;
        for packet in packets.iter().rev() {
            let bytes = crate::net::encode(&crate::net::Packet::Reliable {
                id: 1,
                msg: crate::net::ReliableMsg::AutomationState(packet.clone()),
            });
            let crate::net::Packet::Reliable {
                msg: crate::net::ReliableMsg::AutomationState(chunk),
                ..
            } = crate::net::decode(&bytes).unwrap()
            else {
                panic!("device packet")
            };
            if let Some(s) = receiver.accept(chunk.clone(), &recipes).unwrap() {
                result = Some(s);
            }
            assert!(receiver.accept(chunk, &recipes).unwrap().is_none());
        }
        assert_eq!(result.unwrap(), state);
        let empty = State::default();
        assert_eq!(
            receiver
                .accept(chunks(&empty, 2).remove(0), &recipes)
                .unwrap(),
            Some(empty)
        );
        assert!(receiver
            .accept(packets[0].clone(), &recipes)
            .unwrap()
            .is_none());
    }
}
