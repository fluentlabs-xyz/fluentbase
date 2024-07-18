use crate::{
    Account,
    ContractInput,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_FIELDS_COUNT,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    U256,
};
use alloc::rc::Rc;
use byteorder::{ByteOrder, LittleEndian};
use core::ptr;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_genesis::devnet::{
    devnet_genesis,
    devnet_genesis_from_file,
    KECCAK_HASH_KEY,
    POSEIDON_HASH_KEY,
};
use fluentbase_runtime::{
    instruction::{
        charge_fuel::SyscallChargeFuel,
        checkpoint::SyscallCheckpoint,
        commit::SyscallCommit,
        compute_root::SyscallComputeRoot,
        context_call::SyscallContextCall,
        debug_log::SyscallDebugLog,
        ecrecover::SyscallEcrecover,
        emit_log::SyscallEmitLog,
        exec::SyscallExec,
        exit::SyscallExit,
        forward_output::SyscallForwardOutput,
        get_leaf::SyscallGetLeaf,
        input_size::SyscallInputSize,
        keccak256::SyscallKeccak256,
        output_size::SyscallOutputSize,
        poseidon::SyscallPoseidon,
        poseidon_hash::SyscallPoseidonHash,
        preimage_copy::SyscallPreimageCopy,
        preimage_size::SyscallPreimageSize,
        read::SyscallRead,
        read_context::SyscallReadContext,
        read_output::SyscallReadOutput,
        rollback::SyscallRollback,
        state::SyscallState,
        update_leaf::SyscallUpdateLeaf,
        update_preimage::SyscallUpdatePreimage,
        write::SyscallWrite,
    },
    types::InMemoryTrieDb,
    zktrie::ZkTrieStateDb,
    DefaultEmptyRuntimeDatabase,
    RuntimeContext,
};
use fluentbase_types::{
    address,
    calc_storage_key,
    AccountCheckpoint,
    AccountStatus,
    Address,
    Bytes,
    ContextReader,
    ExitCode,
    Fuel,
    IJournaledTrie,
    JournalCheckpoint,
    SharedAPI,
    SovereignAPI,
    UnwrapExitCode,
    B256,
    F254,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
    JZKT_STORAGE_COMPRESSION_FLAGS,
    KECCAK_EMPTY,
    POSEIDON_EMPTY,
};
use std::{cell::RefCell, mem::take, ops::Deref};

#[derive(Clone, Default)]
pub struct ContextReaderWrapper(Rc<RefCell<ContractInput>>);

impl ContextReaderWrapper {
    pub fn new_from_input(contract_input: ContractInput) -> Self {
        Self(Rc::new(RefCell::new(contract_input)))
    }

    pub fn new(input: &[u8]) -> Self {
        let mut contract_input = ContractInput::default();
        let mut buffer_decoder = BufferDecoder::new(input);
        ContractInput::decode_body(&mut buffer_decoder, 0, &mut contract_input);
        Self::new_from_input(contract_input)
    }
}

impl ContextReader for ContextReaderWrapper {
    fn block_chain_id(&self) -> u64 {
        self.0.borrow().block_chain_id()
    }

    fn block_coinbase(&self) -> Address {
        self.0.borrow().block_coinbase()
    }

    fn block_timestamp(&self) -> u64 {
        self.0.borrow().block_timestamp()
    }

    fn block_number(&self) -> u64 {
        self.0.borrow().block_number()
    }

    fn block_difficulty(&self) -> u64 {
        self.0.borrow().block_difficulty()
    }

    fn block_prevrandao(&self) -> B256 {
        self.0.borrow().block_prevrandao()
    }

    fn block_gas_limit(&self) -> u64 {
        self.0.borrow().block_gas_limit()
    }

    fn block_base_fee(&self) -> U256 {
        self.0.borrow().block_base_fee()
    }

    fn tx_gas_limit(&self) -> u64 {
        self.0.borrow().tx_gas_limit()
    }

    fn tx_nonce(&self) -> u64 {
        self.0.borrow().tx_nonce()
    }

    fn tx_gas_price(&self) -> U256 {
        self.0.borrow().tx_gas_price()
    }

    fn tx_caller(&self) -> Address {
        self.0.borrow().tx_caller()
    }

    fn tx_access_list(&self) -> Vec<(Address, Vec<U256>)> {
        self.0.borrow().tx_access_list()
    }

    fn tx_gas_priority_fee(&self) -> Option<U256> {
        self.0.borrow().tx_gas_priority_fee()
    }

    fn tx_blob_hashes(&self) -> Vec<B256> {
        self.0.borrow().tx_blob_hashes()
    }

    fn tx_blob_hashes_size(&self) -> (u32, u32) {
        self.0.borrow().tx_blob_hashes_size()
    }

    fn tx_max_fee_per_blob_gas(&self) -> Option<U256> {
        self.0.borrow().tx_max_fee_per_blob_gas()
    }

    fn contract_gas_limit(&self) -> u64 {
        self.0.borrow().contract_gas_limit()
    }

    fn contract_address(&self) -> Address {
        self.0.borrow().contract_address()
    }

    fn contract_caller(&self) -> Address {
        self.0.borrow().contract_caller()
    }

    fn contract_value(&self) -> U256 {
        self.0.borrow().contract_value()
    }

    fn contract_is_static(&self) -> bool {
        self.0.borrow().contract_is_static()
    }
}

pub struct RuntimeContextWrapper<DB: IJournaledTrie> {
    ctx: Rc<RefCell<RuntimeContext<DB>>>,
}

impl<DB: IJournaledTrie> Clone for RuntimeContextWrapper<DB> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
        }
    }
}

impl<DB: IJournaledTrie> SharedAPI for RuntimeContextWrapper<DB> {
    fn keccak256(data: &[u8]) -> B256 {
        SyscallKeccak256::fn_impl(data)
    }

    fn poseidon(data: &[u8]) -> F254 {
        SyscallPoseidon::fn_impl(data)
    }

    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254 {
        SyscallPoseidonHash::fn_impl(fa, fb, fd).unwrap_exit_code()
    }

    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
        SyscallEcrecover::fn_impl(digest, sig, rec_id).unwrap_exit_code()
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        let result = SyscallRead::fn_impl(&self.ctx.borrow(), offset, target.len() as u32)
            .unwrap_exit_code();
        target.copy_from_slice(&result);
    }

    fn input_size(&self) -> u32 {
        SyscallInputSize::fn_impl(&self.ctx.borrow())
    }

    fn write(&self, value: &[u8]) {
        SyscallWrite::fn_impl(&mut self.ctx.borrow_mut(), value)
    }

    fn forward_output(&self, offset: u32, len: u32) {
        SyscallForwardOutput::fn_impl(&mut self.ctx.borrow_mut(), offset, len).unwrap_exit_code()
    }

    fn exit(&self, exit_code: i32) -> ! {
        SyscallExit::fn_impl(&mut self.ctx.borrow_mut(), exit_code).unwrap_exit_code();
        loop {}
    }

    fn output_size(&self) -> u32 {
        SyscallOutputSize::fn_impl(&self.ctx.borrow())
    }

    fn read_output(&self, target: &mut [u8], offset: u32) {
        let result = SyscallReadOutput::fn_impl(&self.ctx.borrow(), offset, target.len() as u32)
            .unwrap_exit_code();
        target.copy_from_slice(&result);
    }

    fn state(&self) -> u32 {
        SyscallState::fn_impl(&self.ctx.borrow())
    }

    fn read_context(&self, target: &mut [u8], offset: u32) {
        let result = SyscallReadContext::fn_impl(&self.ctx.borrow(), offset, target.len() as u32)
            .unwrap_exit_code();
        target.copy_from_slice(&result);
    }

    fn charge_fuel(&self, fuel: &mut Fuel) {
        fuel.0 = SyscallChargeFuel::fn_impl(&mut self.ctx.borrow_mut(), fuel.0);
    }

    fn account(&self, address: &Address) -> (Account, bool) {
        let mut result = Account::new(*address);
        let address_word = address.into_word();
        // code size and nonce
        let (buffer32, is_cold) = SyscallGetLeaf::fn_impl(
            &self.ctx.borrow(),
            address_word.as_slice(),
            JZKT_ACCOUNT_NONCE_FIELD,
            false,
        )
        .unwrap_or_default();
        result.nonce = LittleEndian::read_u64(&buffer32);
        result.balance = SyscallGetLeaf::fn_impl(
            &self.ctx.borrow(),
            address_word.as_slice(),
            JZKT_ACCOUNT_BALANCE_FIELD,
            false,
        )
        .map_or(U256::ZERO, |v| U256::from_le_slice(&v.0[..]));
        result.rwasm_code_size = SyscallGetLeaf::fn_impl(
            &self.ctx.borrow(),
            address_word.as_slice(),
            JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
            false,
        )
        .map(|v| LittleEndian::read_u64(&v.0))
        .unwrap_or_default();
        result.rwasm_code_hash = SyscallGetLeaf::fn_impl(
            &self.ctx.borrow(),
            address_word.as_slice(),
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            false,
        )
        .map_or(F254::ZERO, |v| F254::from_slice(&v.0[..]));
        result.source_code_size = SyscallGetLeaf::fn_impl(
            &self.ctx.borrow(),
            address_word.as_slice(),
            JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
            false,
        )
        .map(|v| LittleEndian::read_u64(&v.0))
        .unwrap_or_default();
        result.source_code_hash = SyscallGetLeaf::fn_impl(
            &self.ctx.borrow(),
            address_word.as_slice(),
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            false,
        )
        .map_or(B256::ZERO, |v| B256::from_slice(&v.0[..]));
        (result, is_cold)
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        return SyscallPreimageSize::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap();
    }

    fn preimage_copy(&self, target: &mut [u8], hash: &B256) {
        let result = SyscallPreimageCopy::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap();
        target.copy_from_slice(&result);
    }

    fn log(&self, address: &Address, data: Bytes, topics: &[B256]) {
        SyscallEmitLog::fn_impl(&self.ctx.borrow(), *address, topics.to_vec(), data)
    }

    fn system_call(&self, address: &Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode) {
        let (callee, _) = self.account(address);
        let exit_code = match SyscallExec::fn_impl(
            &mut self.ctx.borrow_mut(),
            &callee.rwasm_code_hash.0,
            input,
            0,
            fuel.0,
        ) {
            Ok(remaining_fuel) => {
                fuel.0 = remaining_fuel;
                ExitCode::Ok
            }
            Err(err) => err,
        };
        (self.ctx.borrow().return_data().clone().into(), exit_code)
    }

    fn debug(&self, msg: &[u8]) {
        SyscallDebugLog::fn_impl(msg)
    }
}

impl<DB: IJournaledTrie> SovereignAPI for RuntimeContextWrapper<DB> {
    fn checkpoint(&self) -> u64 {
        let result = SyscallCheckpoint::fn_impl(&self.ctx.borrow()).unwrap();
        result.to_u64()
    }

    fn commit(&self) {
        let _root = SyscallCommit::fn_impl(&self.ctx.borrow()).unwrap();
    }

    fn rollback(&self, checkpoint: AccountCheckpoint) {
        SyscallRollback::fn_impl(&self.ctx.borrow(), JournalCheckpoint::from_u64(checkpoint))
            .unwrap_exit_code();
    }

    fn write_account(&self, account: &Account, _status: AccountStatus) {
        let account_address = account.address.into_word();
        let account_fields = account.get_fields();
        SyscallUpdateLeaf::fn_impl(
            &mut self.ctx.borrow_mut(),
            account_address.as_slice(),
            JZKT_ACCOUNT_COMPRESSION_FLAGS,
            account_fields.to_vec(),
        )
        .unwrap_exit_code();
    }

    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) {
        SyscallUpdatePreimage::fn_impl(&self.ctx.borrow(), key, field, preimage).unwrap_exit_code();
    }

    fn context_call(
        &self,
        address: &Address,
        input: &[u8],
        context: &[u8],
        fuel: &mut Fuel,
        state: u32,
    ) -> (Bytes, ExitCode) {
        let (callee, _) = self.account(address);
        let exit_code = match SyscallContextCall::fn_impl(
            &mut self.ctx.borrow_mut(),
            &callee.rwasm_code_hash.0,
            input.to_vec(),
            context.to_vec(),
            0,
            fuel.0,
            state,
        ) {
            Ok(remaining_fuel) => {
                fuel.0 = remaining_fuel;
                ExitCode::Ok
            }
            Err(err) => err,
        };
        (self.ctx.borrow().return_data().clone().into(), exit_code)
    }

    fn storage(&self, address: &Address, slot: &U256, committed: bool) -> (U256, bool) {
        // TODO(dmitry123): "what if account is newly created? then result value must be zero"
        let storage_key = calc_storage_key::<Self>(&address, slot.as_le_slice().as_ptr());
        SyscallGetLeaf::fn_impl(&self.ctx.borrow(), storage_key.as_slice(), 0, committed)
            .map_or((U256::ZERO, true), |v| (U256::from_le_slice(&v.0[..]), v.1))
    }

    fn write_storage(&self, address: &Address, slot: &U256, value: &U256) -> bool {
        let storage_key = calc_storage_key::<Self>(&address, slot.as_le_slice().as_ptr());
        let value32 = value.to_le_bytes::<32>();
        SyscallUpdateLeaf::fn_impl(
            &self.ctx.borrow(),
            storage_key.as_slice(),
            JZKT_STORAGE_COMPRESSION_FLAGS,
            vec![value32],
        )
        .unwrap_exit_code();
        true
    }

    fn write_log(&self, address: &Address, data: &Bytes, topics: &[B256]) {
        SyscallEmitLog::fn_impl(&self.ctx.borrow(), *address, topics.to_vec(), data.clone())
    }

    fn precompile(
        &self,
        _address: &Address,
        _input: &Bytes,
        _gas: u64,
    ) -> Option<(Bytes, ExitCode, u64, i64)> {
        None
    }

    fn is_precompile(&self, _address: &Address) -> bool {
        false
    }

    fn transfer(&self, from: &mut Account, to: &mut Account, value: U256) -> Result<(), ExitCode> {
        Account::transfer(from, to, value)
    }

    fn self_destruct(&self, _address: Address, _target: Address) -> [bool; 4] {
        todo!("not supported")
    }

    fn block_hash(&self, _number: U256) -> B256 {
        B256::ZERO
    }

    fn write_transient_storage(&self, _address: Address, _index: U256, _value: U256) {
        todo!("not supported")
    }

    fn transient_storage(&self, _address: Address, _index: U256) -> U256 {
        todo!("not supported")
    }
}

pub type TestingContext = RuntimeContextWrapper<DefaultEmptyRuntimeDatabase>;

impl TestingContext {
    pub fn new() -> Self {
        let ctx = RuntimeContext::<DefaultEmptyRuntimeDatabase>::default()
            .with_jzkt(DefaultEmptyRuntimeDatabase::default());
        Self {
            ctx: Rc::new(RefCell::new(ctx)),
        }
    }

    pub fn with_input<I: Into<Vec<u8>>>(self, input: I) -> Self {
        self.ctx
            .replace_with(|ctx| take(ctx).with_input(input.into()));
        self
    }

    pub fn with_context<I: Into<Vec<u8>>>(self, context: I) -> Self {
        let context: Vec<u8> = context.into();
        self.ctx.replace_with(|ctx| take(ctx).with_context(context));
        self
    }

    pub fn output(&self) -> Vec<u8> {
        self.ctx.borrow().output().clone()
    }

    pub fn with_devnet_genesis(self) -> Self {
        let devnet_genesis = devnet_genesis_from_file();
        for (address, account) in devnet_genesis.alloc.iter() {
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
            let address32 = address.into_word();
            self.write_account(&account2, AccountStatus::NewlyCreated);
            let bytecode = account.code.clone().unwrap_or_default();
            // TODO(dmitry123): "is it true that source matches rwasm in genesis file?"
            self.update_preimage(&address32.0, JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD, &bytecode);
            self.update_preimage(&address32.0, JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD, &bytecode);
        }
        self
    }
}
