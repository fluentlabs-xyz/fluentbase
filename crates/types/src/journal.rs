use crate::ExitCode;
use alloc::vec::Vec;
use alloy_primitives::{Address, Bytes, B256};
use auto_impl::auto_impl;

#[derive(Debug, Clone)]
pub enum JournalEvent {
    ItemChanged {
        key: [u8; 32],
        preimage: Vec<[u8; 32]>,
        flags: u32,
        prev_state: Option<usize>,
    },
    ItemRemoved {
        key: [u8; 32],
        prev_state: Option<usize>,
    },
}

impl JournalEvent {
    pub fn key(&self) -> &[u8; 32] {
        match self {
            JournalEvent::ItemChanged { key, .. } => key,
            JournalEvent::ItemRemoved { key, .. } => key,
        }
    }

    pub fn is_removed(&self) -> bool {
        match self {
            JournalEvent::ItemChanged { .. } => false,
            JournalEvent::ItemRemoved { .. } => true,
        }
    }

    pub fn preimage(&self) -> Option<(Vec<[u8; 32]>, u32)> {
        match self {
            JournalEvent::ItemChanged {
                preimage: value,
                flags,
                ..
            } => Some((value.clone(), *flags)),
            JournalEvent::ItemRemoved { .. } => None,
        }
    }

    pub fn prev_state(&self) -> Option<usize> {
        match self {
            JournalEvent::ItemChanged { prev_state, .. } => *prev_state,
            JournalEvent::ItemRemoved { prev_state, .. } => *prev_state,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct JournalCheckpoint(pub u32, pub u32);

impl From<(u32, u32)> for JournalCheckpoint {
    fn from(value: (u32, u32)) -> Self {
        Self(value.0, value.1)
    }
}
impl Into<(u32, u32)> for JournalCheckpoint {
    fn into(self) -> (u32, u32) {
        (self.0, self.1)
    }
}

impl JournalCheckpoint {
    pub fn from_u64(value: u64) -> Self {
        Self((value >> 32) as u32, value as u32)
    }

    pub fn to_u64(&self) -> u64 {
        (self.0 as u64) << 32 | self.1 as u64
    }

    pub fn state(&self) -> usize {
        self.0 as usize
    }

    pub fn logs(&self) -> usize {
        self.1 as usize
    }
}

pub struct JournalLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
}

#[auto_impl(&, Rc, Arc, Box)]
pub trait IJournaledTrie {
    fn checkpoint(&self) -> JournalCheckpoint;
    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, u32, bool)>;
    fn update(&self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32);
    fn remove(&self, key: &[u8; 32]);
    fn compute_root(&self) -> [u8; 32];
    fn emit_log(&self, address: Address, topics: Vec<B256>, data: Bytes);
    fn commit(&self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode>;
    fn rollback(&self, checkpoint: JournalCheckpoint);
    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) -> bool;
    fn preimage(&self, hash: &[u8; 32]) -> Vec<u8>;
    fn preimage_size(&self, hash: &[u8; 32]) -> u32;
    fn journal(&self) -> Vec<JournalEvent>;
}

#[derive(Default, Clone)]
pub struct EmptyJournalTrie;

#[allow(unused_variables)]
impl IJournaledTrie for EmptyJournalTrie {
    fn checkpoint(&self) -> JournalCheckpoint {
        todo!()
    }

    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, u32, bool)> {
        todo!()
    }

    fn update(&self, key: &[u8; 32], value: &Vec<[u8; 32]>, flags: u32) {
        todo!()
    }

    fn remove(&self, key: &[u8; 32]) {
        todo!()
    }

    fn compute_root(&self) -> [u8; 32] {
        todo!()
    }

    fn emit_log(&self, address: Address, topics: Vec<B256>, data: Bytes) {
        todo!()
    }

    fn commit(&self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        todo!()
    }

    fn rollback(&self, checkpoint: JournalCheckpoint) {
        todo!()
    }

    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) -> bool {
        todo!()
    }

    fn preimage(&self, hash: &[u8; 32]) -> Vec<u8> {
        todo!()
    }

    fn preimage_size(&self, hash: &[u8; 32]) -> u32 {
        todo!()
    }

    fn journal(&self) -> Vec<JournalEvent> {
        todo!()
    }
}
