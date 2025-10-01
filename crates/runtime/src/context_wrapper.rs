use crate::{
    syscall_handler::{
        ecc::{Secp256r1VerifyConfig, SyscallEccRecover, SyscallWeierstrassVerifyAssign},
        *,
    },
    RuntimeContext,
};
use fluentbase_sdk::{
    BytecodeOrHash, Bytes, BytesOrRef, ExitCode, NativeAPI, UnwrapExitCode, B256,
};
use std::cell::RefCell;

#[derive(Default)]
pub struct RuntimeContextWrapper {
    pub ctx: RefCell<RuntimeContext>,
}

impl RuntimeContextWrapper {
    pub fn new(ctx: RuntimeContext) -> Self {
        Self {
            ctx: RefCell::new(ctx),
        }
    }
    pub fn into_inner(self) -> RuntimeContext {
        self.ctx.into_inner()
    }
}

impl NativeAPI for RuntimeContextWrapper {
    fn keccak256(data: &[u8]) -> B256 {
        SyscallKeccak256::fn_impl(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        SyscallSha256::fn_impl(data)
    }

    fn blake3(data: &[u8]) -> B256 {
        SyscallBlake3::fn_impl(data)
    }
    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode> {
        SyscallPoseidon::fn_impl(parameters as u64, endianness as u64, data)
    }

    /////// Weierstrass curves ///////

    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        SyscallEccRecover::<Secp256k1RecoverConfig>::fn_impl(digest, sig, rec_id).map(|v| {
            let mut result = [0u8; 65];
            let min = core::cmp::min(result.len(), v.len());
            result[..min].copy_from_slice(&v[..min]);
            result
        })
    }

    fn curve256r1_verify(input: &[u8]) -> bool {
        SyscallWeierstrassVerifyAssign::<Secp256r1VerifyConfig>::fn_impl(input)
    }

    fn debug_log(message: &str) {
        SyscallDebugLog::fn_impl(message.as_bytes())
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        let result =
            SyscallRead::fn_impl(&mut self.ctx.borrow_mut(), offset, target.len() as u32).unwrap();
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

    fn exit(&self, exit_code: ExitCode) -> ! {
        SyscallExit::fn_impl(&mut self.ctx.borrow_mut(), exit_code).unwrap_exit_code();
        unreachable!("exit code: {}", exit_code)
    }

    fn output_size(&self) -> u32 {
        SyscallOutputSize::fn_impl(&self.ctx.borrow())
    }

    fn read_output(&self, target: &mut [u8], offset: u32) {
        let result =
            SyscallReadOutput::fn_impl(&mut self.ctx.borrow_mut(), offset, target.len() as u32)
                .unwrap();
        target.copy_from_slice(&result);
    }

    fn state(&self) -> u32 {
        SyscallState::fn_impl(&self.ctx.borrow())
    }

    #[inline(always)]
    fn fuel(&self) -> u64 {
        SyscallFuel::fn_impl(&self.ctx.borrow())
    }

    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        SyscallChargeFuelManually::fn_impl(&mut self.ctx.borrow_mut(), fuel_consumed, fuel_refunded)
            .unwrap()
    }

    fn charge_fuel(&self, fuel_consumed: u64) {
        SyscallChargeFuel::fn_impl(&mut self.ctx.borrow_mut(), fuel_consumed).unwrap();
    }

    fn exec(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let (fuel_consumed, fuel_refunded, exit_code) = SyscallExec::fn_impl(
            &mut self.ctx.borrow_mut(),
            code_hash,
            BytesOrRef::Ref(input),
            fuel_limit.unwrap_or(u64::MAX),
            state,
        );
        (fuel_consumed, fuel_refunded, exit_code)
    }

    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64, i32) {
        let (fuel_consumed, fuel_refunded, exit_code) = SyscallResume::fn_impl(
            &mut self.ctx.borrow_mut(),
            call_id,
            return_data,
            exit_code,
            fuel_consumed,
            fuel_refunded,
            0,
        );
        (fuel_consumed, fuel_refunded, exit_code)
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        SyscallPreimageSize::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap()
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        let preimage = SyscallPreimageCopy::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap();
        target.copy_from_slice(&preimage);
    }

    fn return_data(&self) -> Bytes {
        let ctx = self.ctx.borrow();
        ctx.execution_result.return_data.clone().into()
    }
}
