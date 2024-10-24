use crate::{Address, Bytes, B256, U256};
use alloc::{vec, vec::Vec};
use core::mem::take;
#[cfg(feature = "std")]
use fluentbase_genesis::{
    devnet_genesis_from_file,
    Genesis,
    GENESIS_KECCAK_HASH_SLOT,
    GENESIS_POSEIDON_HASH_SLOT,
};
use fluentbase_types::{
    Account,
    AccountStatus,
    BlockContext,
    CallPrecompileResult,
    ContextFreeNativeAPI,
    ContractContext,
    DestroyedAccountResult,
    ExitCode,
    IsColdAccess,
    JournalCheckpoint,
    NativeAPI,
    SharedAPI,
    SovereignAPI,
    SovereignStateResult,
    TxContext,
    F254,
    STATE_MAIN,
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
    accounts: Option<HashMap<Address, Account>>,
    storage: Option<HashMap<(Address, U256), U256>>,
    preimages: Option<HashMap<B256, (Bytes, u32)>>,
    block_context: BlockContext,
    tx_context: TxContext,
    contract_context: Option<ContractContext>,
}

impl JournalStateBuilder {
    pub fn build<API: NativeAPI>(self, native_sdk: API) -> JournalState<API> {
        JournalState::<API> {
            storage: self.storage.unwrap_or_default(),
            accounts: self.accounts.unwrap_or_default(),
            dirty_state: Default::default(),
            preimages: self.preimages.unwrap_or_default(),
            logs: vec![],
            journal: vec![],
            native_sdk,
            transient_storage: Default::default(),
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
        use fluentbase_types::{KECCAK_EMPTY, POSEIDON_EMPTY};
        for (address, account) in genesis.alloc.iter() {
            let _source_code_hash = account
                .storage
                .as_ref()
                .and_then(|storage| storage.get(&GENESIS_KECCAK_HASH_SLOT))
                .cloned()
                .unwrap_or(KECCAK_EMPTY);
            let rwasm_code_hash = account
                .storage
                .as_ref()
                .and_then(|storage| storage.get(&GENESIS_POSEIDON_HASH_SLOT))
                .cloned()
                .unwrap_or(POSEIDON_EMPTY);
            let mut account2 = Account::new(*address);
            account2.balance = account.balance;
            account2.nonce = account.nonce.unwrap_or_default();
            account2.code_size = account
                .code
                .as_ref()
                .map(|v| v.len() as u64)
                .unwrap_or_default();
            account2.code_hash = rwasm_code_hash;
            // self.write_account(&account2, AccountStatus::NewlyCreated);
            let bytecode = account.code.clone().unwrap_or_default();
            // TODO(dmitry123): "is it true that source matches rwasm in genesis file?"
            self.add_preimage(account2.code_hash, bytecode.clone());
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
        self.block_context = block_context;
    }

    pub fn with_tx_context(mut self, tx_context: TxContext) -> Self {
        self.add_tx_context(tx_context);
        self
    }

    pub fn add_tx_context(&mut self, tx_context: TxContext) {
        self.tx_context = tx_context;
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
    // committed state
    storage: HashMap<(Address, U256), U256>,
    preimages: HashMap<B256, (Bytes, u32)>,
    accounts: HashMap<Address, Account>,
    // dirty state
    dirty_state: HashMap<Address, usize>,
    logs: Vec<JournalStateLog>,
    journal: Vec<JournalStateEvent>,
    native_sdk: API,
    transient_storage: HashMap<(Address, U256), U256>,
    // block/tx/contract contexts
    block_context: BlockContext,
    tx_context: TxContext,
    contract_context: Option<ContractContext>,
}

impl<API: NativeAPI> JournalState<API> {
    pub fn empty(native_sdk: API) -> Self {
        Self {
            storage: Default::default(),
            accounts: Default::default(),
            dirty_state: Default::default(),
            preimages: Default::default(),
            logs: Default::default(),
            journal: Default::default(),
            native_sdk,
            transient_storage: Default::default(),
            block_context: Default::default(),
            tx_context: Default::default(),
            contract_context: Default::default(),
        }
    }

    pub fn builder(native_sdk: API, builder: JournalStateBuilder) -> Self {
        builder.build(native_sdk)
    }

    pub fn native_sdk_mut(&mut self) -> &mut API {
        &mut self.native_sdk
    }

    pub fn rewrite_tx_context(&mut self, tx_context: TxContext) {
        self.tx_context = tx_context;
    }

    pub fn rewrite_contract_context(&mut self, contract_context: ContractContext) {
        self.contract_context = Some(contract_context);
    }
}

impl<API: NativeAPI> ContextFreeNativeAPI for JournalState<API> {
    fn keccak256(data: &[u8]) -> B256 {
        API::keccak256(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        API::sha256(data)
    }

    fn poseidon(data: &[u8]) -> F254 {
        API::poseidon(data)
    }

    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254 {
        API::poseidon_hash(fa, fb, fd)
    }

    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
        API::ec_recover(digest, sig, rec_id)
    }

    fn debug_log(message: &str) {
        API::debug_log(message)
    }
}

impl<API: NativeAPI> SovereignAPI for JournalState<API> {
    fn native_sdk(&self) -> &impl NativeAPI {
        &self.native_sdk
    }

    fn block_context(&self) -> &BlockContext {
        &self.block_context
    }

    fn tx_context(&self) -> &TxContext {
        &self.tx_context
    }

    fn contract_context(&self) -> Option<&ContractContext> {
        self.contract_context.as_ref()
    }

    fn checkpoint(&self) -> JournalCheckpoint {
        JournalCheckpoint(self.journal.len() as u32, self.logs.len() as u32)
    }

    fn commit(&mut self) -> SovereignStateResult {
        let mut result = SovereignStateResult::default();
        for event in take(&mut self.journal).into_iter() {
            match event {
                JournalStateEvent::AccountChanged { account, .. } => {
                    result.accounts.push(account);
                }
                JournalStateEvent::StorageChanged {
                    address,
                    slot,
                    had_value,
                } => {
                    result.storages.push((address, slot, had_value));
                }
                JournalStateEvent::PreimageChanged { hash } => {
                    let preimage = self.preimages.get(&hash).cloned().map(|v| v.0).unwrap();
                    result.preimages.push((hash, preimage));
                }
            }
        }
        self.journal.clear();
        self.dirty_state.clear();
        result
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
                        self.dirty_state.insert(*address, *prev_state);
                    }
                    None => {
                        self.dirty_state.remove(address);
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
        let prev_state = self.dirty_state.get(&account.address).copied();
        self.dirty_state.insert(account.address, self.journal.len());
        self.journal.push(JournalStateEvent::AccountChanged {
            address: account.address,
            account_status: status,
            account,
            prev_state,
        });
    }

    fn destroy_account(&mut self, _address: &Address, _target: &Address) -> DestroyedAccountResult {
        todo!()
    }

    fn account(&self, address: &Address) -> (Account, IsColdAccess) {
        match self.dirty_state.get(address) {
            Some(index) => (
                self.journal.get(*index).unwrap().unwrap_account().clone(),
                false,
            ),
            None => self.account_committed(address),
        }
    }

    fn account_committed(&self, address: &Address) -> (Account, IsColdAccess) {
        (
            self.accounts.get(address).cloned().unwrap_or_default(),
            false,
        )
    }

    fn write_preimage(&mut self, _address: Address, hash: B256, preimage: Bytes) {
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

    fn preimage(&self, _address: &Address, hash: &B256) -> Option<Bytes> {
        self.preimages.get(hash).map(|v| v.0.clone())
    }

    fn preimage_size(&self, _address: &Address, hash: &B256) -> u32 {
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

    fn storage(&self, address: &Address, slot: &U256) -> (U256, IsColdAccess) {
        let value = self
            .storage
            .get(&(*address, *slot))
            .copied()
            .unwrap_or(U256::ZERO);
        // we don't support cold storage
        (value, false)
    }

    fn committed_storage(&self, _address: &Address, _slot: &U256) -> (U256, IsColdAccess) {
        (U256::ZERO, false)
    }

    fn write_transient_storage(&mut self, address: Address, index: U256, value: U256) {
        self.transient_storage.insert((address, index), value);
    }

    fn transient_storage(&self, address: &Address, index: &U256) -> U256 {
        self.transient_storage
            .get(&(*address, *index))
            .cloned()
            .unwrap_or_default()
    }

    fn write_log(&mut self, address: Address, data: Bytes, topics: Vec<B256>) {
        self.logs.push(JournalStateLog {
            address,
            topics,
            data,
        });
    }

    fn precompile(
        &self,
        _address: &Address,
        _input: &Bytes,
        _gas: u64,
    ) -> Option<CallPrecompileResult> {
        todo!()
    }

    fn is_precompile(&self, _address: &Address) -> bool {
        todo!()
    }

    fn transfer(
        &mut self,
        _from: &mut Account,
        _to: &mut Account,
        _value: U256,
    ) -> Result<(), ExitCode> {
        todo!()
    }
}

impl<API: NativeAPI> SharedAPI for JournalState<API> {
    fn block_context(&self) -> &BlockContext {
        &self.block_context
    }

    fn tx_context(&self) -> &TxContext {
        &self.tx_context
    }

    fn contract_context(&self) -> &ContractContext {
        self.contract_context.as_ref().unwrap()
    }

    fn write_storage(&mut self, slot: U256, value: U256) {
        let caller = self.contract_context.as_ref().map(|v| v.address).unwrap();
        SovereignAPI::write_storage(self, caller, slot, value);
    }

    fn storage(&self, slot: &U256) -> U256 {
        let caller = self.contract_context.as_ref().map(|v| v.address).unwrap();
        let (value, _) = SovereignAPI::storage(self, &caller, slot);
        value
    }

    fn write_transient_storage(&mut self, slot: U256, value: U256) {
        let caller = self.contract_context.as_ref().map(|v| v.address).unwrap();
        SovereignAPI::write_transient_storage(self, caller, slot, value);
    }

    fn transient_storage(&self, slot: &U256) -> U256 {
        let caller = self.contract_context.as_ref().map(|v| v.address).unwrap();
        SovereignAPI::transient_storage(self, &caller, slot)
    }

    fn ext_storage(&self, address: &Address, slot: &U256) -> U256 {
        let (value, _) = SovereignAPI::storage(self, address, slot);
        value
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        self.native_sdk.read(target, offset)
    }

    fn input_size(&self) -> u32 {
        self.native_sdk.input_size()
    }

    fn charge_fuel(&self, value: u64) {
        self.native_sdk.charge_fuel(value);
    }

    fn fuel(&self) -> u64 {
        self.native_sdk.fuel()
    }

    fn write(&mut self, output: &[u8]) {
        self.native_sdk.write(output)
    }

    fn exit(&self, exit_code: i32) -> ! {
        self.native_sdk.exit(exit_code)
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        let preimage = self
            .preimages
            .get(hash)
            .map(|v| v.0.clone())
            .unwrap_or_default();
        target.copy_from_slice(preimage.as_ref());
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        self.preimages
            .get(hash)
            .map(|v| v.0.len() as u32)
            .unwrap_or(0)
    }

    fn emit_log(&mut self, data: Bytes, topics: &[B256]) {
        let address = self.contract_context.as_ref().unwrap().address;
        SovereignAPI::write_log(self, address, data, topics.to_vec());
    }

    fn balance(&self, _address: &Address) -> U256 {
        todo!()
    }

    fn write_preimage(&mut self, preimage: Bytes) -> B256 {
        let address = self.contract_context.as_ref().unwrap().address;
        let code_hash = API::keccak256(preimage.as_ref());
        SovereignAPI::write_preimage(self, address, code_hash, preimage);
        code_hash
    }

    fn create(
        &mut self,
        _fuel_limit: u64,
        _salt: Option<U256>,
        _value: &U256,
        _init_code: &[u8],
    ) -> Result<Address, i32> {
        todo!()
    }

    fn call(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: u64,
    ) -> (Bytes, i32) {
        todo!()
    }

    fn call_code(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: u64,
    ) -> (Bytes, i32) {
        todo!()
    }

    fn delegate_call(
        &mut self,
        _address: Address,
        _input: &[u8],
        _fuel_limit: u64,
    ) -> (Bytes, i32) {
        todo!()
    }

    fn static_call(&mut self, address: Address, input: &[u8], fuel_limit: u64) -> (Bytes, i32) {
        let (account, _) = self.account(&address);
        let (_, exit_code) =
            self.native_sdk
                .exec(&account.code_hash, input, fuel_limit, STATE_MAIN);
        (self.native_sdk.return_data(), exit_code)
    }

    fn destroy_account(&mut self, _address: Address) {
        todo!()
    }
}
