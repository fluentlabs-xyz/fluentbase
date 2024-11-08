use alloc::rc::Rc;
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
        read_output::SyscallReadOutput,
        resume::SyscallResume,
        state::SyscallState,
        write::SyscallWrite,
    },
    types::{NonePreimageResolver, PreimageResolver},
    RuntimeContext,
};
use fluentbase_types::{Bytes, ContextFreeNativeAPI, NativeAPI, UnwrapExitCode, B256, F254};
use std::{cell::RefCell, mem::take};

pub struct RuntimeContextWrapper<'a, PR: PreimageResolver> {
    pub ctx: Rc<RefCell<RuntimeContext>>,
    pub preimage_resolver: &'a PR,
}

impl RuntimeContextWrapper<'static, NonePreimageResolver> {
    pub fn new(ctx: RuntimeContext) -> Self {
        static EMPTY_PREIMAGE_RESOLVER: NonePreimageResolver = NonePreimageResolver;
        Self {
            ctx: Rc::new(RefCell::new(ctx)),
            preimage_resolver: &EMPTY_PREIMAGE_RESOLVER,
        }
    }

    pub fn with_preimage_resolver<'a, PR: PreimageResolver>(
        self,
        preimage_resolver: &'a PR,
    ) -> RuntimeContextWrapper<'a, PR> {
        RuntimeContextWrapper::<'a, PR> {
            ctx: self.ctx,
            preimage_resolver,
        }
    }
}

impl Clone for RuntimeContextWrapper<'static, NonePreimageResolver> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
            preimage_resolver: self.preimage_resolver,
        }
    }
}

impl<'a, PR: PreimageResolver> ContextFreeNativeAPI for RuntimeContextWrapper<'a, PR> {
    fn keccak256(data: &[u8]) -> B256 {
        SyscallKeccak256::fn_impl(data)
    }

    fn sha256(_data: &[u8]) -> B256 {
        todo!("not implemented")
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

    fn debug_log(message: &str) {
        SyscallDebugLog::fn_impl(message.as_bytes())
    }
}

impl<'a, PR: PreimageResolver> NativeAPI for RuntimeContextWrapper<'a, PR> {
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

    #[inline(always)]
    fn fuel(&self) -> u64 {
        let ctx = self.ctx.borrow();
        SyscallFuel::fn_impl(&ctx)
    }

    fn charge_fuel(&self, value: u64) -> u64 {
        let mut ctx = self.ctx.borrow_mut();
        SyscallChargeFuel::fn_impl(&mut ctx, value)
    }

    fn exec(&self, code_hash: &F254, input: &[u8], fuel_limit: u64, state: u32) -> (u64, i32) {
        let mut ctx = self.ctx.borrow_mut();
        let (fuel_consumed, exit_code) = SyscallExec::fn_impl_ex(
            &mut ctx,
            &code_hash,
            input,
            fuel_limit,
            state,
            self.preimage_resolver,
        );
        (fuel_consumed, exit_code)
    }

    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_used: u64,
    ) -> (u64, i32) {
        let mut ctx = self.ctx.borrow_mut();
        SyscallResume::fn_impl(
            &mut ctx,
            call_id,
            return_data.to_vec(),
            exit_code,
            fuel_used,
        )
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        if let Some(preimage_size) = self.preimage_resolver.preimage_size(&hash.0) {
            return preimage_size;
        }
        SyscallPreimageSize::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap_exit_code()
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        if let Some(preimage) = self.preimage_resolver.preimage(&hash.0) {
            target.copy_from_slice(&preimage);
            return;
        }
        let preimage =
            SyscallPreimageCopy::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap_exit_code();
        target.copy_from_slice(&preimage);
    }

    fn return_data(&self) -> Bytes {
        self.ctx.borrow_mut().return_data().clone().into()
    }
}

pub type TestingContext = RuntimeContextWrapper<'static, NonePreimageResolver>;

impl TestingContext {
    pub fn empty() -> Self {
        Self::new(RuntimeContext::default())
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
        self.ctx.replace_with(|ctx| take(ctx).with_fuel_limit(fuel));
    }

    pub fn take_output(&self) -> Vec<u8> {
        take(self.ctx.borrow_mut().output_mut())
    }
}
