use crate::{BytecodeOrHash, ExitCode};
use alloc::borrow::Cow;

/// A trait for providing shared API functionality.
#[rustfmt::skip]
pub trait NativeAPI {
    /// Low-level function that terminates the execution of the program and exits with the specified
    /// exit code.
    fn exit(&self, exit_code: ExitCode) -> !;
    fn state(&self) -> u32;
    fn read(&self, target: &mut [u8], offset: u32);
    /// Returns the size of the input data provided to the runtime environment.
    fn input_size(&self) -> u32;
    fn write(&self, value: &[u8]);
    fn output_size(&self) -> u32;
    fn read_output(&self, target: &mut [u8], offset: u32);
    /// Executes a nested call with specified bytecode poseidon hash.
    fn exec(
        &self,
        code_hash: BytecodeOrHash,
        input: Cow<'_, [u8]>,
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32);
    /// Resumes the execution of a previously suspended function call.
    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64, i32);
    fn forward_output(&self, offset: u32, len: u32);
    fn fuel(&self) -> u64;
    fn debug_log(message: &str);
    /// Charges specified amount of fuel.
    /// In contrast to `_charge_fuel_manually`, can be called from untrusted code since it can only
    /// charge fuel.
    fn charge_fuel(&self, fuel_consumed: u64);
    fn enter_unconstrained(&self);
    fn exit_unconstrained(&self);
    fn write_fd(&self, fd: u32, slice: &[u8]);
}
