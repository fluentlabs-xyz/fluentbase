use alloc::rc::Rc;
use fluentbase_genesis::devnet::{devnet_genesis_from_file, KECCAK_HASH_KEY, POSEIDON_HASH_KEY};
use fluentbase_runtime::{
    instruction::{
        charge_fuel::SyscallChargeFuel,
        debug_log::SyscallDebugLog,
        ecrecover::SyscallEcrecover,
        exec::SyscallExec,
        exit::SyscallExit,
        forward_output::SyscallForwardOutput,
        fuel::SyscallFuel,
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
        resume::SyscallResume,
        state::SyscallState,
        write::SyscallWrite,
    },
    DefaultEmptyRuntimeDatabase,
    RuntimeContext,
};
use fluentbase_types::{
    Account,
    Bytes,
    NativeAPI,
    UnwrapExitCode,
    B256,
    F254,
    KECCAK_EMPTY,
    POSEIDON_EMPTY,
};
use std::{cell::RefCell, mem::take};

pub struct RuntimeContextWrapper {
    pub ctx: Rc<RefCell<RuntimeContext>>,
}

impl RuntimeContextWrapper {
    pub fn new(ctx: RuntimeContext) -> Self {
        Self {
            ctx: Rc::new(RefCell::new(ctx)),
        }
    }
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

    #[inline(always)]
    fn fuel(&self) -> u64 {
        let ctx = self.ctx.borrow();
        SyscallFuel::fn_impl(&ctx)
    }

    fn charge_fuel(&self, value: u64) -> u64 {
        let mut ctx = self.ctx.borrow_mut();
        SyscallChargeFuel::fn_impl(&mut ctx, value)
    }

    fn exec(&self, code_hash: &F254, input: &[u8], fuel_limit: u64, state: u32) -> i32 {
        let exit_code = SyscallExec::fn_impl(
            &mut self.ctx.borrow_mut(),
            &code_hash.0,
            input,
            fuel_limit,
            state,
        );
        exit_code
    }

    fn resume(&self, call_id: u32, return_data: &[u8], exit_code: i32) -> i32 {
        let exit_code = SyscallResume::fn_impl(
            &mut self.ctx.borrow_mut(),
            call_id,
            return_data.to_vec(),
            exit_code,
        );
        exit_code
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        SyscallPreimageSize::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap_exit_code()
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        let preimage =
            SyscallPreimageCopy::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap_exit_code();
        target.copy_from_slice(&preimage);
    }

    fn return_data(&self) -> Bytes {
        self.ctx.borrow_mut().return_data().clone().into()
    }
}

pub type TestingContext = RuntimeContextWrapper;

impl TestingContext {
    pub fn empty() -> Self {
        let ctx =
            RuntimeContext::default().with_jzkt(Box::new(DefaultEmptyRuntimeDatabase::default()));
        Self {
            ctx: Rc::new(RefCell::new(ctx)),
        }
    }

    pub fn with_input<I: Into<Vec<u8>>>(mut self, input: I) -> Self {
        self.set_input(input);
        self
    }

    pub fn set_input<I: Into<Vec<u8>>>(&mut self, input: I) {
        self.ctx
            .replace_with(|ctx| take(ctx).with_input(input.into()));
    }

    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.set_fuel(fuel);
        self
    }

    pub fn set_fuel(&mut self, fuel: u64) {
        self.ctx.replace_with(|ctx| take(ctx).with_fuel(fuel));
    }

    pub fn with_context<I: Into<Vec<u8>>>(self, context: I) -> Self {
        let context: Vec<u8> = context.into();
        self.ctx.replace_with(|ctx| take(ctx).with_context(context));
        self
    }

    pub fn take_output(&self) -> Vec<u8> {
        take(self.ctx.borrow_mut().output_mut())
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
            // let address32 = address.into_word();
            // self.write_account(&account2, AccountStatus::NewlyCreated);
            // let bytecode = account.code.clone().unwrap_or_default();
            // TODO(dmitry123): "is it true that source matches rwasm in genesis file?"
            // self.update_preimage(&address32.0, JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD, &bytecode);
            // self.update_preimage(&address32.0, JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD, &bytecode);
        }
        self
    }

    // pub(crate) fn add_wasm_contract<I: Into<RwasmModule>>(
    //     &mut self,
    //     address: Address,
    //     rwasm_module: I,
    // ) -> AccountInfo {
    //     let rwasm_binary = {
    //         let rwasm_module: RwasmModule = rwasm_module.into();
    //         let mut result = Vec::new();
    //         rwasm_module.write_binary_to_vec(&mut result).unwrap();
    //         result
    //     };
    //     let account = Account {
    //         address,
    //         balance: U256::ZERO,
    //         nonce: 0,
    //         // it makes not much sense to fill these fields, but it optimizes hash calculation a
    // bit         source_code_size: 0,
    //         source_code_hash: KECCAK_EMPTY,
    //         rwasm_code_size: rwasm_binary.len() as u64,
    //         rwasm_code_hash: poseidon_hash(&rwasm_binary).into(),
    //     };
    //     let mut info: AccountInfo = account.into();
    //     info.code = None;
    //     if !rwasm_binary.is_empty() {
    //         info.rwasm_code = Some(Bytecode::new_raw(rwasm_binary.into()));
    //     }
    //     self.db.insert_account_info(address, info.clone());
    //     info
    // }
    //
    // pub(crate) fn get_balance(&mut self, address: Address) -> U256 {
    //     let account = self.db.load_account(address).unwrap();
    //     account.info.balance
    // }
    //
    // pub(crate) fn add_balance(&mut self, address: Address, value: U256) {
    //     let account = self.db.load_account(address).unwrap();
    //     account.info.balance += value;
    //     let mut revm_account = crate::primitives::Account::from(account.info.clone());
    //     revm_account.mark_touch();
    //     self.db.commit(HashMap::from([(address, revm_account)]));
    // }
}
