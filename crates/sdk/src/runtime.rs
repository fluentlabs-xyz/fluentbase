use crate::{evm::B256, LowLevelAPI, LowLevelSDK};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_runtime::types::InMemoryTrieDb;
use fluentbase_runtime::zktrie::ZkTrieStateDb;
use fluentbase_runtime::{
    instruction::{
        crypto_ecrecover::CryptoEcrecover, crypto_keccak256::CryptoKeccak256,
        crypto_poseidon::CryptoPoseidon, crypto_poseidon2::CryptoPoseidon2,
        jzkt_checkpoint::JzktCheckpoint, jzkt_commit::JzktCommit,
        jzkt_compute_root::JzktComputeRoot, jzkt_emit_log::JzktEmitLog, jzkt_get::JzktGet,
        jzkt_open::JzktOpen, jzkt_preimage_copy::JzktPreimageCopy,
        jzkt_preimage_size::JzktPreimageSize, jzkt_remove::JzktRemove, jzkt_rollback::JzktRollback,
        jzkt_update::JzktUpdate, jzkt_update_preimage::JzktUpdatePreimage,
        sys_exec_hash::SysExecHash, sys_forward_output::SysForwardOutput, sys_halt::SysHalt,
        sys_input_size::SysInputSize, sys_output_size::SysOutputSize, sys_read::SysRead,
        sys_read_output::SysReadOutput, sys_state::SysState, sys_write::SysWrite,
    },
    DefaultEmptyRuntimeDatabase, RuntimeContext,
};
use fluentbase_types::{Address, Bytes, JournalCheckpoint};
use std::ptr;

type Context = RuntimeContext<DefaultEmptyRuntimeDatabase>;

thread_local! {
    pub static CONTEXT: std::cell::Cell<Context> = std::cell::Cell::new(Context::new(&[0u8; 0])
        .with_jzkt(DefaultEmptyRuntimeDatabase::new(ZkTrieStateDb::new_empty(InMemoryTrieDb::default()))));
}

fn with_context<F, R>(func: F) -> R
where
    F: Fn(&Context) -> R,
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
    F: Fn(&mut Context) -> R,
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
        fa32_ptr: *const u8,
        fb32_ptr: *const u8,
        fd32_ptr: *const u8,
        output32_ptr: *mut u8,
    ) {
        let fa32 = unsafe { &*ptr::slice_from_raw_parts(fa32_ptr, 32) };
        let fb32 = unsafe { &*ptr::slice_from_raw_parts(fb32_ptr, 32) };
        let fd32 = unsafe { &*ptr::slice_from_raw_parts(fd32_ptr, 32) };
        match CryptoPoseidon2::fn_impl(fa32, fb32, fd32) {
            Ok(result) => unsafe {
                ptr::copy(result.as_ptr(), output32_ptr, result.len());
            },
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

    fn sys_state() -> u32 {
        with_context(|ctx| SysState::fn_impl(ctx))
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
        with_context_mut(move |ctx| {
            match SysExecHash::fn_impl(
                ctx,
                bytecode_hash32.try_into().unwrap(),
                input.clone(),
                return_len,
                fuel as u64,
                state,
            ) {
                Ok(remaining_fuel) => {
                    if return_len > 0 {
                        let return_data = ctx.return_data();
                        unsafe {
                            ptr::copy(return_data.as_ptr(), return_offset, return_len as usize)
                        }
                    }
                    LittleEndian::write_u32(
                        unsafe { &mut *ptr::slice_from_raw_parts_mut(fuel_offset as *mut u8, 4) },
                        remaining_fuel as u32,
                    );
                    0
                }
                Err(err) => err,
            }
        })
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
        address20_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    ) {
        with_context_mut(|ctx| {
            let key = unsafe { &*ptr::slice_from_raw_parts(address20_ptr, 20) };
            let topics =
                unsafe { &*ptr::slice_from_raw_parts(topics32s_ptr, topics32s_len as usize) }
                    .iter()
                    .map(|v| B256::new(*v))
                    .collect::<Vec<_>>();
            let data = unsafe { &*ptr::slice_from_raw_parts(data_ptr, data_len as usize) };
            JzktEmitLog::fn_impl(
                ctx,
                Address::from_slice(key),
                topics,
                Bytes::copy_from_slice(data),
            )
        });
    }
    fn jzkt_commit(root32_offset: *mut u8) {
        let root = with_context_mut(|ctx| JzktCommit::fn_impl(ctx).unwrap());
        unsafe { ptr::copy(root.as_ptr(), root32_offset, 32) }
    }
    fn jzkt_rollback(checkpoint: u64) {
        with_context_mut(|ctx| JzktRollback::fn_impl(ctx, JournalCheckpoint::from_u64(checkpoint)));
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
        with_context_mut(|ctx| {
            ctx.change_input(input.clone());
        });
    }

    pub fn get_test_output() -> Vec<u8> {
        with_context_mut(|ctx| {
            let output = ctx.output().clone();
            ctx.clean_output();
            output
        })
    }

    pub fn with_default_jzkt() -> DefaultEmptyRuntimeDatabase {
        with_context_mut(|ctx| ctx.jzkt().clone())
    }
}
