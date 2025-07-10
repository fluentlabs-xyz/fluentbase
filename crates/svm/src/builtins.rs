use crate::{
    alloc::string::ToString,
    common::{HasherImpl, Keccak256Hasher, Sha256Hasher},
    context::InvokeContext,
    declare_builtin_function,
    error::Error,
    helpers::SyscallError,
    loaders::syscalls::cpi::cpi_common,
    mem_ops::{
        is_nonoverlapping,
        memmove,
        memset_non_contiguous,
        translate_and_check_program_address_inputs,
        translate_slice,
        translate_slice_mut,
        translate_string_and_do,
        translate_type_mut,
    },
    word_size::slice::SliceFatPtr64,
};
use alloc::{boxed::Box, vec::Vec};
use core::str::from_utf8;
use fluentbase_sdk::SharedAPI;
use solana_feature_set;
use solana_pubkey::{Pubkey, PUBKEY_BYTES};
use solana_rbpf::{
    error::EbpfError,
    memory_region::{AccessType, MemoryMapping},
    program::{BuiltinFunction, FunctionRegistry},
};

pub fn register_builtins<SDK: SharedAPI>(
    function_registry: &mut FunctionRegistry<BuiltinFunction<InvokeContext<SDK>>>,
) {
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
    // function_registry
    //     .register_function_hashed("sol_memcmp_", SyscallMemcmp::vm)
    //     .unwrap();
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
    //     .register_function_hashed("sol_poseidon", SyscallHash::vm::<SDK, PoseidonHasher<SDK>>).unwrap();
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
        if !is_nonoverlapping(src_addr, n, dst_addr, n) {
            return Err(SyscallError::CopyOverlapping.into());
        }

        // host addresses can overlap so we always invoke memmove
        memmove(invoke_context, dst_addr, src_addr, n, memory_mapping)
    }
);

// TODO
// declare_builtin_function!(
//     /// memcmp
//     SyscallMemcmp<SDK: SharedAPI>,
//     fn rust(
//         invoke_context: &mut InvokeContext<SDK>,
//         s1_addr: u64,
//         s2_addr: u64,
//         n: u64,
//         cmp_result_addr: u64,
//         _arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         mem_op_consume(invoke_context, n)?;
//
//         if invoke_context
//             .environment_config.feature_set
//             .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id())
//         {
//             let cmp_result = translate_type_mut::<i32>(
//                 memory_mapping,
//                 cmp_result_addr,
//                 invoke_context.get_check_aligned(),
//             )?;
//             *cmp_result = memcmp_non_contiguous(s1_addr, s2_addr, n, memory_mapping)?;
//         } else {
//             let s1 = translate_slice::<u8>(
//                 memory_mapping,
//                 s1_addr,
//                 n,
//                 invoke_context.get_check_aligned(),
//             )?;
//             let s2 = translate_slice::<u8>(
//                 memory_mapping,
//                 s2_addr,
//                 n,
//                 invoke_context.get_check_aligned(),
//             )?;
//             let cmp_result = translate_type_mut::<i32>(
//                 memory_mapping,
//                 cmp_result_addr,
//                 invoke_context.get_check_aligned(),
//             )?;
//
//             debug_assert_eq!(s1.len(), n);
//             debug_assert_eq!(s2.len(), n);
//             // Safety:
//             // memcmp is marked unsafe since it assumes that the inputs are at least
//             // `n` bytes long. `s1` and `s2` are guaranteed to be exactly `n` bytes
//             // long because `translate_slice` would have failed otherwise.
//             *cmp_result = unsafe { memcmp(s1.as_slice(), s2.as_slice(), n as usize) };
//         }
//
//         Ok(0)
//     }
// );

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
        let mut s = translate_slice_mut::<u8>(
            memory_mapping,
            dst_addr,
            n,
            invoke_context.get_check_aligned(),

        )?;
        s.fill(&(c as u8));
        Ok(0)
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
            #[allow(unused_variables)]
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

        translate_string_and_do(
            memory_mapping,
            addr,
            len,
            invoke_context.get_check_aligned(),
            #[allow(unused_variables)]
            &mut |string: &str| {
                #[cfg(test)]
                println!("SyscallLog: {}", string);
                #[cfg(target_arch = "wasm32")]
                debug_log!("SyscallLog: {}", string);
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
        let mut hash_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            size_of::<H::Output>() as u64,
            invoke_context.get_check_aligned(),

        )?;
        let mut hasher = H::create_hasher();
        if vals_len > 0 {
            let vals = translate_slice::<SliceFatPtr64<u8>>(
                memory_mapping,
                vals_addr,
                vals_len,
                invoke_context.get_check_aligned(),

            )?;
            for val in vals.iter() {
                let bytes = val.as_ref().to_vec_cloned();
                hasher.hash(&bytes);
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
//             return Err(SyscallError::InvalidLength.into());
//         }
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
//             return Err(SyscallError::InvalidLength.into());
//         }
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
        let seeds_as_slice = seeds.iter().map(|v| v.as_slice()).collect::<Vec<_>>();
        let new_address = Pubkey::create_program_address(&seeds_as_slice, program_id)?;
        let mut address = translate_slice_mut::<u8>(
            memory_mapping,
            address_addr,
            PUBKEY_BYTES as u64,
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
        let result = translate_and_check_program_address_inputs(
            seeds_addr,
            seeds_len,
            program_id_addr,
            memory_mapping,
            invoke_context.get_check_aligned(),

        );
        let (seeds, program_id) = result?;

        let mut bump_seed = [u8::MAX];
        for _i in 0..u8::MAX {
            {
                let mut seeds_with_bump = seeds.clone();
                seeds_with_bump.push(bump_seed.to_vec());
                let seeds_with_bump_slice = seeds_with_bump.iter().map(|v| v.as_slice()).collect::<Vec<&[u8]>>();

                let new_address = Pubkey::create_program_address(
                    &seeds_with_bump_slice,
                    program_id
                );
                if let Ok(new_address) =
                    new_address
                {
                    let bump_seed_ref = translate_type_mut::<u8>(
                        memory_mapping,
                        bump_seed_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let mut address = translate_slice_mut::<u8>(
                        memory_mapping,
                        address_addr,
                        size_of::<Pubkey>() as u64,
                        invoke_context.get_check_aligned(),

                    )?;
                    // TODO recheck
                    if !is_nonoverlapping(
                        bump_seed_ref as *const _ as usize,
                        size_of_val(bump_seed_ref),
                        // TODO recheck
                        address.first_item_addr().inner() as usize,
                        size_of::<Pubkey>(),
                    ) {
                        return Err(SyscallError::CopyOverlapping.into());
                    }
                    *bump_seed_ref = bump_seed[0];
                    address.copy_from_slice(new_address.as_ref());
                    return Ok(0);
                }
            }
            bump_seed[0] = bump_seed[0].saturating_sub(1);
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
