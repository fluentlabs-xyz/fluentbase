use crate::{
    alloc::{string::ToString, vec::Vec},
    common::{HasherImpl, Keccak256Hasher, Sha256Hasher},
    context::InvokeContext,
    declare_builtin_function,
    error::Error,
    helpers::SyscallError,
    loaders::{bpf_loader_upgradeable, syscals::cpi::cpi_common},
    mem_ops_original::{
        is_nonoverlapping,
        memcmp,
        memcmp_non_contiguous,
        memmove,
        memset_non_contiguous,
        translate,
        translate_and_check_program_address_inputs,
        translate_slice,
        translate_slice_mut,
        translate_string_and_do,
        translate_type_mut,
    },
    // word_size_mismatch::fat_ptr_repr::{SliceFatPtr64, SLICE_FAT_PTR64_BYTE_SIZE},
};
use alloc::boxed::Box;
use core::str::from_utf8;
use fluentbase_sdk::{debug_log, SharedAPI};
use solana_account_info::AccountInfo;
use solana_feature_set;
use solana_pubkey::Pubkey;
use solana_rbpf::{
    error::EbpfError,
    memory_region::{AccessType, MemoryMapping},
    program::{BuiltinFunction, FunctionRegistry},
};

pub fn register_builtins<SDK: SharedAPI>(
    function_registry: &mut FunctionRegistry<BuiltinFunction<InvokeContext<SDK>>>,
) {
    function_registry
        .register_function_hashed(
            "solana_bpf_loader_deprecated_program",
            bpf_loader_upgradeable::Entrypoint::vm,
        )
        .unwrap();
    function_registry
        .register_function_hashed(
            "solana_bpf_loader_program",
            bpf_loader_upgradeable::Entrypoint::vm,
        )
        .unwrap();
    function_registry
        .register_function_hashed(
            "solana_bpf_loader_upgradeable_program",
            bpf_loader_upgradeable::Entrypoint::vm,
        )
        .unwrap();

    function_registry
        .register_function_hashed("sol_log_", SyscallLog::vm)
        .unwrap();
    function_registry
        .register_function_hashed("abort", SyscallAbort::vm)
        .unwrap();
    function_registry
        .register_function_hashed(
            *b"sol_create_program_address",
            SyscallCreateProgramAddress::vm,
        )
        .unwrap();
    function_registry
        .register_function_hashed(
            *b"sol_try_find_program_address",
            SyscallTryFindProgramAddress::vm,
        )
        .unwrap();

    function_registry
        .register_function_hashed("sol_memcpy_", SyscallMemcpy::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_memmove_", SyscallMemmove::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_memcmp_", SyscallMemcmp::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_memset_", SyscallMemset::vm)
        .unwrap();

    function_registry
        .register_function_hashed("sol_invoke_signed_rust", SyscallInvokeSignedRust::vm)
        .unwrap();

    // function_registry
    //     .register_function_hashed("sol_secp256k1_recover", SyscallSecp256k1Recover::vm)
    //     .unwrap();

    // TODO: doesn't call hash computation handle/function, returns default value (zeroes)
    // function_registry
    //     .register_function_hashed("sol_poseidon", SyscallHash::vm::<SDK,
    // PoseidonHasher<SDK>>)     .unwrap();
    // function_registry
    //     .register_function_hashed("sol_poseidon", SyscallPoseidonSDK::vm)
    //     .unwrap();
    function_registry
        .register_function_hashed("sol_sha256", SyscallHash::vm::<SDK, Sha256Hasher>)
        .unwrap();
    function_registry
        .register_function_hashed(
            "sol_keccak256",
            SyscallHash::vm::<SDK, Keccak256Hasher<SDK>>,
        )
        .unwrap();
    // function_registry
    //     .register_function_hashed("sol_blake3", SyscallHash::vm::<SDK, Blake3Hasher>)
    //     .unwrap();
}

declare_builtin_function!(
    /// memcpy
    SyscallMemcpy<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        dst_addr: u64,
        src_addr: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // mem_op_consume(invoke_context, n)?;

        if !is_nonoverlapping(src_addr, n, dst_addr, n) {
            return Err(SyscallError::CopyOverlapping.into());
        }

        // host addresses can overlap so we always invoke memmove
        memmove(invoke_context, dst_addr, src_addr, n, memory_mapping)
    }
);

declare_builtin_function!(
    /// memcmp
    SyscallMemcmp<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        s1_addr: u64,
        s2_addr: u64,
        n: u64,
        cmp_result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {

        unimplemented!("SyscallMemcmp unimplemented yet");

        // mem_op_consume(invoke_context, n)?;

        // if invoke_context
        //     .environment_config.feature_set
        //     .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id())
        // {
        //     let cmp_result = translate_type_mut::<i32>(
        //         memory_mapping,
        //         cmp_result_addr,
        //         invoke_context.get_check_aligned(),
        //     )?;
        //     *cmp_result = memcmp_non_contiguous(s1_addr, s2_addr, n, memory_mapping)?;
        // } else {
        //     let s1 = translate_slice::<u8>(
        //         memory_mapping,
        //         s1_addr,
        //         n,
        //         invoke_context.get_check_aligned(),
        //     )?;
        //     let s2 = translate_slice::<u8>(
        //         memory_mapping,
        //         s2_addr,
        //         n,
        //         invoke_context.get_check_aligned(),
        //     )?;
        //     let cmp_result = translate_type_mut::<i32>(
        //         memory_mapping,
        //         cmp_result_addr,
        //         invoke_context.get_check_aligned(),
        //     )?;
        //
        //     debug_assert_eq!(s1.len(), n);
        //     debug_assert_eq!(s2.len(), n);
        //     // Safety:
        //     // memcmp is marked unsafe since it assumes that the inputs are at least
        //     // `n` bytes long. `s1` and `s2` are guaranteed to be exactly `n` bytes
        //     // long because `translate_slice` would have failed otherwise.
        //     *cmp_result = unsafe { memcmp(s1.as_slice(), s2.as_slice(), n as usize) };
        // }

        Ok(0)
    }
);

declare_builtin_function!(
    /// memmove
    SyscallMemmove<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        dst_addr: u64,
        src_addr: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {

        // mem_op_consume(invoke_context, n)?;

        memmove(invoke_context, dst_addr, src_addr, n, memory_mapping)
    }
);

declare_builtin_function!(
    /// memset
    SyscallMemset<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        dst_addr: u64,
        c: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {

        // mem_op_consume(invoke_context, n)?;

        if invoke_context
            .environment_config.feature_set
            .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id())
        {
            memset_non_contiguous(dst_addr, c as u8, n, memory_mapping)
        } else {
            let mut s = translate_slice_mut::<u8>(
                memory_mapping,
                dst_addr,
                n,
                invoke_context.get_check_aligned(),
            )?;
            s.fill(c as u8);
            Ok(0)
        }
    }
);

declare_builtin_function!(
    /// Prints a NULL-terminated UTF-8 string.
    SyscallString<SDK: SharedAPI>,
    fn rust(
        _invoke_context: &mut InvokeContext<SDK>,
        vm_addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn core::error::Error>> {
        #[cfg(target_arch = "wasm32")]
        use fluentbase_sdk::debug_log;
        let host_addr: Result<u64, EbpfError> =
            memory_mapping.map(AccessType::Load, vm_addr, len).into();
        let host_addr = host_addr?;
        unsafe {
            let c_buf = alloc::slice::from_raw_parts(host_addr as *const u8, len as usize);
            let len = c_buf.iter().position(|c| *c == 0).unwrap_or(len as usize);
            let message = from_utf8(&c_buf[0..len]).unwrap_or("Invalid UTF-8 String");
            #[cfg(test)]
            println!("message={}", message);
            #[cfg(target_arch = "wasm32")]
            debug_log!("message={}", message);
        }
        Ok(0)
    }
);

declare_builtin_function!(
    /// Log a user's info message
    SyscallLog<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        #[cfg(target_arch = "wasm32")]
        use fluentbase_sdk::debug_log;
        // let cost = invoke_context
        //     .get_compute_budget()
        //     .syscall_base_cost
        //     .max(len);
        // consume_compute_meter(invoke_context, cost)?;

        translate_string_and_do(
            memory_mapping,
            addr,
            len,
            invoke_context.get_check_aligned(),
            // true,
            &mut |string: &str| {
                // stable_log::program_log(&invoke_context.get_log_collector(), string);
                #[cfg(test)]
                println!("Log: {}", string);
                #[cfg(target_arch = "wasm32")]
                debug_log!("Log: {}", string);
                Ok(0)
            },
        )?;
        Ok(0)
    }
);

declare_builtin_function!(
    /// Abort syscall functions, called when the SBF program calls `abort()`
    /// LLVM will insert calls to `abort()` if it detects an untenable situation,
    /// `abort()` is not intended to be called explicitly by the program.
    /// Causes the SBF program to be halted immediately
    SyscallAbort<SDK: SharedAPI>,
    fn rust(
        _invoke_context: &mut InvokeContext<SDK>,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        Err(SyscallError::Abort.into())
    }
);

declare_builtin_function!(
    /// Panic syscall function, called when the SBF program calls 'sol_panic_()`
    /// Causes the SBF program to be halted immediately
    SyscallPanic<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        file: u64,
        len: u64,
        line: u64,
        column: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // consume_compute_meter(invoke_context, len)?;

        translate_string_and_do(
            memory_mapping,
            file,
            len,
            invoke_context.get_check_aligned(),
            &mut |string: &str| Err(SyscallError::Panic(string.to_string(), line, column).into()),
        )
    }
);

declare_builtin_function!(
    // Generic Hashing Syscall
    SyscallHash<SDK: SharedAPI, H: HasherImpl>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        vals_addr: u64,
        vals_len: u64,
        result_addr: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {

        // let compute_budget = invoke_context.get_compute_budget();
        // let hash_base_cost = H::get_base_cost(compute_budget);
        // let hash_byte_cost = H::get_byte_cost(compute_budget);
        // let hash_max_slices = H::get_max_slices(compute_budget);
        // if hash_max_slices < vals_len {
        //     ic_msg!(
        //         invoke_context,
        //         "{} Hashing {} sequences in one syscall is over the limit {}",
        //         H::NAME,
        //         vals_len,
        //         hash_max_slices,
        //     );
        //     return Err(SyscallError::TooManySlices.into());
        // }

        // consume_compute_meter(invoke_context, hash_base_cost)?;

        let mut hash_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            size_of::<H::Output>() as u64,
            invoke_context.get_check_aligned(),
        )?;
        let mut hasher = H::create_hasher();
        if vals_len > 0 {
            let vals = translate_slice::<&[u8]>(
                memory_mapping,
                vals_addr,
                vals_len,
                invoke_context.get_check_aligned(),
            )?;
            for val in vals.iter() {
                // TODO
                let bytes = translate_slice::<u8>(
                    memory_mapping,
                    val.as_ptr() as u64,
                    val.len() as u64,
                    invoke_context.get_check_aligned(),
                )?;
                hasher.hash(&bytes.to_vec());
            }
        }
        hash_result.copy_from_slice(hasher.result().as_ref());
        Ok(0)
    }
);

// declare_builtin_function!(
//     /// secp256k1_recover
//     SyscallSecp256k1Recover<SDK: SharedAPI>,
//     fn rust(
//         invoke_context: &mut InvokeContext<SDK>,
//         hash_addr: u64,
//         recovery_id_val: u64,
//         signature_addr: u64,
//         result_addr: u64,
//         _arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//
//         // let cost = invoke_context.get_compute_budget().secp256k1_recover_cost;
//         // consume_compute_meter(invoke_context, cost)?;
//
//         let hash = translate_slice::<u8>(
//             memory_mapping,
//             hash_addr,
//             keccak::HASH_BYTES as u64,
//             invoke_context.get_check_aligned(),
//         )?;
//         let signature = translate_slice::<u8>(
//             memory_mapping,
//             signature_addr,
//             SECP256K1_SIGNATURE_LENGTH as u64,
//             invoke_context.get_check_aligned(),
//         )?;
//         let secp256k1_recover_result = translate_slice_mut::<u8>(
//             memory_mapping,
//             result_addr,
//             SECP256K1_PUBLIC_KEY_LENGTH as u64,
//             invoke_context.get_check_aligned(),
//         )?;
//
//         let Ok(message) = libsecp256k1::Message::parse_slice(hash) else {
//             return Ok(Secp256k1RecoverError::InvalidHash.into());
//         };
//         let Ok(adjusted_recover_id_val) = recovery_id_val.try_into() else {
//             return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
//         };
//         let Ok(recovery_id) =libsecp256k1::RecoveryId::parse(adjusted_recover_id_val) else {
//             return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
//         };
//         let Ok(signature) = libsecp256k1::Signature::parse_standard_slice(signature) else {
//             return Ok(Secp256k1RecoverError::InvalidSignature.into());
//         };
//
//         let public_key = match libsecp256k1::recover(&message, &signature, &recovery_id) {
//             Ok(key) => key.serialize(),
//             Err(_) => {
//                 return Ok(Secp256k1RecoverError::InvalidSignature.into());
//             }
//         };
//
//         secp256k1_recover_result.copy_from_slice(&public_key[1..65]);
//         Ok(SUCCESS)
//     }
// );

// declare_builtin_function!(
//     SyscallPoseidon<SDK: SharedAPI>,
//     fn rust(
//         invoke_context: &mut InvokeContext<SDK>,
//         parameters: u64,
//         endianness: u64,
//         vals_addr: u64,
//         vals_len: u64,
//         result_addr: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//
//         let parameters: solana_poseidon::Parameters = parameters.try_into().map_err(|e| {
//             SyscallError::Abort
//         })?;
//         let endianness: solana_poseidon::Endianness = endianness.try_into().map_err(|e| {
//             SyscallError::Abort
//         })?;
//
//         if vals_len > 12 {
//             // ic_msg!(
//             //     invoke_context,
//             //     "Poseidon hashing {} sequences is not supported",
//             //     vals_len,
//             // );
//             return Err(SyscallError::InvalidLength.into());
//         }
//
//         // let budget = invoke_context.get_compute_budget();
//         // let Some(cost) = budget.poseidon_cost(vals_len) else {
//         //     ic_msg!(
//         //         invoke_context,
//         //         "Overflow while calculating the compute cost"
//         //     );
//         //     return Err(SyscallError::ArithmeticOverflow.into());
//         // };
//         // consume_compute_meter(invoke_context, cost.to_owned())?;
//
//         let hash_result = translate_slice_mut::<u8>(
//             memory_mapping,
//             result_addr,
//             solana_poseidon::HASH_BYTES as u64,
//             invoke_context.get_check_aligned(),
//         )?;
//         let inputs = translate_slice::<&[u8]>(
//             memory_mapping,
//             vals_addr,
//             vals_len,
//             invoke_context.get_check_aligned(),
//         )?;
//         let inputs = inputs
//             .iter()
//             .map(|input| {
//                 translate_slice::<u8>(
//                     memory_mapping,
//                     input.as_ptr() as *const _ as u64,
//                     input.len() as u64,
//                     invoke_context.get_check_aligned(),
//                 )
//             })
//             .collect::<Result<Vec<_>, Error>>()?;
//
//         // let simplify_alt_bn128_syscall_error_codes = invoke_context
//         //     .feature_set
//         //     .is_active(&feature_set::simplify_alt_bn128_syscall_error_codes::id());
//
//         let hash = match solana_poseidon::hashv(parameters, endianness, inputs.as_slice()) {
//             Ok(hash) => hash,
//             Err(e) => {
//                 return /*if simplify_alt_bn128_syscall_error_codes {
//                     Ok(1)
//                 } else {*/
//                     Ok(e.into());
//                 /*};*/
//             }
//         };
//         hash_result.copy_from_slice(&hash.to_bytes());
//
//         Ok(SUCCESS)
//     }
// );

// declare_builtin_function!(
//     SyscallPoseidonSDK<SDK: SharedAPI>,
//     fn rust(
//         invoke_context: &mut InvokeContext<SDK>,
//         parameters: u64,
//         endianness: u64,
//         vals_addr: u64,
//         vals_len: u64,
//         result_addr: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//
//         let parameters: solana_poseidon::Parameters = parameters.try_into().map_err(|e| {
//             SyscallError::Abort
//         })?;
//         let endianness: solana_poseidon::Endianness = endianness.try_into().map_err(|e| {
//             SyscallError::Abort
//         })?;
//
//         if vals_len > 12 {
//             // ic_msg!(
//             //     invoke_context,
//             //     "Poseidon hashing {} sequences is not supported",
//             //     vals_len,
//             // );
//             return Err(SyscallError::InvalidLength.into());
//         }
//
//         // let budget = invoke_context.get_compute_budget();
//         // let Some(cost) = budget.poseidon_cost(vals_len) else {
//         //     ic_msg!(
//         //         invoke_context,
//         //         "Overflow while calculating the compute cost"
//         //     );
//         //     return Err(SyscallError::ArithmeticOverflow.into());
//         // };
//         // consume_compute_meter(invoke_context, cost.to_owned())?;
//
//         let hash_result = translate_slice_mut::<u8>(
//             memory_mapping,
//             result_addr,
//             solana_poseidon::HASH_BYTES as u64,
//             invoke_context.get_check_aligned(),
//         )?;
//         let inputs = translate_slice::<&[u8]>(
//             memory_mapping,
//             vals_addr,
//             vals_len,
//             invoke_context.get_check_aligned(),
//         )?;
//         let inputs = inputs
//             .iter()
//             .map(|input| {
//                 translate_slice::<u8>(
//                     memory_mapping,
//                     input.as_ptr() as *const _ as u64,
//                     input.len() as u64,
//                     invoke_context.get_check_aligned(),
//                 )
//             })
//             .collect::<Result<Vec<_>, Error>>()?;
//
//         // let simplify_alt_bn128_syscall_error_codes = invoke_context
//         //     .feature_set
//         //     .is_active(&feature_set::simplify_alt_bn128_syscall_error_codes::id());
//
//         let mut inputs_vec = Vec::new();
//         for i in inputs {
//             inputs_vec.extend_from_slice(i);
//         }
//         // invoke_context.sdk.deref().;
//         // TODO
//         // let hash = invo::poseidon(&inputs_vec);
//         let hash = [0u8; 32];
//
//         // let hash = match poseidon::hashv(parameters, endianness, inputs.as_slice()) {
//         //     Ok(hash) => hash,
//         //     Err(e) => {
//         //         return /*if simplify_alt_bn128_syscall_error_codes {
//         //             Ok(1)
//         //         } else {*/
//         //             Ok(e.into());
//         //         /*};*/
//         //     }
//         // };
//         hash_result.copy_from_slice(&hash);
//
//         Ok(SUCCESS)
//     }
// );

declare_builtin_function!(
    /// Create a program address
    SyscallCreateProgramAddress<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        seeds_addr: u64,
        seeds_len: u64,
        program_id_addr: u64,
        address_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // let cost = invoke_context
        //     .get_compute_budget()
        //     .create_program_address_units;
        // consume_compute_meter(invoke_context, cost)?;
        debug_log!("in SyscallCreateProgramAddress");

        let (seeds, program_id) = translate_and_check_program_address_inputs(
            seeds_addr,
            seeds_len,
            program_id_addr,
            memory_mapping,
            invoke_context.get_check_aligned(),
        )?;

        // replace smv pubkey with evm create2
        // deployer: &Address, -> program_id  (check first 12 bytes and cut then)
        // salt: &U256, -> seeds
        // init_code_hash: &B256, -> ?
        // let Ok(new_address) = Pubkey::create_program_address(&seeds, program_id) else {
        //     return Ok(1);
        // };
        let new_address = Pubkey::create_program_address(&seeds, program_id)?;
        let mut address = translate_slice_mut::<u8>(
            memory_mapping,
            address_addr,
            32,
            invoke_context.get_check_aligned(),
        )?;
        address.copy_from_slice(new_address.as_ref());
        Ok(0)
    }
);

declare_builtin_function!(
    /// Create a program address
    SyscallTryFindProgramAddress<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        seeds_addr: u64,
        seeds_len: u64,
        program_id_addr: u64,
        address_addr: u64,
        bump_seed_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // let cost = invoke_context
        //     .get_compute_budget()
        //     .create_program_address_units;
        // consume_compute_meter(invoke_context, cost)?;
        debug_log!(
            "in SyscallTryFindProgramAddress: seeds_addr {} seeds_len {} program_id_addr {} invoke_context.get_check_aligned() {} memory_mapping {:?}",
            seeds_addr,
            seeds_len,
            program_id_addr,
            invoke_context.get_check_aligned(),
            memory_mapping,
        );

        // let host_addr = memory_mapping.map(AccessType::Load, seeds_addr, 8).unwrap();
        let word_size = size_of::<usize>();
        let host_addr = translate(memory_mapping, AccessType::Load, seeds_addr, word_size as u64 * seeds_len)?;
        let untranslated_seeds = crate::mem_ops::translate_slice::<&[u8]>(memory_mapping, seeds_addr, seeds_len, true)?;
        // let untranslated_seeds = translate_slice::<&[u8]>(memory_mapping, seeds_addr, seeds_len, true)?;
        let seeds_slice_fat_ptr_data =
                unsafe { core::slice::from_raw_parts(host_addr as *const u8, crate::ptr_size::slice_fat_ptr_v2::SLICE_FAT_PTR64_SIZE_BYTES) };
        let addr_from_fat_ptr_data = u64::from_le_bytes(seeds_slice_fat_ptr_data[..crate::ptr_size::slice_fat_ptr_v2::FAT_PTR64_ELEM_BYTE_SIZE].try_into().unwrap());
        let addr_from_fat_ptr_data_on_host = translate(memory_mapping, AccessType::Load, addr_from_fat_ptr_data, word_size as u64 * seeds_len)?;
        let len_from_fat_ptr_data = u64::from_le_bytes(seeds_slice_fat_ptr_data[crate::ptr_size::slice_fat_ptr_v2::FAT_PTR64_ELEM_BYTE_SIZE..crate::ptr_size::slice_fat_ptr_v2::FAT_PTR64_ELEM_BYTE_SIZE*2].try_into().unwrap());
        let reconstructed_slice = unsafe { core::slice::from_raw_parts(addr_from_fat_ptr_data_on_host as *const u8, len_from_fat_ptr_data as usize) };
        debug_log!(
            "seeds_slice_fat_ptr_data2 (addr:{} host_addr:{}): untranslated_seeds ({}) seeds_slice_fat_ptr_data (addr:{} addr_on_host:{} len:{}) {:x?} reconstructed_slice {:x?}",
            seeds_addr,
            host_addr,
            untranslated_seeds.len(),
            addr_from_fat_ptr_data,
            addr_from_fat_ptr_data_on_host,
            len_from_fat_ptr_data,
            seeds_slice_fat_ptr_data,
            reconstructed_slice,
        );
        for (idx, untranslated_seed) in untranslated_seeds.iter().enumerate() {
            debug_log!(
                "untranslated_seed{} ({}): ",
                idx,
                untranslated_seed.as_ref().len(),
                // untranslated_seed.to_vec()
            );
        }
        let result = translate_and_check_program_address_inputs(
            seeds_addr,
            seeds_len,
            program_id_addr,
            memory_mapping,
            invoke_context.get_check_aligned(),
        );
        if let Err(e) = &result {
            debug_log!("in SyscallTryFindProgramAddress: translate_and_check_program_address_inputs: {:?}", e);
        }
        let (seeds, program_id) = result?;
        debug_log!("in SyscallTryFindProgramAddress: translate_and_check_program_address_inputs: seeds {:x?}", &seeds);

        let mut bump_seed = [u8::MAX];
        for i in 0..u8::MAX {
            {
                debug_log!("in SyscallTryFindProgramAddress: i={}", i);
                let mut seeds_with_bump = seeds.clone();
                seeds_with_bump.push(&bump_seed);

                let new_address = Pubkey::create_program_address(
                    &seeds_with_bump,
                    program_id
                );
                if let Ok(new_address) =
                    // Pubkey::create_program_address(&seeds_with_bump, program_id)
                    new_address
                {
                    let bump_seed_ref = translate_type_mut::<u8>(
                        memory_mapping,
                        bump_seed_addr,
                        invoke_context.get_check_aligned(),
                    )?;
                    let mut address = translate_slice_mut::<u8>(
                        memory_mapping,
                        address_addr,
                        core::mem::size_of::<Pubkey>() as u64,
                        invoke_context.get_check_aligned(),
                    )?;
                    // TODO recheck this check
                    if !is_nonoverlapping(
                        bump_seed_ref as *const _ as usize,
                        core::mem::size_of_val(bump_seed_ref),
                        // TODO recheck
                        address.as_ptr() as usize,
                        core::mem::size_of::<Pubkey>(),
                    ) {
                        return Err(SyscallError::CopyOverlapping.into());
                    }
                    *bump_seed_ref = bump_seed[0];
                    address.copy_from_slice(new_address.as_ref());
                    return Ok(0);
                }
            }
            bump_seed[0] = bump_seed[0].saturating_sub(1);
            // consume_compute_meter(invoke_context, cost)?;
        }
        Ok(1)
    }
);

declare_builtin_function!(
    /// Cross-program invocation called from Rust
    SyscallInvokeSignedRust<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        instruction_addr: u64,
        account_infos_addr: u64,
        account_infos_len: u64,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        debug_log!("in SyscallInvokeSignedRust");
        cpi_common::<SDK, Self>(
            invoke_context,
            instruction_addr,
            account_infos_addr,
            account_infos_len,
            signers_seeds_addr,
            signers_seeds_len,
            memory_mapping,
        )
    }
);

// declare_builtin_function!(
//     /// Create a program address
//     SyscallTryFindProgramAddress,
//     fn rust(
//         invoke_context: &mut InvokeContext,
//         seeds_addr: u64,
//         seeds_len: u64,
//         program_id_addr: u64,
//         address_addr: u64,
//         bump_seed_addr: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         let cost = invoke_context
//             .get_compute_budget()
//             .create_program_address_units;
//         consume_compute_meter(invoke_context, cost)?;
//
//         let (seeds, program_id) = translate_and_check_program_address_inputs(
//             seeds_addr,
//             seeds_len,
//             program_id_addr,
//             memory_mapping,
//             invoke_context.get_check_aligned(),
//         )?;
//
//         let mut bump_seed = [std::u8::MAX];
//         for _ in 0..std::u8::MAX {
//             {
//                 let mut seeds_with_bump = seeds.to_vec();
//                 seeds_with_bump.push(&bump_seed);
//
//                 if let Ok(new_address) =
//                     Pubkey::create_program_address(&seeds_with_bump, program_id)
//                 {
//                     let bump_seed_ref = translate_type_mut::<u8>(
//                         memory_mapping,
//                         bump_seed_addr,
//                         invoke_context.get_check_aligned(),
//                     )?;
//                     let address = translate_slice_mut::<u8>(
//                         memory_mapping,
//                         address_addr,
//                         std::mem::size_of::<Pubkey>() as u64,
//                         invoke_context.get_check_aligned(),
//                     )?;
//                     if !is_nonoverlapping(
//                         bump_seed_ref as *const _ as usize,
//                         std::mem::size_of_val(bump_seed_ref),
//                         address.as_ptr() as usize,
//                         std::mem::size_of::<Pubkey>(),
//                     ) {
//                         return Err(SyscallError::CopyOverlapping.into());
//                     }
//                     *bump_seed_ref = bump_seed[0];
//                     address.copy_from_slice(new_address.as_ref());
//                     return Ok(0);
//                 }
//             }
//             bump_seed[0] = bump_seed[0].saturating_sub(1);
//             consume_compute_meter(invoke_context, cost)?;
//         }
//         Ok(1)
//     }
// );
