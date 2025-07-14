use crate::{
    alloc::string::ToString,
    common::{Blake3Hasher, HasherImpl, Keccak256Hasher, Sha256Hasher},
    context::InvokeContext,
    declare_builtin_function,
    error::{Error, RuntimeError, SvmError},
    helpers::SyscallError,
    loaders::syscalls::cpi::cpi_common,
    mem_ops::{
        is_nonoverlapping,
        memmove,
        translate_and_check_program_address_inputs,
        translate_slice,
        translate_slice_mut,
        translate_string_and_do,
        translate_type,
        translate_type_mut,
    },
    word_size::slice::SliceFatPtr64,
};
use alloc::{boxed::Box, vec::Vec};
use core::str::from_utf8;
use fluentbase_sdk::{debug_log_ext, SharedAPI};
use solana_feature_set::simplify_alt_bn128_syscall_error_codes;
use solana_program_entrypoint::SUCCESS;
use solana_pubkey::{Pubkey, PUBKEY_BYTES};
use solana_rbpf::{
    error::EbpfError,
    memory_region::{AccessType, MemoryMapping},
    program::{BuiltinFunction, FunctionRegistry},
};

/// Maximum size that can be set using [`set_return_data`].
pub const MAX_RETURN_DATA: usize = 1024;

pub fn register_builtins<SDK: SharedAPI>(
    function_registry: &mut FunctionRegistry<BuiltinFunction<InvokeContext<SDK>>>,
) {
    function_registry
        .register_function_hashed("sol_log_", SyscallLog::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_log_64_", SyscallLogU64::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_log_pubkey", SyscallLogPubkey::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_log_data", SyscallLogData::vm)
        .unwrap();

    function_registry
        .register_function_hashed(*b"sol_set_return_data", SyscallSetReturnData::vm)
        .unwrap();
    function_registry
        .register_function_hashed(*b"sol_get_return_data", SyscallGetReturnData::vm)
        .unwrap();

    function_registry
        .register_function_hashed("abort", SyscallAbort::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_panic_", SyscallPanic::vm)
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

    // TODO
    // function_registry
    //     .register_function_hashed("sol_secp256k1_recover", SyscallSecp256k1Recover::vm)
    //     .unwrap();

    // TODO: doesn't call hash computation handle/function, returns default value (zeroes)
    // function_registry
    //     .register_function_hashed("sol_poseidon", SyscallHash::vm::<SDK, PoseidonHasher<SDK>>)
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
    function_registry
        .register_function_hashed("sol_blake3", SyscallHash::vm::<SDK, Blake3Hasher>)
        .unwrap();
    #[cfg(feature = "enable-poseidon")]
    function_registry
        .register_function_hashed("sol_poseidon", SyscallPoseidon::vm)
        .unwrap();
}

macro_rules! log_str_common {
    ($value:expr) => {
        #[allow(unused)]
        {
            #[cfg(all(test, not(target_arch = "wasm32")))]
            println!("builtin log: {}", $value);
            #[cfg(target_arch = "wasm32")]
            {
                #[cfg(not(feature = "use-extended-debug-log"))]
                use fluentbase_sdk::debug_log as log_macro;
                #[cfg(feature = "use-extended-debug-log")]
                use fluentbase_sdk::debug_log_ext as log_macro;
                log_macro!("builtin log: {}", $value);
            }
        }
    };
}

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

// TODO recheck
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

// TODO recheck
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

// );

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
        translate_string_and_do(
            memory_mapping,
            addr,
            len,
            invoke_context.get_check_aligned(),
            #[allow(unused_variables)]
            &mut |string: &str| {
                log_str_common!(string);
                Ok(0)
            },

        )?;
        Ok(0)
    }
);

declare_builtin_function!(
    /// Log 5 64-bit values
    SyscallLogU64<SDK: SharedAPI>,
    fn rust(
        _invoke_context: &mut InvokeContext<SDK>,
        arg1: u64,
        arg2: u64,
        arg3: u64,
        arg4: u64,
        arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use alloc::format;
        log_str_common!(&format!("{arg1:#x}, {arg2:#x}, {arg3:#x}, {arg4:#x}, {arg5:#x}"));
        Ok(0)
    }
);

declare_builtin_function!(
    /// Log a [`Pubkey`] as a base58 string
    SyscallLogPubkey<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        pubkey_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let pubkey = translate_type::<Pubkey>(
            memory_mapping,
            pubkey_addr,
            invoke_context.get_check_aligned(),
            false,
        )?;
        log_str_common!(alloc::format!("{} (hex bytes: {:x?})", pubkey, pubkey.to_bytes()));
        Ok(0)
    }
);

declare_builtin_function!(
    /// Set return data
    SyscallSetReturnData<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        if len > MAX_RETURN_DATA as u64 {
            return Err(SyscallError::ReturnDataTooLarge(len, MAX_RETURN_DATA as u64).into());
        }

        let return_data = if len == 0 {
            Vec::new()
        } else {
            translate_slice::<u8>(
                memory_mapping,
                addr,
                len,
                invoke_context.get_check_aligned(),
            )?
            .to_vec_cloned()
        };
        let transaction_context = &mut invoke_context.transaction_context;
        let program_id = *transaction_context
            .get_current_instruction_context()
            .and_then(|instruction_context| {
                instruction_context.get_last_program_key(transaction_context)
            })?;

        transaction_context.set_return_data(program_id, return_data)?;

        Ok(0)
    }
);

declare_builtin_function!(
    /// Get return data
    SyscallGetReturnData<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        return_data_addr: u64,
        length: u64,
        program_id_addr: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let (program_id, return_data) = invoke_context.transaction_context.get_return_data();
        let length = length.min(return_data.len() as u64);
        if length != 0 {
            let return_data_result = translate_slice_mut::<u8>(
                memory_mapping,
                return_data_addr,
                length,
                invoke_context.get_check_aligned(),
            )?;

            let mut to_slice = return_data_result;
            let from_slice = return_data
                .get(..length as usize)
                .ok_or(SyscallError::InvokeContextBorrowFailed)?;
            if to_slice.len() != from_slice.len() {
                return Err(SyscallError::InvalidLength.into());
            }
            to_slice.copy_from_slice(from_slice);

            let program_id_result = translate_type_mut::<Pubkey>(
                memory_mapping,
                program_id_addr,
                invoke_context.get_check_aligned(),
                false,
            )?;
            log_str_common!(alloc::format!("program_id_result {}", program_id_result));

            if !is_nonoverlapping(
                to_slice.first_item_addr().inner(),
                length,
                program_id_result as *const _ as u64,
                size_of::<Pubkey>() as u64,
            ) {
                return Err(SyscallError::CopyOverlapping.into());
            }

            *program_id_result = *program_id;
        }

        // Return the actual length, rather the length returned
        let return_data_len = return_data.len() as u64;
        log_str_common!(alloc::format!("return_data_len {}", return_data_len));
        Ok(return_data_len)
    }
);

declare_builtin_function!(
    /// Log data handling
    SyscallLogData<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let untranslated_data_fields = translate_slice::<SliceFatPtr64<u8>>(
            memory_mapping,
            addr,
            len,
            invoke_context.get_check_aligned(),
        )?;
        let data_fields = untranslated_data_fields
            .iter()
            .map(|untranslated_data_field| {
                Ok(untranslated_data_field.as_ref().to_vec_cloned())
            })
            .collect::<Result<Vec<_>, SvmError>>()?;

        log_str_common!(alloc::format!("hex fields: {:x?}", data_fields));

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

// TODO recheck
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
            let untranslated_vals = translate_slice::<SliceFatPtr64<u8>>(
                memory_mapping,
                vals_addr,
                vals_len,
                invoke_context.get_check_aligned(),
            )?;
            for untranslated_val in untranslated_vals.iter() {
                let bytes = untranslated_val.as_ref().to_vec_cloned();
                hasher.hash(&bytes);
            }
        }
        let hasher_result = hasher.result();
        let hashing_result = hasher_result.as_ref();
        hash_result.copy_from_slice(hashing_result);
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

#[cfg(feature = "enable-poseidon")]
declare_builtin_function!(
    // Poseidon
    SyscallPoseidon<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        parameters: u64,
        endianness: u64,
        vals_addr: u64,
        vals_len: u64,
        result_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let parameters: solana_poseidon::Parameters = parameters.try_into().map_err(|_| RuntimeError::InvalidConversion)?;
        let endianness: solana_poseidon::Endianness = endianness.try_into().map_err(|_| RuntimeError::InvalidConversion)?;

        if vals_len > 12 {
            debug_log_ext!(
                "Poseidon hashing {} sequences is not supported",
                vals_len,
            );
            return Err(SyscallError::InvalidLength.into());
        }

        let mut hash_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            solana_poseidon::HASH_BYTES as u64,
            invoke_context.get_check_aligned(),
        )?;
        let untranslated_inputs = translate_slice::<SliceFatPtr64<u8>>(
            memory_mapping,
            vals_addr,
            vals_len,
            invoke_context.get_check_aligned(),
        )?;
       let inputs_vec = untranslated_inputs
            .iter()
            .map(|v| {
                Ok(v.as_ref().to_vec_cloned())
            })
            .collect::<Result<Vec<_>, SvmError>>()?;
        let inputs = inputs_vec.iter().map(|v| v.as_slice()).collect::<Vec<_>>();

        let simplify_alt_bn128_syscall_error_codes = invoke_context
            .get_feature_set()
            .is_active(&simplify_alt_bn128_syscall_error_codes::id());

        let hash = match solana_poseidon::hashv(parameters, endianness, inputs.as_slice()) {
            Ok(hash) => hash,
            Err(e) => {
                return if simplify_alt_bn128_syscall_error_codes {
                    Ok(1)
                } else {
                    Ok(e.into())
                };
            }
        };
        hash_result.copy_from_slice(&hash.to_bytes());

        Ok(SUCCESS)
    }
);

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
