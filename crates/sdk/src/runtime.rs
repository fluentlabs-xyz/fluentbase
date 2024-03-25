use crate::{evm::B256, LowLevelAPI, LowLevelSDK};
use alloc::rc::Rc;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_runtime::{
    instruction::{
        crypto_ecrecover::CryptoEcrecover,
        crypto_keccak256::CryptoKeccak256,
        crypto_poseidon::CryptoPoseidon,
        crypto_poseidon2::CryptoPoseidon2,
        jzkt_checkpoint::JzktCheckpoint,
        jzkt_commit::JzktCommit,
        jzkt_compute_root::JzktComputeRoot,
        jzkt_emit_log::JzktEmitLog,
        jzkt_get::JzktGet,
        jzkt_load::JzktLoad,
        jzkt_open::JzktOpen,
        jzkt_preimage_copy::JzktPreimageCopy,
        jzkt_preimage_size::JzktPreimageSize,
        jzkt_remove::JzktRemove,
        jzkt_rollback::JzktRollback,
        jzkt_store::JzktStore,
        jzkt_update::JzktUpdate,
        jzkt_update_preimage::JzktUpdatePreimage,
        sys_exec::SysExec,
        sys_exec_hash::SysExecHash,
        sys_forward_output::SysForwardOutput,
        sys_halt::SysHalt,
        sys_input_size::SysInputSize,
        sys_output_size::SysOutputSize,
        sys_read::SysRead,
        sys_read_output::SysReadOutput,
        sys_state::SysState,
        sys_write::SysWrite,
    },
    IJournaledTrie,
    JournalCheckpoint,
    RuntimeContext,
};
use std::{cell::RefCell, ptr};

thread_local! {
    pub static CONTEXT: std::cell::Cell<RuntimeContext<'static, ()>> = std::cell::Cell::new(RuntimeContext::new(&[0u8; 0]));
}

fn with_context<F, R>(func: F) -> R
where
    F: Fn(&RuntimeContext<'static, ()>) -> R,
{
    CONTEXT.with(|ctx| {
        let ctx2 = ctx.take();
        let result = func(&ctx2);
        ctx.set(ctx2);
        result
    })
}

fn with_context_mut<F, R>(func: F) -> R
where
    F: Fn(&mut RuntimeContext<'static, ()>) -> R,
{
    CONTEXT.with(|ctx| {
        let mut ctx2 = ctx.take();
        let result = func(&mut ctx2);
        ctx.set(ctx2);
        result
    })
}

impl LowLevelAPI for LowLevelSDK {
    fn crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8) {
        let result = CryptoKeccak256::fn_impl(unsafe {
            &*ptr::slice_from_raw_parts(data_offset, data_len as usize)
        });
        unsafe {
            ptr::copy(result.as_ptr(), output32_offset, 32);
        }
    }

    fn crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8) {
        let result = CryptoPoseidon::fn_impl(unsafe {
            &*ptr::slice_from_raw_parts(data_offset, data_len as usize)
        });
        unsafe {
            ptr::copy(result.as_ptr(), output32_offset, 32);
        }
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        fd_data: &[u8; 32],
        output: &mut [u8],
    ) {
        match CryptoPoseidon2::fn_impl(fa_data, fb_data, fd_data) {
            Ok(result) => {
                output.copy_from_slice(&result);
            }
            Err(_) => {}
        }
    }

    fn crypto_ecrecover(
        digest32_ptr: *const u8,
        sig64_ptr: *const u8,
        output65_ptr: *mut u8,
        rec_id: u8,
    ) {
        let digest = unsafe { &*ptr::slice_from_raw_parts(digest32_ptr, 32) };
        let sig = unsafe { &*ptr::slice_from_raw_parts(sig64_ptr, 64) };
        let output = unsafe { &mut *ptr::slice_from_raw_parts_mut(output65_ptr, 65) };
        let result = CryptoEcrecover::fn_impl(digest, sig, rec_id as u32).expect("");
        output.copy_from_slice(&result);
    }

    fn sys_read(target: &mut [u8], offset: u32) {
        let result =
            with_context(|ctx| SysRead::fn_impl(ctx, offset, target.len() as u32).unwrap());
        target.copy_from_slice(&result);
    }

    fn sys_input_size() -> u32 {
        with_context(|ctx| SysInputSize::fn_impl(ctx))
    }

    fn sys_write(value: &[u8]) {
        with_context_mut(|ctx| SysWrite::fn_impl(ctx, value))
    }
    fn sys_forward_output(offset: u32, len: u32) {
        with_context_mut(|ctx| SysForwardOutput::fn_impl(ctx, offset, len)).unwrap()
    }

    fn sys_halt(exit_code: i32) {
        with_context_mut(|ctx| SysHalt::fn_impl(ctx, exit_code))
    }

    fn sys_output_size() -> u32 {
        with_context(|ctx| SysOutputSize::fn_impl(ctx))
    }

    fn sys_read_output(target: *mut u8, offset: u32, length: u32) {
        let result = with_context(|ctx| SysReadOutput::fn_impl(ctx, offset, length).unwrap());
        unsafe { ptr::copy(result.as_ptr(), target, length as usize) }
    }

    fn sys_exec(
        code_offset: *const u8,
        code_len: u32,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *const u32,
        state: u32,
    ) -> i32 {
        let bytecode =
            unsafe { &*ptr::slice_from_raw_parts(code_offset, code_len as usize) }.to_vec();
        let input =
            unsafe { &*ptr::slice_from_raw_parts(input_offset, input_len as usize) }.to_vec();
        let fuel = LittleEndian::read_u32(unsafe {
            &*ptr::slice_from_raw_parts(fuel_offset as *const u8, 4)
        });
        match with_context_mut(move |ctx| {
            SysExec::fn_impl(
                ctx,
                bytecode.clone(),
                input.clone(),
                return_len,
                fuel,
                state,
            )
        }) {
            Ok((result, remaining_fuel)) => {
                if return_len > 0 {
                    unsafe { ptr::copy(result.as_ptr(), return_offset, return_len as usize) }
                }
                LittleEndian::write_u32(
                    unsafe { &mut *ptr::slice_from_raw_parts_mut(fuel_offset as *mut u8, 4) },
                    remaining_fuel,
                );
                0
            }
            Err(err) => err,
        }
    }

    fn sys_exec_hash(
        bytecode_hash32_offset: *const u8,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *const u32,
        state: u32,
    ) -> i32 {
        let bytecode_hash32 = unsafe { &*ptr::slice_from_raw_parts(bytecode_hash32_offset, 32) };
        let input =
            unsafe { &*ptr::slice_from_raw_parts(input_offset, input_len as usize) }.to_vec();
        let fuel = LittleEndian::read_u32(unsafe {
            &*ptr::slice_from_raw_parts(fuel_offset as *const u8, 4)
        });
        match with_context_mut(move |ctx| {
            SysExecHash::fn_impl(
                ctx,
                bytecode_hash32.try_into().unwrap(),
                input.clone(),
                return_len,
                fuel,
                state,
            )
        }) {
            Ok((result, remaining_fuel)) => {
                if return_len > 0 {
                    unsafe { ptr::copy(result.as_ptr(), return_offset, return_len as usize) }
                }
                LittleEndian::write_u32(
                    unsafe { &mut *ptr::slice_from_raw_parts_mut(fuel_offset as *mut u8, 4) },
                    remaining_fuel,
                );
                0
            }
            Err(err) => err,
        }
    }

    fn sys_state() -> u32 {
        with_context(|ctx| SysState::fn_impl(ctx))
    }

    fn jzkt_open(root32_ptr: *const u8) {
        let root = unsafe { &*ptr::slice_from_raw_parts(root32_ptr, 32) };
        with_context_mut(|ctx| JzktOpen::fn_impl(ctx, root).unwrap());
    }
    fn jzkt_checkpoint() -> u64 {
        let result = with_context_mut(|ctx| JzktCheckpoint::fn_impl(ctx).unwrap());
        result.to_u64()
    }
    fn jzkt_get(key32_offset: *const u8, field: u32, output32_offset: *mut u8) -> bool {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_offset, 32) };
        match with_context_mut(|ctx| JzktGet::fn_impl(ctx, key, field)) {
            Some((output, is_cold)) => {
                unsafe { ptr::copy(output.as_ptr(), output32_offset, 32) }
                is_cold
            }
            None => true,
        }
    }
    fn jzkt_update(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32) {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let values =
            unsafe { &*ptr::slice_from_raw_parts(vals32_ptr, vals32_len as usize / 32) }.to_vec();
        with_context_mut(|ctx| JzktUpdate::fn_impl(ctx, key, flags, values.clone()).unwrap());
    }
    fn jzkt_update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let preimage = unsafe { &*ptr::slice_from_raw_parts(preimage_ptr, preimage_len as usize) };
        with_context_mut(|ctx| JzktUpdatePreimage::fn_impl(ctx, key, field, preimage).unwrap())
    }
    fn jzkt_remove(key32_ptr: *const u8) {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        with_context_mut(|ctx| JzktRemove::fn_impl(ctx, key).unwrap())
    }
    fn jzkt_compute_root(output32_offset: *mut u8) {
        let root = with_context_mut(|ctx| JzktComputeRoot::fn_impl(ctx));
        unsafe { ptr::copy(root.as_ptr(), output32_offset, 32) }
    }
    fn jzkt_emit_log(
        key32_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    ) {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let topics = unsafe { &*ptr::slice_from_raw_parts(topics32s_ptr, topics32s_len as usize) }
            .iter()
            .map(|v| B256::new(*v))
            .collect::<Vec<_>>();
        let data = unsafe { &*ptr::slice_from_raw_parts(data_ptr, data_len as usize) };
        with_context_mut(|ctx| JzktEmitLog::fn_impl(ctx, key, &topics, data));
    }
    fn jzkt_commit(root32_offset: *mut u8) {
        let root = with_context_mut(|ctx| JzktCommit::fn_impl(ctx).unwrap());
        unsafe { ptr::copy(root.as_ptr(), root32_offset, 32) }
    }
    fn jzkt_rollback(checkpoint: u64) {
        with_context_mut(|ctx| JzktRollback::fn_impl(ctx, JournalCheckpoint::from_u64(checkpoint)));
    }
    fn jzkt_store(slot32_ptr: *const u8, value32_ptr: *const u8) {
        let slot: [u8; 32] = unsafe { &*ptr::slice_from_raw_parts(slot32_ptr, 32) }
            .try_into()
            .unwrap();
        let value: [u8; 32] = unsafe { &*ptr::slice_from_raw_parts(value32_ptr, 32) }
            .try_into()
            .unwrap();
        with_context_mut(|ctx| JzktStore::fn_impl(ctx, &slot, &value).unwrap())
    }
    fn jzkt_load(slot32_ptr: *const u8, value32_ptr: *mut u8) -> i32 {
        let slot: [u8; 32] = unsafe { &*ptr::slice_from_raw_parts(slot32_ptr, 32) }
            .try_into()
            .unwrap();
        match with_context_mut(|ctx| JzktLoad::fn_impl(ctx, &slot).unwrap()) {
            Some((value, is_cold)) => {
                unsafe { ptr::copy(value.as_ptr(), value32_ptr, 32) }
                is_cold as i32
            }
            None => -1,
        }
    }
    fn jzkt_preimage_size(key32_ptr: *const u8) -> u32 {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        return with_context_mut(|ctx| JzktPreimageSize::fn_impl(ctx, key).unwrap());
    }
    fn jzkt_preimage_copy(key32_ptr: *const u8, preimage_ptr: *mut u8) {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let preimage_copy = with_context_mut(|ctx| JzktPreimageCopy::fn_impl(ctx, key).unwrap());
        let dest =
            unsafe { &mut *ptr::slice_from_raw_parts_mut(preimage_ptr, preimage_copy.len()) };
        dest.copy_from_slice(&preimage_copy);
    }
}

impl LowLevelSDK {
    pub fn with_test_input(input: Vec<u8>) {
        CONTEXT.with(|ctx| {
            let mut ctx2 = ctx.take();
            ctx2.with_input(input);
            ctx.set(ctx2);
        });
    }

    pub fn get_test_output() -> Vec<u8> {
        CONTEXT.with(|ctx| {
            let mut ctx2 = ctx.take();
            let result = ctx2.output().clone();
            ctx2.clean_output();
            ctx.set(ctx2);
            result
        })
    }

    pub fn with_test_state(state: u32) {
        CONTEXT.with(|ctx| {
            let mut ctx2 = ctx.take();
            ctx2.with_state(state);
            ctx.set(ctx2);
        });
    }

    pub fn with_jzkt(v: Rc<RefCell<dyn IJournaledTrie>>) {
        CONTEXT.with(|ctx| {
            let mut ctx2 = ctx.take();
            ctx2.with_jzkt(v);
            ctx.set(ctx2);
        });
    }
}
