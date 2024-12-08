use crate::blended::svm_common::HasherImpl;
use core::{slice::from_raw_parts, str::from_utf8};
use fluentbase_sdk::SovereignAPI;
use solana_ee_core::{
    context::ExecContextObject,
    helpers::{
        is_nonoverlapping,
        memcmp,
        memmove,
        translate_slice,
        translate_slice_mut,
        translate_string_and_do,
        translate_type_mut,
        Error,
        InvokeContext,
        SyscallError,
    },
};
use solana_program::{
    entrypoint::SUCCESS,
    keccak,
    poseidon,
    secp256k1_recover::{
        Secp256k1RecoverError,
        SECP256K1_PUBLIC_KEY_LENGTH,
        SECP256K1_SIGNATURE_LENGTH,
    },
};
use solana_rbpf::{
    declare_builtin_function,
    error::EbpfError,
    memory_region::{AccessType, MemoryMapping},
};

declare_builtin_function!(
    /// memcpy
    SyscallMemcpy<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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
    SyscallMemcmp<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        s1_addr: u64,
        s2_addr: u64,
        n: u64,
        cmp_result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // mem_op_consume(invoke_context, n)?;

        // if invoke_context
        //     .feature_set
        //     .is_active(&feature_set::bpf_account_data_direct_mapping::id())
        // {
        //     let cmp_result = translate_type_mut::<i32>(
        //         memory_mapping,
        //         cmp_result_addr,
        //         invoke_context.get_check_aligned(),
        //     )?;
        //     *cmp_result = memcmp_non_contiguous(s1_addr, s2_addr, n, memory_mapping)?;
        // } else {
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
        )?;

        debug_assert_eq!(s1.len(), n as usize);
        debug_assert_eq!(s2.len(), n as usize);
        // Safety:
        // memcmp is marked unsafe since it assumes that the inputs are at least
        // `n` bytes long. `s1` and `s2` are guaranteed to be exactly `n` bytes
        // long because `translate_slice` would have failed otherwise.
        *cmp_result = unsafe { memcmp(s1, s2, n as usize) };
        // }

        Ok(0)
    }
);

declare_builtin_function!(
    /// memmove
    SyscallMemmove<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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
    SyscallMemset<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        dst_addr: u64,
        c: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // mem_op_consume(invoke_context, n)?;

        // if invoke_context
        //     .feature_set
        //     .is_active(&feature_set::bpf_account_data_direct_mapping::id())
        // {
        //     memset_non_contiguous(dst_addr, c as u8, n, memory_mapping)
        // } else {
        let s = translate_slice_mut::<u8>(
            memory_mapping,
            dst_addr,
            n,
            invoke_context.get_check_aligned(),
        )?;
        s.fill(c as u8);
        Ok(0)
        // }
    }
);

declare_builtin_function!(
    /// Prints a NULL-terminated UTF-8 string.
    SyscallString<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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
            let c_buf = from_raw_parts(host_addr as *const u8, len as usize);
            let len = c_buf.iter().position(|c| *c == 0).unwrap_or(len as usize);
            let message = from_utf8(&c_buf[0..len]).unwrap_or("Invalid UTF-8 String");
            println!("message={}", message);
        }
        Ok(0)
    }
);

declare_builtin_function!(
    /// Log a user's info message
    SyscallLog<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // let cost = invoke_context
        //     .get_compute_budget()
        //     .syscall_base_cost
        //     .max(len);
        // consume_compute_meter(invoke_context, cost)?;

        translate_string_and_do(
            memory_mapping,
            addr,
            len,
            // invoke_context.get_check_aligned(),
            true,
            &mut |string: &str| {
                // stable_log::program_log(&invoke_context.get_log_collector(), string);
                #[cfg(all(feature = "std"))]
                println!("Log: {string}");
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
    SyscallAbort<SDK: SovereignAPI>,
    fn rust(
        _invoke_context: &mut ExecContextObject<SDK>,
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
    // Generic Hashing Syscall
    SyscallHash<SDK: SovereignAPI, H: HasherImpl>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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

        let hash_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            std::mem::size_of::<H::Output>() as u64,
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
                let bytes = translate_slice::<u8>(
                    memory_mapping,
                    val.as_ptr() as u64,
                    val.len() as u64,
                    invoke_context.get_check_aligned(),
                )?;
                hasher.hash(bytes);
            }
        }
        hash_result.copy_from_slice(hasher.result().as_ref());
        Ok(0)
    }
);

declare_builtin_function!(
    /// secp256k1_recover
    SyscallSecp256k1Recover<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        hash_addr: u64,
        recovery_id_val: u64,
        signature_addr: u64,
        result_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // let cost = invoke_context.get_compute_budget().secp256k1_recover_cost;
        // consume_compute_meter(invoke_context, cost)?;

        let hash = translate_slice::<u8>(
            memory_mapping,
            hash_addr,
            keccak::HASH_BYTES as u64,
            invoke_context.get_check_aligned(),
        )?;
        let signature = translate_slice::<u8>(
            memory_mapping,
            signature_addr,
            SECP256K1_SIGNATURE_LENGTH as u64,
            invoke_context.get_check_aligned(),
        )?;
        let secp256k1_recover_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            SECP256K1_PUBLIC_KEY_LENGTH as u64,
            invoke_context.get_check_aligned(),
        )?;

        let Ok(message) = libsecp256k1::Message::parse_slice(hash) else {
            return Ok(Secp256k1RecoverError::InvalidHash.into());
        };
        let Ok(adjusted_recover_id_val) = recovery_id_val.try_into() else {
            return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
        };
        let Ok(recovery_id) = libsecp256k1::RecoveryId::parse(adjusted_recover_id_val) else {
            return Ok(Secp256k1RecoverError::InvalidRecoveryId.into());
        };
        let Ok(signature) = libsecp256k1::Signature::parse_standard_slice(signature) else {
            return Ok(Secp256k1RecoverError::InvalidSignature.into());
        };

        let public_key = match libsecp256k1::recover(&message, &signature, &recovery_id) {
            Ok(key) => key.serialize(),
            Err(_) => {
                return Ok(Secp256k1RecoverError::InvalidSignature.into());
            }
        };

        secp256k1_recover_result.copy_from_slice(&public_key[1..65]);
        Ok(SUCCESS)
    }
);

declare_builtin_function!(
    SyscallPoseidon<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        parameters: u64,
        endianness: u64,
        vals_addr: u64,
        vals_len: u64,
        result_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let parameters: poseidon::Parameters = parameters.try_into().map_err(|e| {
            SyscallError::Abort
        })?;
        let endianness: poseidon::Endianness = endianness.try_into().map_err(|e| {
            SyscallError::Abort
        })?;

        if vals_len > 12 {
            // ic_msg!(
            //     invoke_context,
            //     "Poseidon hashing {} sequences is not supported",
            //     vals_len,
            // );
            return Err(SyscallError::InvalidLength.into());
        }

        // let budget = invoke_context.get_compute_budget();
        // let Some(cost) = budget.poseidon_cost(vals_len) else {
        //     ic_msg!(
        //         invoke_context,
        //         "Overflow while calculating the compute cost"
        //     );
        //     return Err(SyscallError::ArithmeticOverflow.into());
        // };
        // consume_compute_meter(invoke_context, cost.to_owned())?;

        let hash_result = translate_slice_mut::<u8>(
            memory_mapping,
            result_addr,
            poseidon::HASH_BYTES as u64,
            invoke_context.get_check_aligned(),
        )?;
        let inputs = translate_slice::<&[u8]>(
            memory_mapping,
            vals_addr,
            vals_len,
            invoke_context.get_check_aligned(),
        )?;
        let inputs = inputs
            .iter()
            .map(|input| {
                translate_slice::<u8>(
                    memory_mapping,
                    input.as_ptr() as *const _ as u64,
                    input.len() as u64,
                    invoke_context.get_check_aligned(),
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;

        // let simplify_alt_bn128_syscall_error_codes = invoke_context
        //     .feature_set
        //     .is_active(&feature_set::simplify_alt_bn128_syscall_error_codes::id());

        let hash = match poseidon::hashv(parameters, endianness, inputs.as_slice()) {
            Ok(hash) => hash,
            Err(e) => {
                return /*if simplify_alt_bn128_syscall_error_codes {
                    Ok(1)
                } else {*/
                    Ok(e.into());
                /*};*/
            }
        };
        hash_result.copy_from_slice(&hash.to_bytes());

        Ok(SUCCESS)
    }
);
