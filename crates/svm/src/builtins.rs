#[cfg(feature = "enable-solana-original-builtins")]
use crate::common::Sha256HasherOriginal;
use crate::{
    alloc::string::ToString,
    big_mod_exp::{big_mod_exp, BigModExpParams},
    common::{HasherImpl, Keccak256Hasher, Sha256Hasher},
    context::InvokeContext,
    declare_builtin_function,
    error::{Error, Secp256k1RecoverError, SvmError, SyscallError},
    hash::{SECP256K1_PUBLIC_KEY_LENGTH, SECP256K1_SIGNATURE_LENGTH},
    loaders::syscalls::cpi::cpi_common,
    mem_ops::{
        is_nonoverlapping,
        memcmp,
        memmove,
        translate_and_check_program_address_inputs,
        translate_slice,
        translate_slice_mut,
        translate_string_and_do,
        translate_type,
        translate_type_mut,
    },
    word_size::{
        common::{typecast_bytes, MemoryMappingHelper},
        slice::{RetVal, SliceFatPtr64, SpecMethods},
    },
};
use alloc::{boxed::Box, vec::Vec};
use core::str::from_utf8;
use fluentbase_sdk::{debug_log_ext, SharedAPI, B256};
use itertools::Itertools;
use solana_bn254::{
    compression::prelude::convert_endianness,
    prelude::{
        alt_bn128_multiplication,
        alt_bn128_pairing,
        ALT_BN128_ADDITION_OUTPUT_LEN,
        ALT_BN128_MULTIPLICATION_OUTPUT_LEN,
        ALT_BN128_PAIRING_OUTPUT_LEN,
    },
    target_arch::alt_bn128_addition,
    AltBn128Error,
};
use solana_curve25519::{edwards, ristretto, scalar};
use solana_feature_set::{abort_on_invalid_curve, simplify_alt_bn128_syscall_error_codes};
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
    function_registry
        .register_function_hashed("sol_memcmp_", SyscallMemcmp::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_memset_", SyscallMemset::vm)
        .unwrap();

    function_registry
        .register_function_hashed("sol_invoke_signed_rust", SyscallInvokeSignedRust::vm)
        .unwrap();

    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_secp256k1_recover_original",
            SyscallSecp256k1RecoverOriginal::vm,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_secp256k1_recover_original", SyscallStub::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_secp256k1_recover", SyscallSecp256k1Recover::vm)
        .unwrap();
    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_curve_group_op_original",
            SyscallCurveGroupOpsOriginal::vm,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_curve_group_op_original", SyscallStub::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_curve_group_op", SyscallCurveGroupOps::vm)
        .unwrap();
    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_curve_multiscalar_mul_original",
            SyscallCurveMultiscalarMultiplicationOriginal::vm,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_curve_multiscalar_mul_original", SyscallStub::vm)
        .unwrap();
    function_registry
        .register_function_hashed(
            "sol_curve_multiscalar_mul",
            SyscallCurveMultiscalarMultiplication::vm,
        )
        .unwrap();
    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_curve_validate_point_original",
            SyscallCurvePointValidationOriginal::vm,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_curve_validate_point_original", SyscallStub::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_curve_validate_point", SyscallCurvePointValidation::vm)
        .unwrap();

    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_alt_bn128_group_op_original",
            SyscallAltBn128Original::vm,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_alt_bn128_group_op_original", SyscallStub::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_alt_bn128_group_op", SyscallAltBn128::vm)
        .unwrap();

    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_alt_bn128_compression_original",
            SyscallAltBn128CompressionOriginal::vm,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_alt_bn128_compression_original", SyscallStub::vm)
        .unwrap();
    function_registry
        .register_function_hashed("sol_alt_bn128_compression", SyscallAltBn128Compression::vm)
        .unwrap();

    #[cfg(feature = "enable-solana-original-builtins")]
    function_registry
        .register_function_hashed(
            "sol_sha256_original",
            SyscallHash::vm::<SDK, Sha256HasherOriginal>,
        )
        .unwrap();
    #[cfg(not(feature = "enable-solana-original-builtins"))]
    function_registry
        .register_function_hashed("sol_sha256_original", SyscallStub::vm)
        .unwrap();

    function_registry
        .register_function_hashed("sol_sha256", SyscallHash::vm::<SDK, Sha256Hasher<SDK>>)
        .unwrap();
    function_registry
        .register_function_hashed(
            "sol_keccak256",
            SyscallHash::vm::<SDK, Keccak256Hasher<SDK>>,
        )
        .unwrap();
    #[cfg(feature = "enable-solana-extended-builtins")]
    function_registry
        .register_function_hashed("sol_blake3", SyscallHash::vm::<SDK, Blake3Hasher>)
        .unwrap();
    #[cfg(not(feature = "enable-solana-extended-builtins"))]
    function_registry
        .register_function_hashed("sol_blake3", SyscallStub::vm)
        .unwrap();
    #[cfg(feature = "enable-solana-extended-builtins")]
    function_registry
        .register_function_hashed("sol_big_mod_exp", SyscallBigModExp::vm)
        .unwrap();
    #[cfg(not(feature = "enable-solana-extended-builtins"))]
    function_registry
        .register_function_hashed("sol_big_mod_exp", SyscallStub::vm)
        .unwrap();
    #[cfg(feature = "enable-solana-extended-builtins")]
    function_registry
        .register_function_hashed("sol_poseidon", SyscallPoseidon::vm)
        .unwrap();
    #[cfg(not(feature = "enable-solana-extended-builtins"))]
    function_registry
        .register_function_hashed("sol_poseidon", SyscallStub::vm)
        .unwrap();
}

macro_rules! log_str_common {
    ($value:expr) => {
        #[allow(unused)]
        {
            #[cfg(all(test, not(target_arch = "wasm32")))]
            println!("svm_log: {}", $value);
            #[cfg(target_arch = "wasm32")]
            {
                #[cfg(not(feature = "use-extended-debug-log"))]
                use fluentbase_sdk::debug_log as log_macro;
                #[cfg(feature = "use-extended-debug-log")]
                use fluentbase_sdk::debug_log_ext as log_macro;
                log_macro!("svm_log: {}", $value);
            }
        }
    };
}

// TODO check again
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
        let s1 = translate_slice::<u8>(
            memory_mapping,
            s1_addr,
            n,
            invoke_context.get_check_aligned(),
        )?;
        let s2 = translate_slice::<u8>(
            memory_mapping,
            s2_addr,
            n,
            invoke_context.get_check_aligned(),
        )?;
        let cmp_result = translate_type_mut::<i32>(
            memory_mapping,
            cmp_result_addr,
            invoke_context.get_check_aligned(),
            false,
        )?;

        debug_assert_eq!(s1.len(), n as usize);
        debug_assert_eq!(s2.len(), n as usize);
        // Safety:
        // memcmp is marked unsafe since it assumes that the inputs are at least
        // `n` bytes long. `s1` and `s2` are guaranteed to be exactly `n` bytes
        // long because `translate_slice` would have failed otherwise.
        *cmp_result = unsafe { memcmp(s1.as_slice(), s2.as_slice(), n as usize) };

        Ok(0)
    }
);

// TODO check again
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

// TODO check again
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

// TODO check again
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
        let host_addr: Result<u64, EbpfError> =
            memory_mapping.map(AccessType::Load, vm_addr, len).into();
        let host_addr = host_addr?;
        unsafe {
            let c_buf = alloc::slice::from_raw_parts(host_addr as *const u8, len as usize);
            let len = c_buf.iter().position(|c| *c == 0).unwrap_or(len as usize);
            #[allow(unused_variables)]
            let message = from_utf8(&c_buf[0..len]).unwrap_or("Invalid UTF-8 String");
            log_str_common!(message);
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
        log_str_common!(&alloc::format!("{arg1:#x}, {arg2:#x}, {arg3:#x}, {arg4:#x}, {arg5:#x}"));
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
        #[allow(unused_variables)]
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
        #[allow(unused_variables)]
        let data_fields = untranslated_data_fields
            .iter()
            .map(|untranslated_data_field| {
                Ok(untranslated_data_field.as_ref().to_vec_cloned())
            })
            .collect::<Result<Vec<_>, SvmError>>()?;

        log_str_common!(alloc::format!("hex fields: {:x?}", &data_fields));

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
    /// Panic syscall function, called when the SBF program calls 'sol_panic_()`
    /// Causes the SBF program to be halted immediately
    SyscallStub<SDK: SharedAPI>,
    fn rust(
        _invoke_context: &mut InvokeContext<SDK>,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        panic!("function disabled")
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
                let bytes = untranslated_val.as_ref().as_slice();
                hasher.hash(bytes);
            }
        }
        let hasher_result = hasher.result();
        let hashing_result = hasher_result.as_ref();
        hash_result.copy_from_slice(hashing_result);
        Ok(0)
    }
);

declare_builtin_function!(
    /// secp256k1_recover
    SyscallSecp256k1RecoverOriginal<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        hash_addr: u64,
        recovery_id_val: u64,
        signature_addr: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let hash = translate_slice::<u8>(
            memory_mapping,
            hash_addr,
            solana_hash::HASH_BYTES as u64,
            invoke_context.get_check_aligned(),
        )?;
        let signature = translate_slice::<u8>(
            memory_mapping,
            signature_addr,
            SECP256K1_SIGNATURE_LENGTH as u64,
            invoke_context.get_check_aligned(),
        )?;
        let mut result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            SECP256K1_PUBLIC_KEY_LENGTH as u64,
            invoke_context.get_check_aligned(),
        )?;

        let Ok(message) = libsecp256k1::Message::parse_slice(hash.as_slice()) else {
            return Ok(Secp256k1RecoverError::InvalidHash.into());
        };
        let Ok(adjusted_recover_id_val) = recovery_id_val.try_into() else {
            return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
        };
        let Ok(recovery_id) =libsecp256k1::RecoveryId::parse(adjusted_recover_id_val) else {
            return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
        };
        let Ok(signature) = libsecp256k1::Signature::parse_standard_slice(signature.as_slice()) else {
            return Ok(Secp256k1RecoverError::InvalidSignature.into());
        };

        let public_key = match libsecp256k1::recover(&message, &signature, &recovery_id) {
            Ok(key) => key.serialize(),
            Err(_) => {
                return Ok(Secp256k1RecoverError::InvalidSignature.into());
            }
        };

        result.copy_from_slice(&public_key[1..65]);
        Ok(SUCCESS)
    }
);

declare_builtin_function!(
    /// secp256k1_recover
    SyscallSecp256k1Recover<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        hash_addr: u64,
        recovery_id_val: u64,
        signature_addr: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let hash = translate_slice::<u8>(
            memory_mapping,
            hash_addr,
            solana_hash::HASH_BYTES as u64,
            invoke_context.get_check_aligned(),
        )?;
        let signature = translate_slice::<u8>(
            memory_mapping,
            signature_addr,
            SECP256K1_SIGNATURE_LENGTH as u64,
            invoke_context.get_check_aligned(),
        )?;
        let mut result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            SECP256K1_PUBLIC_KEY_LENGTH as u64,
            invoke_context.get_check_aligned(),
        )?;

        let Ok(digest) = B256::try_from(hash.as_slice()) else {
            return Ok(Secp256k1RecoverError::InvalidHash.into());
        };
        let Ok(recovery_id) = recovery_id_val.try_into() else {
            return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
        };
        let Ok(signature) = signature.as_slice().try_into() else {
            return Ok(Secp256k1RecoverError::InvalidSignature.into());
        };

        let public_key = match SDK::secp256k1_recover(&digest, &signature, recovery_id) {
            Some(key) => key,
            None => {
                return Ok(Secp256k1RecoverError::InvalidSignature.into());
            }
        };

        result.copy_from_slice(&public_key[1..65]);
        Ok(SUCCESS)
    }
);

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
        use crate::error::RuntimeError;
        let parameters: solana_poseidon::Parameters = parameters.try_into().map_err(|_| RuntimeError::InvalidConversion)?;
        let endianness: solana_poseidon::Endianness = endianness.try_into().map_err(|_| RuntimeError::InvalidConversion)?;

        if vals_len > 12 {
            debug_log_ext!("Poseidon hashing {} sequences is not supported", vals_len);
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

declare_builtin_function!(
    /// Big integer modular exponentiation
    SyscallBigModExp<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        params: u64,
        return_value: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let params_slice = &translate_slice::<BigModExpParams>(
            memory_mapping,
            params,
            1,
            invoke_context.get_check_aligned(),
        )?;
        let params_ret = params_slice.try_get(0).ok_or(SyscallError::InvalidLength)?;
        let params = params_ret.as_ref();

        if params.base_len > 512 || params.exponent_len > 512 || params.modulus_len > 512 {
            return Err(Box::new(SyscallError::InvalidLength));
        }

        // let input_len: u64 = core::cmp::max(params.base_len, params.exponent_len);
        // let input_len: u64 = core::cmp::max(input_len, params.modulus_len);

        let base = translate_slice::<u8>(
            memory_mapping,
            params.base,
            params.base_len,
            invoke_context.get_check_aligned(),
        )?;

        let exponent = translate_slice::<u8>(
            memory_mapping,
            params.exponent,
            params.exponent_len,
            invoke_context.get_check_aligned(),
        )?;

        let modulus = translate_slice::<u8>(
            memory_mapping,
            params.modulus,
            params.modulus_len,
            invoke_context.get_check_aligned(),
        )?;

        let value = big_mod_exp(base.as_slice(), exponent.as_slice(), modulus.as_slice());

        let mut return_value = translate_slice_mut::<u8>(
            memory_mapping,
            return_value,
            params.modulus_len,
            invoke_context.get_check_aligned(),
        )?;
        return_value.copy_from_slice(value.as_slice());

        Ok(0)
    }
);

declare_builtin_function!(
    // Elliptic Curve Point Validation
    //
    // Currently, only curve25519 Edwards and Ristretto representations are supported
    SyscallCurvePointValidationOriginal<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        curve_id: u64,
        point_addr: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_curve25519::{curve_syscall_traits::*, edwards, ristretto};
        match curve_id {
            CURVE25519_EDWARDS => {
                let point = translate_type::<edwards::PodEdwardsPoint>(
                    memory_mapping,
                    point_addr,
                    invoke_context.get_check_aligned(),
                    false,
                )?;

                if edwards::validate_edwards(&point) {
                    Ok(0)
                } else {
                    Ok(1)
                }
            }
            CURVE25519_RISTRETTO => {
                let point = translate_type::<ristretto::PodRistrettoPoint>(
                    memory_mapping,
                    point_addr,
                    invoke_context.get_check_aligned(),
                    false,
                )?;

                if ristretto::validate_ristretto(point) {
                    Ok(0)
                } else {
                    Ok(1)
                }
            }
            _ => {
                if invoke_context
                    .get_feature_set()
                    .is_active(&abort_on_invalid_curve::id())
                {
                    Err(SyscallError::InvalidAttribute.into())
                } else {
                    Ok(1)
                }
            }
        }
    }
);

declare_builtin_function!(
    // Elliptic Curve Point Validation
    //
    // Currently, only curve25519 Edwards and Ristretto representations are supported
    SyscallCurvePointValidation<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        curve_id: u64,
        point_addr: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_curve25519::{curve_syscall_traits::*};
        match curve_id {
            CURVE25519_EDWARDS => {
                let point = translate_type::<[u8; 32]>(
                    memory_mapping,
                    point_addr,
                    invoke_context.get_check_aligned(),
                    false,
                )?;

                if SDK::ed25519_edwards_decompress_validate(point) {
                    Ok(0)
                } else {
                    Ok(1)
                }
            }
            CURVE25519_RISTRETTO => {
                let point = translate_type::<[u8; 32]>(
                    memory_mapping,
                    point_addr,
                    invoke_context.get_check_aligned(),
                    false,
                )?;

                if SDK::ed25519_ristretto_decompress_validate(point) {
                    Ok(0)
                } else {
                    Ok(1)
                }
            }
            _ => {
                if invoke_context
                    .get_feature_set()
                    .is_active(&abort_on_invalid_curve::id())
                {
                    Err(SyscallError::InvalidAttribute.into())
                } else {
                    Ok(1)
                }
            }
        }
    }
);

declare_builtin_function!(
    // Elliptic Curve Group Operations
    //
    // Currently, only curve25519 Edwards and Ristretto representations are supported
    SyscallCurveGroupOpsOriginal<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        curve_id: u64,
        group_op: u64,
        left_input_addr: u64,
        right_input_addr: u64,
        result_point_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_curve25519::{curve_syscall_traits::*, edwards, ristretto, scalar};
        match curve_id {
            CURVE25519_EDWARDS => match group_op {
                ADD => {
                    let left_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    if let Some(result_point) = edwards::add_edwards(left_point, right_point) {
                        *translate_type_mut::<edwards::PodEdwardsPoint>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = result_point;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                SUB => {
                    let left_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    if let Some(result_point) = edwards::subtract_edwards(left_point, right_point) {
                        *translate_type_mut::<edwards::PodEdwardsPoint>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = result_point;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                MUL => {
                    let scalar = translate_type::<scalar::PodScalar>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let input_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    if let Some(result_point) = edwards::multiply_edwards(scalar, input_point) {
                        *translate_type_mut::<edwards::PodEdwardsPoint>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = result_point;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                _ => {
                    if invoke_context
                        .get_feature_set()
                        .is_active(&abort_on_invalid_curve::id())
                    {
                        Err(SyscallError::InvalidAttribute.into())
                    } else {
                        Ok(1)
                    }
                }
            },

            CURVE25519_RISTRETTO => match group_op {
                ADD => {
                    let left_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    if let Some(result_point) = ristretto::add_ristretto(left_point, right_point) {
                        *translate_type_mut::<ristretto::PodRistrettoPoint>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                        false,
                        )? = result_point;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                SUB => {
                    let left_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    if let Some(result_point) =
                        ristretto::subtract_ristretto(left_point, right_point)
                    {
                        *translate_type_mut::<ristretto::PodRistrettoPoint>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = result_point;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                MUL => {
                    let scalar = translate_type::<scalar::PodScalar>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let input_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    if let Some(result_point) = ristretto::multiply_ristretto(scalar, input_point) {
                        *translate_type_mut::<ristretto::PodRistrettoPoint>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = result_point;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                _ => {
                    if invoke_context
                        .get_feature_set()
                        .is_active(&abort_on_invalid_curve::id())
                    {
                        Err(SyscallError::InvalidAttribute.into())
                    } else {
                        Ok(1)
                    }
                }
            },

            _ => {
                if invoke_context
                    .get_feature_set()
                    .is_active(&abort_on_invalid_curve::id())
                {
                    Err(SyscallError::InvalidAttribute.into())
                } else {
                    Ok(1)
                }
            }
        }
    }
);

declare_builtin_function!(
    // Elliptic Curve Group Operations
    //
    // Currently, only curve25519 Edwards and Ristretto representations are supported
    SyscallCurveGroupOps<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        curve_id: u64,
        group_op: u64,
        left_input_addr: u64,
        right_input_addr: u64,
        result_point_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_curve25519::{curve_syscall_traits::*, edwards, ristretto, scalar};
        match curve_id {
            CURVE25519_EDWARDS => match group_op {
                ADD => {
                    let left_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    let mut left_point_or_result = left_point.0.clone();
                    if SDK::ed25519_edwards_add(&mut left_point_or_result, &right_point.0) {
                        *translate_type_mut::<[u8; 32]>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = left_point_or_result;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                SUB => {
                    let left_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    let mut left_point_or_result = left_point.0.clone();

                    if SDK::ed25519_edwards_sub(&mut left_point_or_result, &right_point.0) {
                        *translate_type_mut::<[u8; 32]>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = left_point_or_result;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                MUL => {
                    let scalar = translate_type::<scalar::PodScalar>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let input_point = translate_type::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    let mut left_point_or_result = input_point.0.clone();
                    if SDK::ed25519_edwards_mul(&mut left_point_or_result, &scalar.0) {
                        *translate_type_mut::<[u8; 32]>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = left_point_or_result;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                _ => {
                    if invoke_context
                        .get_feature_set()
                        .is_active(&abort_on_invalid_curve::id())
                    {
                        Err(SyscallError::InvalidAttribute.into())
                    } else {
                        Ok(1)
                    }
                }
            },

            CURVE25519_RISTRETTO => match group_op {
                ADD => {
                    let left_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    let mut left_point_or_result = left_point.0.clone();
                    if SDK::ed25519_ristretto_add(&mut left_point_or_result, &right_point.0) {
                        *translate_type_mut::<[u8; 32]>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = left_point_or_result;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                SUB => {
                    let left_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let right_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    let mut left_point_or_result = left_point.0.clone();
                    if SDK::ed25519_ristretto_sub(&mut left_point_or_result, &right_point.0) {
                        *translate_type_mut::<[u8; 32]>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = left_point_or_result;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                MUL => {
                    let scalar = translate_type::<scalar::PodScalar>(
                        memory_mapping,
                        left_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;
                    let input_point = translate_type::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        right_input_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )?;

                    let mut left_point_or_result = input_point.0.clone();
                    if SDK::ed25519_ristretto_mul(&mut left_point_or_result, &scalar.0) {
                        *translate_type_mut::<[u8; 32]>(
                            memory_mapping,
                            result_point_addr,
                            invoke_context.get_check_aligned(),
                            false,
                        )? = left_point_or_result;
                        Ok(0)
                    } else {
                        Ok(1)
                    }
                }
                _ => {
                    if invoke_context
                        .get_feature_set()
                        .is_active(&abort_on_invalid_curve::id())
                    {
                        Err(SyscallError::InvalidAttribute.into())
                    } else {
                        Ok(1)
                    }
                }
            },

            _ => {
                if invoke_context
                    .get_feature_set()
                    .is_active(&abort_on_invalid_curve::id())
                {
                    Err(SyscallError::InvalidAttribute.into())
                } else {
                    Ok(1)
                }
            }
        }
    }
);

macro_rules! impl_spec_methods_for_tuple_struct {
    ($typ:ty) => {
        impl<'a> SpecMethods<'a> for $typ {
            const ITEM_SIZE_BYTES: usize = size_of::<Self>();

            fn recover_from_bytes(
                byte_repr: &'a [u8],
                _memory_mapping_helper: MemoryMappingHelper<'a>,
            ) -> RetVal<'a, Self>
            where
                Self: Sized,
            {
                RetVal::Reference(typecast_bytes(byte_repr))
            }
        }
    };
}

impl<'a> SpecMethods<'a> for scalar::PodScalar {
    const ITEM_SIZE_BYTES: usize = size_of::<Self>();

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        _memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self>
    where
        Self: Sized,
    {
        RetVal::Reference(typecast_bytes(byte_repr))
    }
}

impl_spec_methods_for_tuple_struct!(edwards::PodEdwardsPoint);
impl_spec_methods_for_tuple_struct!(ristretto::PodRistrettoPoint);

declare_builtin_function!(
    // Elliptic Curve Multiscalar Multiplication
    //
    // Currently, only curve25519 Edwards and Ristretto representations are supported
    SyscallCurveMultiscalarMultiplicationOriginal<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        curve_id: u64,
        scalars_addr: u64,
        points_addr: u64,
        points_len: u64,
        result_point_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_curve25519::{curve_syscall_traits::*, edwards, ristretto, scalar};

        if points_len > 512 {
            return Err(Box::new(SyscallError::InvalidLength));
        }

        match curve_id {
            CURVE25519_EDWARDS => {
                let scalars = translate_slice::<scalar::PodScalar>(
                    memory_mapping,
                    scalars_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                let points = translate_slice::<edwards::PodEdwardsPoint>(
                    memory_mapping,
                    points_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                if let Some(result_point) = edwards::multiscalar_multiply_edwards(scalars.as_slice(), points.as_slice()) {
                    *translate_type_mut::<edwards::PodEdwardsPoint>(
                        memory_mapping,
                        result_point_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )? = result_point;
                    Ok(0)
                } else {
                    Ok(1)
                }
            }

            CURVE25519_RISTRETTO => {
                let scalars = translate_slice::<scalar::PodScalar>(
                    memory_mapping,
                    scalars_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                let points = translate_slice::<ristretto::PodRistrettoPoint>(
                    memory_mapping,
                    points_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                if let Some(result_point) =
                    ristretto::multiscalar_multiply_ristretto(scalars.as_slice(), points.as_slice())
                {
                    *translate_type_mut::<ristretto::PodRistrettoPoint>(
                        memory_mapping,
                        result_point_addr,
                        invoke_context.get_check_aligned(),
                    false,
                    )? = result_point;
                    Ok(0)
                } else {
                    Ok(1)
                }
            }

            _ => {
                if invoke_context
                    .get_feature_set()
                    .is_active(&abort_on_invalid_curve::id())
                {
                    Err(SyscallError::InvalidAttribute.into())
                } else {
                    Ok(1)
                }
            }
        }
    }
);

declare_builtin_function!(
    // Elliptic Curve Multiscalar Multiplication
    //
    // Currently, only curve25519 Edwards and Ristretto representations are supported
    SyscallCurveMultiscalarMultiplication<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        curve_id: u64,
        scalars_addr: u64,
        points_addr: u64,
        points_len: u64,
        result_point_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_curve25519::{curve_syscall_traits::*, edwards, ristretto, scalar};

        if points_len > 512 {
            return Err(Box::new(SyscallError::InvalidLength));
        }

        match curve_id {
            CURVE25519_EDWARDS => {
                let scalars = translate_slice::<scalar::PodScalar>(
                    memory_mapping,
                    scalars_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                let points = translate_slice::<edwards::PodEdwardsPoint>(
                    memory_mapping,
                    points_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                let mut result = [0u8; 32];
                let pairs = points.as_slice().iter()
                    .zip(scalars.as_slice().iter())
                    .map(|v| (v.0.0, v.1.0)).collect_vec();
                if SDK::ed25519_edwards_multiscalar_mul(&pairs, &mut result) {
                    *translate_type_mut::<[u8; 32]>(
                        memory_mapping,
                        result_point_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )? = result;
                    Ok(0)
                } else {
                    Ok(1)
                }
            }

            CURVE25519_RISTRETTO => {
                let scalars = translate_slice::<scalar::PodScalar>(
                    memory_mapping,
                    scalars_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                let points = translate_slice::<ristretto::PodRistrettoPoint>(
                    memory_mapping,
                    points_addr,
                    points_len,
                    invoke_context.get_check_aligned(),
                )?;

                let mut result = [0u8; 32];
                let pairs = points.as_slice().iter()
                    .zip(scalars.as_slice().iter())
                    .map(|v| (v.0.0, v.1.0)).collect_vec();
                if SDK::ed25519_ristretto_multiscalar_mul(&pairs, &mut result) {
                    *translate_type_mut::<[u8; 32]>(
                        memory_mapping,
                        result_point_addr,
                        invoke_context.get_check_aligned(),
                        false,
                    )? = result;
                    Ok(0)
                } else {
                    Ok(1)
                }
            }

            _ => {
                if invoke_context
                    .get_feature_set()
                    .is_active(&abort_on_invalid_curve::id())
                {
                    Err(SyscallError::InvalidAttribute.into())
                } else {
                    Ok(1)
                }
            }
        }
    }
);

fn convert_endianness_64(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunks(32)
        .flat_map(|b| b.iter().copied().rev().collect::<Vec<u8>>())
        .collect::<Vec<u8>>()
}

fn convert_endianness_128(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunks(64)
        .flat_map(|b| b.iter().copied().rev().collect::<Vec<u8>>())
        .collect::<Vec<u8>>()
}

declare_builtin_function!(
    /// alt_bn128 group operations
    SyscallAltBn128Original<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        group_op: u64,
        input_addr: u64,
        input_size: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_bn254::prelude::{ALT_BN128_ADD, ALT_BN128_MUL, ALT_BN128_PAIRING};
        let output: usize = match group_op {
            ALT_BN128_ADD =>
                ALT_BN128_ADDITION_OUTPUT_LEN,
            ALT_BN128_MUL =>
                ALT_BN128_MULTIPLICATION_OUTPUT_LEN,
            ALT_BN128_PAIRING => {
                ALT_BN128_PAIRING_OUTPUT_LEN
            }
            _ => {
                return Err(SyscallError::InvalidAttribute.into());
            }
        };

        let input = translate_slice::<u8>(
            memory_mapping,
            input_addr,
            input_size,
            invoke_context.get_check_aligned(),
        )?;

        let mut call_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            output as u64,
            invoke_context.get_check_aligned(),
        )?;

        let calculation = match group_op {
            ALT_BN128_ADD => alt_bn128_addition,
            ALT_BN128_MUL => alt_bn128_multiplication,
            ALT_BN128_PAIRING => alt_bn128_pairing,
            _ => {
                return Err(SyscallError::InvalidAttribute.into());
            }
        };

        let simplify_alt_bn128_syscall_error_codes = invoke_context
            .get_feature_set()
            .is_active(&simplify_alt_bn128_syscall_error_codes::id());

        let result_point = match calculation(input.as_slice()) {
            Ok(result_point) => result_point,
            Err(e) => {
                return if simplify_alt_bn128_syscall_error_codes {
                    Ok(1)
                } else {
                    Ok(e.into())
                };
            }
        };

        // This can never happen and should be removed when the
        // simplify_alt_bn128_syscall_error_codes feature gets activated
        if result_point.len() != output && !simplify_alt_bn128_syscall_error_codes {
            return Ok(AltBn128Error::SliceOutOfBounds.into());
        }

        call_result.copy_from_slice(&result_point);
        Ok(SUCCESS)
    }
);

declare_builtin_function!(
    /// alt_bn128 group operations
    SyscallAltBn128<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        group_op: u64,
        input_addr: u64,
        input_size: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_bn254::prelude::{ALT_BN128_ADD, ALT_BN128_MUL, ALT_BN128_PAIRING};
        let output: usize = match group_op {
            ALT_BN128_ADD =>
                ALT_BN128_ADDITION_OUTPUT_LEN,
            ALT_BN128_MUL =>
                ALT_BN128_MULTIPLICATION_OUTPUT_LEN,
            ALT_BN128_PAIRING => {
                ALT_BN128_PAIRING_OUTPUT_LEN
            }
            _ => {
                return Err(SyscallError::InvalidAttribute.into());
            }
        };

        let input = translate_slice::<u8>(
            memory_mapping,
            input_addr,
            input_size,
            invoke_context.get_check_aligned(),
        )?;

        let mut call_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            output as u64,
            invoke_context.get_check_aligned(),
        )?;

        let calculation = match group_op {
            ALT_BN128_ADD => |input: &[u8]| -> Result<Vec<u8>, AltBn128Error> {
                const MAX_LEN: usize = 128;
                if input.len() > MAX_LEN {
                    return Err(AltBn128Error::InvalidInputData);
                }
                let mut input = input.to_vec();
                input.resize(MAX_LEN, 0);

                let mut p: [u8; 64] = convert_endianness_64(&input[0..64]).try_into().unwrap();
                let q: [u8; 64] = convert_endianness_64(&input[64..MAX_LEN]).try_into().unwrap();
                SDK::bn254_add(&mut p, &q);
                Ok(convert_endianness_64(&p).to_vec())
            },
            ALT_BN128_MUL => |input: &[u8]| -> Result<Vec<u8>, AltBn128Error> {
                const MAX_LEN: usize = 96;
                if input.len() > MAX_LEN {
                    return Err(AltBn128Error::InvalidInputData);
                }
                let mut input = input.to_vec();
                input.resize(MAX_LEN, 0);

                let mut p: [u8; 64] = convert_endianness_64(&input[0..64]).try_into().unwrap();
                let q: [u8; 32] = convert_endianness_64(&input[64..MAX_LEN]).try_into().unwrap();
                SDK::bn254_mul(&mut p, &q);
                Ok(convert_endianness_64(&p).to_vec())
            },
            ALT_BN128_PAIRING => |input: &[u8]| -> Result<Vec<u8>, AltBn128Error> {
                const PAIRING_ELEMENT_LEN: usize = 192;
                const G1_POINT_SIZE: usize = 64;
                const G2_POINT_SIZE: usize = 128;

                // let mut vec_pairs: Vec<([u8; G1_POINT_SIZE], [u8; G2_POINT_SIZE])> = Default::default();
                let mut vec_pairs2: Vec<([u8; G1_POINT_SIZE], [u8; G2_POINT_SIZE])> = Vec::new();
                let ele_len = input.len().saturating_div(PAIRING_ELEMENT_LEN);
                for i in 0..ele_len {
                    let g1_vec = convert_endianness_64(
                        &input[i.saturating_mul(PAIRING_ELEMENT_LEN)
                            ..i.saturating_mul(PAIRING_ELEMENT_LEN)
                                .saturating_add(G1_POINT_SIZE)],
                    );
                    let g2_vec = convert_endianness_128(
                        &input[i
                            .saturating_mul(PAIRING_ELEMENT_LEN)
                            .saturating_add(G1_POINT_SIZE)
                            ..i.saturating_mul(PAIRING_ELEMENT_LEN)
                                .saturating_add(PAIRING_ELEMENT_LEN)],
                    );
                    vec_pairs2.push((g1_vec.clone().try_into().unwrap(), g2_vec.clone().try_into().unwrap()));
                }

                let output = SDK::bn254_multi_pairing(&vec_pairs2);
                Ok(convert_endianness_64(&output))
            },
            _ => {
                return Err(SyscallError::InvalidAttribute.into());
            }
        };

        let simplify_alt_bn128_syscall_error_codes = invoke_context
            .get_feature_set()
            .is_active(&simplify_alt_bn128_syscall_error_codes::id());

        let result_point = match calculation(input.as_slice()) {
            Ok(result_point) => result_point,
            Err(e) => {
                return if simplify_alt_bn128_syscall_error_codes {
                    Ok(1)
                } else {
                    Ok(e.into())
                };
            }
        };

        // This can never happen and should be removed when the
        // simplify_alt_bn128_syscall_error_codes feature gets activated
        if result_point.len() != output && !simplify_alt_bn128_syscall_error_codes {
            return Ok(AltBn128Error::SliceOutOfBounds.into());
        }

        call_result.copy_from_slice(&result_point);
        Ok(SUCCESS)
    }
);

declare_builtin_function!(
    /// alt_bn128 g1 and g2 compression and decompression
    SyscallAltBn128CompressionOriginal<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        op: u64,
        input_addr: u64,
        input_size: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_bn254::compression::prelude::{
            alt_bn128_g1_compress, alt_bn128_g1_decompress, alt_bn128_g2_compress,
            alt_bn128_g2_decompress, ALT_BN128_G1_COMPRESS, ALT_BN128_G1_DECOMPRESS,
            ALT_BN128_G2_COMPRESS, ALT_BN128_G2_DECOMPRESS, G1, G1_COMPRESSED, G2, G2_COMPRESSED,
        };
        let output: usize = match op {
            ALT_BN128_G1_COMPRESS => G1_COMPRESSED,
            ALT_BN128_G1_DECOMPRESS => {
                G1
            }
            ALT_BN128_G2_COMPRESS => G2_COMPRESSED,
            ALT_BN128_G2_DECOMPRESS => {
                G2
            }
            _ => {
                return Err(SyscallError::InvalidAttribute.into());
            }
        };

        let input = translate_slice::<u8>(
            memory_mapping,
            input_addr,
            input_size,
            invoke_context.get_check_aligned(),
        )?;

        let mut call_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            output as u64,
            invoke_context.get_check_aligned(),
        )?;

        let simplify_alt_bn128_syscall_error_codes = invoke_context
            .get_feature_set()
            .is_active(&simplify_alt_bn128_syscall_error_codes::id());

        match op {
            ALT_BN128_G1_COMPRESS => {
                let result_point = match alt_bn128_g1_compress(input.as_slice()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Ok(e.into())
                        };
                    }
                };
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            ALT_BN128_G1_DECOMPRESS => {
                let result_point = match alt_bn128_g1_decompress(input.as_slice()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Ok(e.into())
                        };
                    }
                };
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            ALT_BN128_G2_COMPRESS => {
                let result_point = match alt_bn128_g2_compress(input.as_slice()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Ok(e.into())
                        };
                    }
                };
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            ALT_BN128_G2_DECOMPRESS => {
                let result_point = match alt_bn128_g2_decompress(input.as_slice()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Ok(e.into())
                        };
                    }
                };
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            _ => Err(SyscallError::InvalidAttribute.into()),
        }
    }
);

declare_builtin_function!(
    /// alt_bn128 g1 and g2 compression and decompression
    SyscallAltBn128Compression<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        op: u64,
        input_addr: u64,
        input_size: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        use solana_bn254::compression::prelude::{
            alt_bn128_g2_compress, alt_bn128_g2_decompress, ALT_BN128_G1_COMPRESS, ALT_BN128_G1_DECOMPRESS,
            ALT_BN128_G2_COMPRESS, ALT_BN128_G2_DECOMPRESS, G1, G1_COMPRESSED, G2, G2_COMPRESSED,
        };
        let output: usize = match op {
            ALT_BN128_G1_COMPRESS => G1_COMPRESSED,
            ALT_BN128_G1_DECOMPRESS => {
                G1
            }
            ALT_BN128_G2_COMPRESS => G2_COMPRESSED,
            ALT_BN128_G2_DECOMPRESS => {
                G2
            }
            _ => {
                return Err(SyscallError::InvalidAttribute.into());
            }
        };

        let input = translate_slice::<u8>(
            memory_mapping,
            input_addr,
            input_size,
            invoke_context.get_check_aligned(),
        )?;

        let mut call_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            output as u64,
            invoke_context.get_check_aligned(),
        )?;

        let simplify_alt_bn128_syscall_error_codes = invoke_context
            .get_feature_set()
            .is_active(&simplify_alt_bn128_syscall_error_codes::id());

        match op {
            ALT_BN128_G1_COMPRESS => {
                let input = convert_endianness::<G1_COMPRESSED, G1>(input.as_slice().try_into().unwrap());
                let result_point = match SDK::bn254_g1_compress(input.as_slice().try_into().unwrap()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Err(SvmError::ExitCode(e).into())
                        };
                    }
                };
                let result_point = convert_endianness::<G1_COMPRESSED, G1_COMPRESSED>(&result_point);
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            ALT_BN128_G1_DECOMPRESS => {
                let input = convert_endianness::<G1_COMPRESSED, G1_COMPRESSED>(input.as_slice().try_into().unwrap());
                let result_point = match SDK::bn254_g1_decompress(input.as_slice().try_into().unwrap()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Err(SvmError::ExitCode(e).into())
                        };
                    }
                };
                let result_point = convert_endianness::<G1_COMPRESSED, G1>(&result_point);
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            ALT_BN128_G2_COMPRESS => {
                let input = convert_endianness::<G2_COMPRESSED, G2>(input.as_slice().try_into().unwrap());
                let result_point = match SDK::bn254_g2_compress(input.as_slice().try_into().unwrap()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Err(SvmError::ExitCode(e).into())
                        };
                    }
                };
                let result_point = convert_endianness::<G2_COMPRESSED, G2_COMPRESSED>(&result_point);
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            ALT_BN128_G2_DECOMPRESS => {
                let input = convert_endianness::<G2_COMPRESSED, G2_COMPRESSED>(input.as_slice().try_into().unwrap());
                let result_point = match SDK::bn254_g2_decompress(input.as_slice().try_into().unwrap()) {
                    Ok(result_point) => result_point,
                    Err(e) => {
                        return if simplify_alt_bn128_syscall_error_codes {
                            Ok(1)
                        } else {
                            Err(SvmError::ExitCode(e).into())
                        };
                    }
                };
                let result_point = convert_endianness::<G2_COMPRESSED, G2>(&result_point);
                call_result.copy_from_slice(&result_point);
                Ok(SUCCESS)
            }
            _ => Err(SyscallError::InvalidAttribute.into()),
        }
    }
);
