use crate::{
    Account,
    LowLevelSDK,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_FIELDS_COUNT,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
};
use byteorder::{ByteOrder, LittleEndian};
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
        context_call::SyscallContextCall,
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
    Address,
    Bytes,
    ExitCode,
    JournalCheckpoint,
    SharedAPI,
    SovereignAPI,
    B256,
    KECCAK_EMPTY,
    POSEIDON_EMPTY,
};
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

impl SharedAPI for LowLevelSDK {
    fn keccak256(data_ptr: *const u8, data_len: u32, output32_ptr: *mut u8) {
        let result = SyscallKeccak256::fn_impl(unsafe {
            &*ptr::slice_from_raw_parts(data_ptr, data_len as usize)
        });
        unsafe {
            ptr::copy(result.as_ptr(), output32_ptr, 32);
        }
    }

    fn poseidon(data_ptr: *const u8, data_len: u32, output32_ptr: *mut u8) {
        let result = SyscallPoseidon::fn_impl(unsafe {
            &*ptr::slice_from_raw_parts(data_ptr, data_len as usize)
        });
        unsafe {
            ptr::copy(result.as_ptr(), output32_ptr, 32);
        }
    }

    fn poseidon_hash(
        fa32_ptr: *const u8,
        fb32_ptr: *const u8,
        fd32_ptr: *const u8,
        output32_ptr: *mut u8,
    ) {
        let fa32 = unsafe { &*ptr::slice_from_raw_parts(fa32_ptr, 32) };
        let fb32 = unsafe { &*ptr::slice_from_raw_parts(fb32_ptr, 32) };
        let fd32 = unsafe { &*ptr::slice_from_raw_parts(fd32_ptr, 32) };
        match SyscallPoseidonHash::fn_impl(fa32, fb32, fd32) {
            Ok(result) => unsafe {
                ptr::copy(result.as_ptr(), output32_ptr, result.len());
            },
            Err(_) => {}
        }
    }

    fn ecrecover(digest32_ptr: *const u8, sig64_ptr: *const u8, output65_ptr: *mut u8, rec_id: u8) {
        let digest = unsafe { &*ptr::slice_from_raw_parts(digest32_ptr, 32) };
        let sig = unsafe { &*ptr::slice_from_raw_parts(sig64_ptr, 64) };
        let output = unsafe { &mut *ptr::slice_from_raw_parts_mut(output65_ptr, 65) };
        let result = SyscallEcrecover::fn_impl(digest, sig, rec_id as u32).expect("");
        output.copy_from_slice(&result);
    }

    fn read(target_ptr: *mut u8, target_len: u32, offset: u32) {
        let target =
            unsafe { &mut *ptr::slice_from_raw_parts_mut(target_ptr, target_len as usize) };
        let result =
            with_context(|ctx| SyscallRead::fn_impl(ctx, offset, target.len() as u32).unwrap());
        target.copy_from_slice(&result);
    }

    fn input_size() -> u32 {
        with_context(|ctx| SyscallInputSize::fn_impl(ctx))
    }

    fn write(value_ptr: *const u8, value_len: u32) {
        let value = unsafe { &*ptr::slice_from_raw_parts(value_ptr, value_len as usize) };
        with_context_mut(|ctx| SyscallWrite::fn_impl(ctx, value))
    }

    fn forward_output(offset: u32, len: u32) {
        with_context_mut(|ctx| SyscallForwardOutput::fn_impl(ctx, offset, len)).unwrap()
    }

    fn exit(exit_code: i32) -> ! {
        with_context_mut(|ctx| SyscallExit::fn_impl(ctx, exit_code));
        unreachable!("exit code: {}", exit_code);
    }

    fn output_size() -> u32 {
        with_context(|ctx| SyscallOutputSize::fn_impl(ctx))
    }

    fn read_output(target: *mut u8, offset: u32, length: u32) {
        let result = with_context(|ctx| SyscallReadOutput::fn_impl(ctx, offset, length).unwrap());
        unsafe { ptr::copy(result.as_ptr(), target, length as usize) }
    }

    fn state() -> u32 {
        with_context(|ctx| SyscallState::fn_impl(ctx))
    }

    fn exec(
        bytecode_hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        return_ptr: *mut u8,
        return_len: u32,
        fuel_ptr: *mut u32,
    ) -> i32 {
        with_context_mut(|ctx| {
            let bytecode_hash32 = unsafe { &*ptr::slice_from_raw_parts(bytecode_hash32_ptr, 32) };
            let input =
                unsafe { &*ptr::slice_from_raw_parts(input_ptr, input_len as usize) }.to_vec();
            let fuel = unsafe { *fuel_ptr };
            match SyscallExec::fn_impl(
                ctx,
                bytecode_hash32.try_into().unwrap(),
                input,
                return_len,
                fuel as u64,
            ) {
                Ok(remaining_fuel) => {
                    if return_len > 0 {
                        let return_data = ctx.return_data();
                        unsafe { ptr::copy(return_data.as_ptr(), return_ptr, return_len as usize) }
                    }
                    unsafe {
                        *fuel_ptr = remaining_fuel as u32;
                    }
                    0
                }
                Err(err) => err,
            }
        })
    }

    fn charge_fuel(delta: u64) -> u64 {
        with_context_mut(|ctx| SyscallChargeFuel::fn_impl(ctx, delta))
    }

    fn read_context(target_ptr: *mut u8, offset: u32, length: u32) {
        let context =
            with_context_mut(|ctx| SyscallReadContext::fn_impl(ctx, offset, length).unwrap());
        unsafe {
            ptr::copy(context.as_ptr(), target_ptr, length as usize);
        }
    }
}

impl SovereignAPI for LowLevelSDK {
    fn context_call(
        bytecode_hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        context_ptr: *const u8,
        context_len: u32,
        return_ptr: *mut u8,
        return_len: u32,
        fuel_ptr: *mut u32,
        state: u32,
    ) -> i32 {
        with_context_mut(|ctx| {
            let bytecode_hash32 = unsafe { &*ptr::slice_from_raw_parts(bytecode_hash32_ptr, 32) };
            let input =
                unsafe { &*ptr::slice_from_raw_parts(input_ptr, input_len as usize) }.to_vec();
            let context =
                unsafe { &*ptr::slice_from_raw_parts(context_ptr, context_len as usize) }.to_vec();
            let fuel = unsafe { *fuel_ptr };
            match SyscallContextCall::fn_impl(
                ctx,
                bytecode_hash32.try_into().unwrap(),
                input,
                context,
                return_len,
                fuel as u64,
                state,
            ) {
                Ok(remaining_fuel) => {
                    if return_len > 0 {
                        let return_data = ctx.return_data();
                        unsafe { ptr::copy(return_data.as_ptr(), return_ptr, return_len as usize) }
                    }
                    unsafe {
                        *fuel_ptr = remaining_fuel as u32;
                    }
                    0
                }
                Err(err) => err,
            }
        })
    }

    fn checkpoint() -> u64 {
        let result = with_context_mut(|ctx| SyscallCheckpoint::fn_impl(ctx).unwrap());
        result.to_u64()
    }

    fn get_leaf(key32_ptr: *const u8, field: u32, output32_ptr: *mut u8, committed: bool) -> bool {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        match with_context_mut(|ctx| SyscallGetLeaf::fn_impl(ctx, key, field, committed)) {
            Some((output, is_cold)) => {
                unsafe { ptr::copy(output.as_ptr(), output32_ptr, 32) }
                is_cold
            }
            None => true,
        }
    }

    fn update_leaf(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32) {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let values =
            unsafe { &*ptr::slice_from_raw_parts(vals32_ptr, vals32_len as usize / 32) }.to_vec();
        with_context_mut(|ctx| {
            SyscallUpdateLeaf::fn_impl(ctx, key, flags, values.clone()).unwrap()
        });
    }

    fn update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let preimage = unsafe { &*ptr::slice_from_raw_parts(preimage_ptr, preimage_len as usize) };
        with_context_mut(|ctx| SyscallUpdatePreimage::fn_impl(ctx, key, field, preimage).unwrap())
    }

    fn compute_root(output32_ptr: *mut u8) {
        let root = with_context_mut(|ctx| SyscallComputeRoot::fn_impl(ctx));
        unsafe { ptr::copy(root.as_ptr(), output32_ptr, 32) }
    }

    fn emit_log(
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
            SyscallEmitLog::fn_impl(
                ctx,
                Address::from_slice(key),
                topics,
                Bytes::copy_from_slice(data),
            )
        });
    }

    fn commit(root32_ptr: *mut u8) {
        let root = with_context_mut(|ctx| SyscallCommit::fn_impl(ctx).unwrap());
        unsafe { ptr::copy(root.as_ptr(), root32_ptr, 32) }
    }

    fn rollback(checkpoint: u64) {
        with_context_mut(|ctx| {
            SyscallRollback::fn_impl(ctx, JournalCheckpoint::from_u64(checkpoint))
        });
    }

    fn preimage_size(key32_ptr: *const u8) -> u32 {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        return with_context_mut(|ctx| SyscallPreimageSize::fn_impl(ctx, key).unwrap());
    }

    fn preimage_copy(key32_ptr: *const u8, preimage_ptr: *mut u8) {
        let key = unsafe { &*ptr::slice_from_raw_parts(key32_ptr, 32) };
        let preimage_copy = with_context_mut(|ctx| SyscallPreimageCopy::fn_impl(ctx, key).unwrap());
        let dest =
            unsafe { &mut *ptr::slice_from_raw_parts_mut(preimage_ptr, preimage_copy.len()) };
        dest.copy_from_slice(&preimage_copy);
    }

    fn debug_log(msg_ptr: *const u8, msg_len: u32) {
        let msg = unsafe { &*ptr::slice_from_raw_parts(msg_ptr, msg_len as usize) };
        SyscallDebugLog::fn_impl(msg)
    }
}

impl LowLevelSDK {
    pub fn with_test_input(input: Vec<u8>) {
        with_context_mut(|ctx| {
            ctx.change_input(input.clone());
        });
    }

    pub fn with_test_context(input: Vec<u8>) {
        with_context_mut(|ctx| {
            ctx.change_context(input.clone());
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

    pub fn init_with_devnet_genesis() {
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
            let fields = account2.get_fields();
            let address32 = address.into_word();
            Self::update_leaf(
                address32.as_ptr(),
                JZKT_ACCOUNT_COMPRESSION_FLAGS,
                fields.as_ptr(),
                JZKT_ACCOUNT_FIELDS_COUNT * 32,
            );
            let bytecode = account.code.clone().unwrap_or_default();
            Self::update_preimage(
                address32.as_ptr(),
                JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
                bytecode.as_ptr(),
                bytecode.len() as u32,
            );
            Self::update_preimage(
                address32.as_ptr(),
                JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
                bytecode.as_ptr(),
                bytecode.len() as u32,
            );
        }
    }
}
