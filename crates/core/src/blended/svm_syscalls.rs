use crate::blended::{svm::SVM_ADDRESS_PREFIX, svm_common::HasherImpl};
use core::{slice::from_raw_parts, str::from_utf8};
use fluentbase_sdk::{calc_create2_address, Address, SovereignAPI, B256, U256};
use solana_ee_core::{
    context::ExecContextObject,
    helpers::{
        is_nonoverlapping,
        memcmp,
        memmove,
        translate_and_check_program_address_inputs,
        translate_slice,
        translate_slice_mut,
        translate_string_and_do,
        translate_type_mut,
        Error,
        SyscallError,
    },
};
use solana_program::{
    entrypoint::SUCCESS,
    keccak,
    poseidon,
    pubkey::{bytes_are_curve_point, Pubkey, PubkeyError},
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
    /// Panic syscall function, called when the SBF program calls 'sol_panic_()`
    /// Causes the SBF program to be halted immediately
    SyscallPanic<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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
            core::mem::size_of::<H::Output>() as u64,
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

declare_builtin_function!(
    SyscallPoseidonSDK<SDK: SovereignAPI>,
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

        let mut inputs_vec = Vec::new();
        for i in &inputs {
            inputs_vec.extend_from_slice(i);
        }
        let hash = SDK::poseidon(&inputs_vec);

        // let hash = match poseidon::hashv(parameters, endianness, inputs.as_slice()) {
        //     Ok(hash) => hash,
        //     Err(e) => {
        //         return /*if simplify_alt_bn128_syscall_error_codes {
        //             Ok(1)
        //         } else {*/
        //             Ok(e.into());
        //         /*};*/
        //     }
        // };
        hash_result.copy_from_slice(&hash.0);

        Ok(SUCCESS)
    }
);

fn create_program_address<SDK: SovereignAPI>(
    seeds: &Vec<&[u8]>,
    program_id: &Pubkey,
) -> Result<[u8; 32], Error> {
    let deployer = program_id.to_bytes();
    // TODO do we need this check?
    if deployer[0..SVM_ADDRESS_PREFIX.len()] != SVM_ADDRESS_PREFIX {
        return Err(SyscallError::ProgramNotSupported(program_id.clone()).into());
    }
    let deployer: [u8; 20] = deployer[SVM_ADDRESS_PREFIX.len()..].try_into().unwrap();
    let deployer = Address::new(deployer);
    let mut salt = U256::default();
    if seeds.len() <= 0 {
        return Err(SyscallError::InvalidLength.into());
    } else {
        let mut seeds_bytes = Vec::new();
        seeds.iter().for_each(|&v| {
            seeds_bytes.extend_from_slice(&v);
        });
        salt = U256::from_be_bytes(SDK::keccak256(&seeds_bytes).0);
    }
    // TODO what to use as a code hash
    let init_code_hash = B256::default();
    let evm_address = calc_create2_address::<SDK>(&deployer, &salt, &init_code_hash);
    let mut new_address = [0u8; 32];
    new_address[0..SVM_ADDRESS_PREFIX.len()].copy_from_slice(&SVM_ADDRESS_PREFIX);
    new_address[SVM_ADDRESS_PREFIX.len()..].copy_from_slice(evm_address.as_slice());
    if bytes_are_curve_point(new_address.as_slice()) {
        return Err(SyscallError::BadSeeds(PubkeyError::InvalidSeeds).into());
    }
    Ok(new_address)
}

declare_builtin_function!(
    /// Create a program address
    SyscallCreateProgramAddress<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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
        let new_address = create_program_address::<SDK>(&seeds, program_id)?;
        let address = translate_slice_mut::<u8>(
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
    SyscallTryFindProgramAddress<SDK: SovereignAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
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

        let (seeds, program_id) = translate_and_check_program_address_inputs(
            seeds_addr,
            seeds_len,
            program_id_addr,
            memory_mapping,
            invoke_context.get_check_aligned(),
        )?;

        let mut bump_seed = [core::u8::MAX];
        for _ in 0..core::u8::MAX {
            {
                let mut seeds_with_bump = seeds.to_vec();
                seeds_with_bump.push(&bump_seed);

                let new_address = create_program_address::<SDK>(&seeds_with_bump, program_id);
                if let Ok(new_address) =
                    // Pubkey::create_program_address(&seeds_with_bump, program_id)
                    new_address
                {
                    let bump_seed_ref = translate_type_mut::<u8>(
                        memory_mapping,
                        bump_seed_addr,
                        invoke_context.get_check_aligned(),
                    )?;
                    let address = translate_slice_mut::<u8>(
                        memory_mapping,
                        address_addr,
                        core::mem::size_of::<Pubkey>() as u64,
                        invoke_context.get_check_aligned(),
                    )?;
                    if !is_nonoverlapping(
                        bump_seed_ref as *const _ as usize,
                        core::mem::size_of_val(bump_seed_ref),
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
