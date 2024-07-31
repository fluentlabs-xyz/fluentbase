use crate::{Address, Bytes, B256, U256};
use alloc::{vec, vec::Vec};
#[cfg(feature = "std")]
use fluentbase_genesis::{
    devnet::{devnet_genesis_from_file, KECCAK_HASH_KEY, POSEIDON_HASH_KEY},
    Genesis,
};
use fluentbase_types::{
    Account,
    AccountCheckpoint,
    AccountStatus,
    BlockContext,
    ContractContext,
    ExitCode,
    Fuel,
    IsColdAccess,
    JournalCheckpoint,
    NativeAPI,
    SharedAPI,
    SovereignAPI,
    TxContext,
    F254,
    KECCAK_EMPTY,
    POSEIDON_EMPTY,
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

#[derive(Default)]
pub struct JournalStateBuilder {
    storage: Option<HashMap<(Address, U256), U256>>,
    accounts: Option<HashMap<Address, Account>>,
    preimages: Option<HashMap<B256, (Bytes, u32)>>,
    block_context: Option<BlockContext>,
    tx_context: Option<TxContext>,
    contract_context: Option<ContractContext>,
}

impl JournalStateBuilder {
    pub fn build<API: NativeAPI>(self, native_sdk: API) -> JournalState<API> {
        JournalState::<API> {
            storage: self.storage.unwrap_or_default(),
            accounts: self.accounts.unwrap_or_default(),
            state: Default::default(),
            preimages: self.preimages.unwrap(),
            logs: vec![],
            journal: vec![],
            native_sdk,
            block_context: self.block_context,
            tx_context: self.tx_context,
            contract_context: self.contract_context,
        }
    }

    #[cfg(feature = "std")]
    pub fn with_devnet_genesis(mut self) -> Self {
        self.add_devnet_genesis();
        self
    }

    #[cfg(feature = "std")]
    pub fn add_devnet_genesis(&mut self) {
        self.add_genesis(devnet_genesis_from_file())
    }

    #[cfg(feature = "std")]
    pub fn with_genesis(mut self, genesis: Genesis) -> Self {
        self.add_genesis(genesis);
        self
    }

    #[cfg(feature = "std")]
    pub fn add_genesis(&mut self, genesis: Genesis) {
        for (address, account) in genesis.alloc.iter() {
            let source_code_hash = account
                .storage
                .as_ref()
                .and_then(|storage| storage.get(&KECCAK_HASH_KEY))
                .cloned()
                .unwrap_or(KECCAK_EMPTY);
            let rwasm_code_hash = account
                .storage
                .as_ref()
                .and_then(|storage| storage.get(&POSEIDON_HASH_KEY))
                .cloned()
                .unwrap_or(POSEIDON_EMPTY);
            let mut account2 = Account::new(*address);
            account2.balance = account.balance;
            account2.nonce = account.nonce.unwrap_or_default();
            account2.source_code_size = account
                .code
                .as_ref()
                .map(|v| v.len() as u64)
                .unwrap_or_default();
            account2.source_code_hash = source_code_hash;
            account2.rwasm_code_size = account
                .code
                .as_ref()
                .map(|v| v.len() as u64)
                .unwrap_or_default();
            account2.rwasm_code_hash = rwasm_code_hash;
            // self.write_account(&account2, AccountStatus::NewlyCreated);
            let bytecode = account.code.clone().unwrap_or_default();
            // TODO(dmitry123): "is it true that source matches rwasm in genesis file?"
            self.add_preimage(account2.source_code_hash, bytecode.clone());
            self.add_preimage(account2.rwasm_code_hash, bytecode);
        }
    }

    pub fn with_storage(mut self, address: Address, slot: U256, value: U256) -> Self {
        self.add_storage(address, slot, value);
        self
    }

    pub fn add_storage(&mut self, address: Address, slot: U256, value: U256) {
        self.storage
            .get_or_insert_with(Default::default)
            .insert((address, slot), value);
    }

    pub fn with_account<I: Into<Account>>(mut self, address: Address, account: I) -> Self {
        self.add_account(address, account);
        self
    }

    pub fn add_account<I: Into<Account>>(&mut self, address: Address, account: I) {
        self.accounts
            .get_or_insert_with(Default::default)
            .insert(address, account.into());
    }

    pub fn with_preimage(mut self, hash: B256, preimage: Bytes) -> Self {
        self.add_preimage(hash, preimage);
        self
    }

    pub fn add_preimage(&mut self, hash: B256, preimage: Bytes) {
        self.preimages
            .get_or_insert_with(Default::default)
            .insert(hash, (preimage, 1));
    }

    pub fn with_block_context(mut self, block_context: BlockContext) -> Self {
        self.add_block_context(block_context);
        self
    }

    pub fn add_block_context(&mut self, block_context: BlockContext) {
        self.block_context.replace(block_context);
    }

    pub fn with_tx_context(mut self, tx_context: TxContext) -> Self {
        self.add_tx_context(tx_context);
        self
    }

    pub fn add_tx_context(&mut self, tx_context: TxContext) {
        self.tx_context.replace(tx_context);
    }

    pub fn with_contract_context(mut self, contract_context: ContractContext) -> Self {
        self.add_contract_context(contract_context);
        self
    }

    pub fn add_contract_context(&mut self, contract_context: ContractContext) {
        self.contract_context.replace(contract_context);
    }
}

pub struct JournalState<API: NativeAPI> {
    storage: HashMap<(Address, U256), U256>,
    accounts: HashMap<Address, Account>,
    state: HashMap<Address, usize>,
    preimages: HashMap<B256, (Bytes, u32)>,
    logs: Vec<JournalStateLog>,
    journal: Vec<JournalStateEvent>,
    native_sdk: API,
    block_context: Option<BlockContext>,
    tx_context: Option<TxContext>,
    contract_context: Option<ContractContext>,
}

impl<API: NativeAPI> JournalState<API> {
    pub fn empty(native_sdk: API) -> Self {
        Self {
            storage: Default::default(),
            accounts: Default::default(),
            state: Default::default(),
            preimages: Default::default(),
            logs: Default::default(),
            journal: Default::default(),
            native_sdk,
            block_context: Default::default(),
            tx_context: Default::default(),
            contract_context: Default::default(),
        }
    }

    pub fn builder(native_sdk: API, builder: JournalStateBuilder) -> Self {
        builder.build(native_sdk)
    }
}

impl<API: NativeAPI> SovereignAPI for JournalState<API> {
    fn native_sdk(&self) -> &impl NativeAPI {
        &self.native_sdk
    }

    fn block_context(&self) -> &BlockContext {
        self.block_context.as_ref().unwrap()
    }

    fn tx_context(&self) -> &TxContext {
        self.tx_context.as_ref().unwrap()
    }

    fn checkpoint(&self) -> JournalCheckpoint {
        JournalCheckpoint(self.journal.len() as u32, self.logs.len() as u32)
    }

    fn commit(&mut self) {
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

    fn account(&self, address: &Address) -> (Account, IsColdAccess) {
        match self.state.get(address) {
            Some(index) => (
                self.journal.get(*index).unwrap().unwrap_account().clone(),
                false,
            ),
            None => self.account_committed(address),
        }
    }

    fn account_committed(&self, address: &Address) -> (Account, IsColdAccess) {
        (
            self.accounts
                .get(address)
                .cloned()
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

    fn context_call(
        &mut self,
        caller: Address,
        address: Address,
        value: U256,
        fuel: &mut Fuel,
        input: &[u8],
        state: u32,
    ) -> (Bytes, ExitCode) {
        todo!()
    }

    fn is_precompile(&self, address: &Address) -> bool {
        todo!()
    }

    fn transfer(
        &mut self,
        from: &mut Account,
        to: &mut Account,
        value: U256,
    ) -> Result<(), ExitCode> {
        todo!()
    }
}

impl<API: NativeAPI> SharedAPI for JournalState<API> {
    fn native_sdk(&self) -> &impl NativeAPI {
        &self.native_sdk
    }

    fn block_context(&self) -> &BlockContext {
        self.block_context.as_ref().unwrap()
    }

    fn tx_context(&self) -> &TxContext {
        self.tx_context.as_ref().unwrap()
    }

    fn contract_context(&self) -> &ContractContext {
        self.contract_context.as_ref().unwrap()
    }

    fn account(&self, address: &Address) -> (Account, IsColdAccess) {
        SovereignAPI::account(self, address)
    }

    fn transfer(&mut self, from: &mut Account, to: &mut Account, amount: U256) {
        todo!()
    }

    fn write_storage(&mut self, slot: U256, value: U256) {
        let caller = self.contract_context.as_ref().map(|v| v.caller).unwrap();
        SovereignAPI::write_storage(self, caller, slot, value);
    }

    fn storage(&self, slot: U256) -> U256 {
        let caller = self.contract_context.as_ref().map(|v| v.caller).unwrap();
        let (value, _) = SovereignAPI::storage(self, caller, slot);
        value
    }

    fn write_log(&mut self, data: Bytes, topics: &[B256]) {
        todo!()
    }

    fn call(&mut self, address: Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode) {
        todo!()
    }

    fn delegate(&mut self, address: Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode) {
        todo!()
    }
}
