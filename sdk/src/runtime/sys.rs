#[allow(dead_code)]
use crate::{LowLevelSDK, LowLevelSysSDK};
#[cfg(test)]
use alloc::vec::Vec;
#[cfg(test)]
use fluentbase_runtime::RuntimeContext;

#[cfg(not(test))]
impl LowLevelSysSDK for LowLevelSDK {
    fn sys_read(_target: &mut [u8], _offset: u32) {
        unreachable!("sys methods are not available in this mode")
    }

    fn sys_input_size() -> u32 {
        unreachable!("sys methods are not available in this mode")
    }

    fn sys_write(_value: &[u8]) {
        unreachable!("sys methods are not available in this mode")
    }

    fn sys_halt(_exit_code: i32) {
        unreachable!("sys methods are not available in this mode")
    }

    fn sys_state() -> u32 {
        unreachable!("sys methods are not available in this mode")
    }
}

#[cfg(test)]
thread_local! {
    pub static CONTEXT: std::cell::Cell<RuntimeContext<'static, ()>> = std::cell::Cell::new(RuntimeContext::new(&[]));
}

#[cfg(test)]
impl LowLevelSDK {
    pub fn with_test_input(input: Vec<u8>) {
        CONTEXT.with(|ctx| {
            let output = ctx.take();
            ctx.set(output.with_input(input));
        });
    }

    pub fn get_test_output() -> Vec<u8> {
        CONTEXT.with(|ctx| {
            let mut output = ctx.take();
            let result = output.output().clone();
            output.clean_output();
            ctx.set(output);
            result
        })
    }

    #[deprecated]
    pub fn with_test_state(state: u32) {
        CONTEXT.with(|ctx| {
            let output = ctx.take();
            ctx.set(output.with_state(state));
        });
    }
}

#[cfg(test)]
impl LowLevelSysSDK for LowLevelSDK {
    fn sys_read(target: &mut [u8], offset: u32) {
        let input = CONTEXT.with(|ctx| {
            let ctx2 = ctx.take();
            let result = ctx2
                .read_input(offset, target.len() as u32)
                .unwrap()
                .to_vec();
            ctx.set(ctx2);
            result.to_vec()
        });
        target.copy_from_slice(&input);
    }

    fn sys_input_size() -> u32 {
        CONTEXT.with(|ctx| {
            let ctx2 = ctx.take();
            let result = ctx2.input_size();
            ctx.set(ctx2);
            result
        })
    }

    fn sys_write(value: &[u8]) {
        CONTEXT.with(|ctx| {
            let mut output = ctx.take();
            output.extend_return_data(value);
            ctx.set(output);
        });
    }

    fn sys_halt(exit_code: i32) {
        CONTEXT.with(|ctx| {
            let mut output = ctx.take();
            output.set_exit_code(exit_code);
            ctx.set(output);
        });
    }

    fn sys_state() -> u32 {
        CONTEXT.with(|ctx| {
            let output = ctx.take();
            let result = output.state();
            ctx.set(output);
            result
        })
    }
}
