use crate::{Address, Bytes, B256, U256};
use alloc::vec::Vec;
use fluentbase_types::{
    Account,
    AccountCheckpoint,
    AccountStatus,
    ExitCode,
    Fuel,
    IsColdAccess,
    JournalCheckpoint,
    SharedAPI,
    SovereignAPI,
    SovereignJournalAPI,
    F254,
};
use hashbrown::{hash_map::Entry, HashMap};

pub struct JournalStateLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
}

pub enum JournalStateEvent {
    AccountChanged {
        address: Address,
        account_status: AccountStatus,
        account: Account,
        prev_state: Option<usize>,
    },
    StorageChanged {
        address: Address,
        slot: U256,
        had_value: U256,
    },
    PreimageChanged {
        hash: B256,
    },
}

impl JournalStateEvent {
    pub(crate) fn unwrap_account(&self) -> &Account {
        match self {
            JournalStateEvent::AccountChanged { account, .. } => account,
            _ => unreachable!("can't unwrap account"),
        }
    }
}

pub struct JournalStateWrapper {
    storage: HashMap<(Address, U256), U256>,
    accounts: HashMap<Address, Account>,
    state: HashMap<Address, usize>,
    preimages: HashMap<B256, (Bytes, u32)>,
    logs: Vec<JournalStateLog>,
    journal: Vec<JournalStateEvent>,
}

impl SovereignJournalAPI for JournalStateWrapper {
    fn new<SDK: SharedAPI>(sdk: &SDK) -> Self {
        todo!()
    }

    fn checkpoint(&self) -> JournalCheckpoint {
        JournalCheckpoint(self.journal.len() as u32, self.logs.len() as u32)
    }

    fn commit<SDK: SharedAPI>(&mut self, sdk: &SDK) {
        // for (key, value) in self
        //     .journal
        //     .iter()
        //     .map(|v| (*v.key(), v.preimage()))
        //     .collect::<HashMap<_, _>>()
        //     .into_iter()
        // {
        //     match value {
        //         Some((value, flags)) => {
        //             self.storage.update(&key[..], flags, &value)?;
        //         }
        //         None => {
        //             self.storage.remove(&key[..])?;
        //         }
        //     }
        // }
        // for (hash, preimage) in self.preimages.iter() {
        //     self.storage
        //         .update_preimage(hash, Bytes::from(preimage.clone()));
        // }
        // self.journal.clear();
        // self.preimages.clear();
        // self.state.clear();
        // let logs = take(&mut self.logs);
        // self.root = self.storage.compute_root();
        // Ok((self.root, logs))
    }

    fn rollback(&mut self, checkpoint: JournalCheckpoint) {
        if checkpoint.state() > self.journal.len() {
            panic!(
                "checkpoint overflow during rollback ({} > {})",
                checkpoint.state(),
                self.journal.len()
            )
        }
        self.journal
            .iter()
            .rev()
            .take(self.journal.len() - checkpoint.state())
            .for_each(|v| match v {
                JournalStateEvent::AccountChanged {
                    address,
                    prev_state,
                    ..
                } => match prev_state {
                    Some(prev_state) => {
                        self.state.insert(*address, *prev_state);
                    }
                    None => {
                        self.state.remove(address);
                    }
                },
                JournalStateEvent::StorageChanged {
                    address,
                    slot,
                    had_value,
                } => {
                    self.storage.insert((*address, *slot), *had_value);
                }
                JournalStateEvent::PreimageChanged { hash } => {
                    let entry = self.preimages.get_mut(hash).unwrap();
                    entry.1 -= 1;
                    if entry.1 == 0 {
                        self.preimages.remove(hash);
                    }
                }
            });
        self.journal.truncate(checkpoint.state());
        self.logs.truncate(checkpoint.logs());
    }

    fn write_account(&mut self, account: Account, status: AccountStatus) {
        let prev_state = self.state.get(&account.address).copied();
        self.state.insert(account.address, self.journal.len());
        self.journal.push(JournalStateEvent::AccountChanged {
            address: account.address,
            account_status: status,
            account,
            prev_state,
        });
    }

    fn account(&self, address: &Address) -> (&Account, IsColdAccess) {
        match self.state.get(address) {
            Some(index) => (self.journal.get(*index).unwrap().unwrap_account(), false),
            None => self.account_committed(address),
        }
    }

    fn account_committed(&self, address: &Address) -> (&Account, IsColdAccess) {
        (
            self.accounts
                .get(address)
                .unwrap_or_else(|| unreachable!("missing account: {}", address)),
            false,
        )
    }

    fn write_preimage(&mut self, hash: B256, preimage: Bytes) {
        match self.preimages.entry(hash) {
            Entry::Occupied(mut entry) => {
                // increment ref count
                entry.get_mut().1 += 1;
            }
            Entry::Vacant(entry) => {
                entry.insert((preimage, 1u32));
            }
        }
        self.journal
            .push(JournalStateEvent::PreimageChanged { hash })
    }

    fn preimage(&self, hash: &B256) -> Option<&[u8]> {
        self.preimages.get(hash).map(|v| v.0.as_ref())
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        self.preimages
            .get(hash)
            .map(|v| v.0.len() as u32)
            .unwrap_or(0)
    }

    fn write_storage(&mut self, address: Address, slot: U256, value: U256) -> IsColdAccess {
        let had_value = match self.storage.entry((address, slot)) {
            Entry::Occupied(mut entry) => entry.insert(value),
            Entry::Vacant(entry) => {
                entry.insert(value);
                U256::ZERO
            }
        };
        self.journal.push(JournalStateEvent::StorageChanged {
            address,
            slot,
            had_value,
        });
        // we don't support cold storage right now
        false
    }

    fn storage(&self, address: Address, slot: U256) -> (U256, IsColdAccess) {
        let value = self
            .storage
            .get(&(address, slot))
            .copied()
            .unwrap_or(U256::ZERO);
        // we don't support cold storage
        (value, false)
    }

    fn committed_storage(&self, address: Address, slot: U256) -> (U256, IsColdAccess) {
        todo!("not supported yet")
    }

    fn write_log(&mut self, address: Address, data: Bytes, topics: &[B256]) {
        self.logs.push(JournalStateLog {
            address,
            topics: topics.to_vec(),
            data,
        });
    }

    fn context_call<SDK: SharedAPI>(
        &mut self,
        sdk: &SDK,
        caller: Address,
        address: Address,
        value: U256,
        fuel: &mut Fuel,
        input: &[u8],
    ) -> (Bytes, ExitCode) {
        todo!()
    }
}
