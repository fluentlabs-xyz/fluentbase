use crate::{
    instruction::{
        charge_fuel::SyscallChargeFuel,
        charge_fuel_manually::SyscallChargeFuelManually,
        debug_log::SyscallDebugLog,
        ed25519_edwards_add::SyscallED25519EdwardsAdd,
        ed25519_edwards_decompress_validate::SyscallED25519EdwardsDecompressValidate,
        ed25519_edwards_mul::SyscallED25519EdwardsMul,
        ed25519_edwards_multiscalar_mul::SyscallED25519EdwardsMultiscalarMul,
        ed25519_edwards_sub::SyscallED25519EdwardsSub,
        ed25519_ristretto_add::SyscallED25519RistrettoAdd,
        ed25519_ristretto_decompress_validate::SyscallED25519RistrettoDecompressValidate,
        ed25519_ristretto_mul::SyscallED25519RistrettoMul,
        ed25519_ristretto_multiscalar_mul::SyscallED25519RistrettoMultiscalarMul,
        ed25519_ristretto_sub::SyscallED25519RistrettoSub,
        exec::SyscallExec,
        exit::SyscallExit,
        forward_output::SyscallForwardOutput,
        fp2_mul::SyscallFp2Mul,
        fp_op::SyscallFpOp,
        fuel::SyscallFuel,
        input_size::SyscallInputSize,
        keccak256::SyscallKeccak256,
        output_size::SyscallOutputSize,
        preimage_copy::SyscallPreimageCopy,
        preimage_size::SyscallPreimageSize,
        read::SyscallRead,
        read_output::SyscallReadOutput,
        resume::SyscallResume,
        secp256k1_recover::SyscallSecp256k1Recover,
        state::SyscallState,
        weierstrass_add::SyscallWeierstrassAddAssign,
        weierstrass_double::SyscallWeierstrassDoubleAssign,
        weierstrass_mul::SyscallWeierstrassMulAssign,
        weierstrass_multi_pairing::SyscallWeierstrassMultiPairingAssign,
        write::SyscallWrite,
        FieldMul,
    },
    RuntimeContext,
};
use fluentbase_types::{
    bn254_add_common_impl,
    native_api::NativeAPI,
    BytecodeOrHash,
    Bytes,
    ExitCode,
    UnwrapExitCode,
    B256,
};
use sp1_curves::weierstrass::bn254::{Bn254, Bn254BaseField};
use std::{cell::RefCell, mem::take, rc::Rc};

#[derive(Default, Clone)]
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

impl NativeAPI for RuntimeContextWrapper {
    fn keccak256(data: &[u8]) -> B256 {
        SyscallKeccak256::fn_impl(data)
    }

    fn sha256(_data: &[u8]) -> B256 {
        todo!("not implemented")
    }

    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        SyscallSecp256k1Recover::fn_impl(digest, sig, rec_id)
    }

    fn ed25519_edwards_decompress_validate(p: &[u8; 32]) -> bool {
        SyscallED25519EdwardsDecompressValidate::fn_impl(p).map_or_else(|_| false, |_| true)
    }

    fn ed25519_edwards_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallED25519EdwardsAdd::fn_impl(p, q).is_ok()
    }

    fn ed25519_edwards_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallED25519EdwardsSub::fn_impl(p, q).is_ok()
    }

    fn ed25519_edwards_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallED25519EdwardsMul::fn_impl(p, q).is_ok()
    }

    fn ed25519_edwards_multiscalar_mul(pairs: &[([u8; 32], [u8; 32])], out: &mut [u8; 32]) -> bool {
        let result = SyscallED25519EdwardsMultiscalarMul::fn_impl(pairs);
        match result {
            Ok(v) => {
                *out = v.compress().to_bytes();
            }
            Err(_) => return false,
        }
        true
    }

    fn ed25519_ristretto_decompress_validate(p: &[u8; 32]) -> bool {
        SyscallED25519RistrettoDecompressValidate::fn_impl(p).map_or_else(|_| false, |_| true)
    }

    fn ed25519_ristretto_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallED25519RistrettoAdd::fn_impl(p, q).is_ok()
    }

    fn ed25519_ristretto_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallED25519RistrettoSub::fn_impl(p, q).is_ok()
    }

    fn ed25519_ristretto_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallED25519RistrettoMul::fn_impl(p, q).is_ok()
    }
    fn ed25519_ristretto_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        let result = SyscallED25519RistrettoMultiscalarMul::fn_impl(pairs);
        match result {
            Ok(v) => {
                *out = v.compress().to_bytes();
            }
            Err(_) => return false,
        }
        true
    }

    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) {
        let result = bn254_add_common_impl!(
            p,
            q,
            { SyscallWeierstrassDoubleAssign::<Bn254>::fn_impl(p) },
            { SyscallWeierstrassAddAssign::<Bn254>::fn_impl(p, q) }
        );
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn bn254_double(p: &mut [u8; 64]) {
        let result = SyscallWeierstrassDoubleAssign::<Bn254>::fn_impl(p);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        let result = SyscallWeierstrassMulAssign::<Bn254>::fn_impl(p, q);
        p.copy_from_slice(&result);
    }

    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> [u8; 32] {
        let result = SyscallWeierstrassMultiPairingAssign::<Bn254>::fn_impl(elements);
        result.try_into().unwrap()
    }

    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        let result = SyscallFpOp::<Bn254BaseField, FieldMul>::fn_impl(p, q);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        let result = SyscallFp2Mul::<Bn254BaseField>::fn_impl(p, q);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
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
    }

    fn charge_fuel(&self, fuel_consumed: u64) {
        SyscallChargeFuel::fn_impl(&mut self.ctx.borrow_mut(), fuel_consumed);
    }

    fn exec<I: Into<BytecodeOrHash>>(
        &self,
        code_hash: I,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let (fuel_consumed, fuel_refunded, exit_code) = SyscallExec::fn_impl(
            &mut self.ctx.borrow_mut(),
            code_hash,
            input,
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
        self.ctx.borrow_mut().return_data().clone().into()
    }
}

pub type TestingContext = RuntimeContextWrapper;

impl TestingContext {
    pub fn empty() -> Self {
        Self::new(RuntimeContext::default())
    }

    pub fn with_input<I: Into<Bytes>>(mut self, input: I) -> Self {
        self.set_input(input);
        self
    }

    pub fn set_input<I: Into<Bytes>>(&mut self, input: I) {
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

    pub fn exit_code(&self) -> i32 {
        self.ctx.borrow_mut().exit_code()
    }
}
