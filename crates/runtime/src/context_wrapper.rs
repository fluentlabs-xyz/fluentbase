use crate::{syscall_handler::*, RuntimeContext};
use fluentbase_types::{BytecodeOrHash, Bytes, BytesOrRef, ExitCode, NativeAPI, UnwrapExitCode};
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
    fn exit(&self, exit_code: ExitCode) -> ! {
        syscall_exit_impl(&mut self.ctx.borrow_mut(), exit_code).unwrap_exit_code();
        unreachable!("exit code: {}", exit_code)
    }

    fn state(&self) -> u32 {
        syscall_state_impl(&self.ctx.borrow())
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        let result =
            syscall_read_input_impl(&mut self.ctx.borrow_mut(), offset, target.len() as u32)
                .unwrap();
        target.copy_from_slice(&result);
    }

    fn input_size(&self) -> u32 {
        syscall_input_size_impl(&self.ctx.borrow())
    }

    fn write(&self, value: &[u8]) {
        syscall_write_output_impl(&mut self.ctx.borrow_mut(), value)
    }

    fn output_size(&self) -> u32 {
        syscall_output_size_impl(&self.ctx.borrow())
    }

    fn read_output(&self, target: &mut [u8], offset: u32) {
        let result =
            syscall_read_output_impl(&mut self.ctx.borrow_mut(), offset, target.len() as u32)
                .unwrap();
        target.copy_from_slice(&result);
    }

    fn exec(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let (fuel_consumed, fuel_refunded, exit_code) = syscall_exec_impl(
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
        let (fuel_consumed, fuel_refunded, exit_code) = syscall_resume_impl(
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

    fn forward_output(&self, offset: u32, len: u32) {
        syscall_forward_output_impl(&mut self.ctx.borrow_mut(), offset, len).unwrap_exit_code()
    }

    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        syscall_charge_fuel_manually_impl(&mut self.ctx.borrow_mut(), fuel_consumed, fuel_refunded)
            .unwrap()
    }

    #[inline(always)]
    fn fuel(&self) -> u64 {
        syscall_fuel_impl(&self.ctx.borrow())
    }

    fn debug_log(message: &str) {
        syscall_debug_log_impl(message.as_bytes())
    }

    fn charge_fuel(&self, fuel_consumed: u64) {
        syscall_charge_fuel_impl(&mut self.ctx.borrow_mut(), fuel_consumed).unwrap();
    }

    fn enter_unconstrained(&self) {
        syscall_enter_leave_unconstrained_impl(&mut self.ctx.borrow_mut());
    }

    fn exit_unconstrained(&self) {
        syscall_enter_leave_unconstrained_impl(&mut self.ctx.borrow_mut());
    }

    fn write_fd(&self, fd: u32, slice: &[u8]) {
        syscall_write_fd_impl(&mut self.ctx.borrow_mut(), fd, slice).unwrap_exit_code();
    }

    fn return_data(&self) -> Bytes {
        let ctx = self.ctx.borrow();
        ctx.execution_result.return_data.clone().into()
    }
}
