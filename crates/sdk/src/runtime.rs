use crate::{
    Account,
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
    ExitCode,
    Fuel,
    IJournaledTrie,
    JournalCheckpoint,
    NativeAPI,
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

pub struct RuntimeContextWrapper {
    ctx: Rc<RefCell<RuntimeContext>>,
}

impl Clone for RuntimeContextWrapper {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
        }
    }
}

impl NativeAPI for RuntimeContextWrapper {
    fn keccak256(&self, data: &[u8]) -> B256 {
        SyscallKeccak256::fn_impl(data)
    }

    fn poseidon(&self, data: &[u8]) -> F254 {
        SyscallPoseidon::fn_impl(data)
    }

    fn poseidon_hash(&self, fa: &F254, fb: &F254, fd: &F254) -> F254 {
        SyscallPoseidonHash::fn_impl(fa, fb, fd).unwrap_exit_code()
    }

    fn ec_recover(&self, digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
        SyscallEcrecover::fn_impl(digest, sig, rec_id).unwrap_exit_code()
    }

    fn debug_log(&self, message: &str) {
        SyscallDebugLog::fn_impl(message.as_bytes())
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
        unreachable!("exit code: {}", exit_code)
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

    fn exec(
        &self,
        code_hash: F254,
        address: Address,
        input: &[u8],
        context: &[u8],
        fuel: &mut Fuel,
    ) -> i32 {
        todo!()
    }

    fn resume(&self, call_id: i32, exit_code: i32) -> i32 {
        todo!()
    }
}

pub type TestingContext = RuntimeContextWrapper;

impl TestingContext {
    pub fn new() -> Self {
        let ctx = RuntimeContext::default().with_jzkt(Rc::new(RefCell::new(
            DefaultEmptyRuntimeDatabase::default(),
        )));
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
            // self.write_account(&account2, AccountStatus::NewlyCreated);
            let bytecode = account.code.clone().unwrap_or_default();
            // TODO(dmitry123): "is it true that source matches rwasm in genesis file?"
            // self.update_preimage(&address32.0, JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD, &bytecode);
            // self.update_preimage(&address32.0, JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD, &bytecode);
        }
        self
    }
}
