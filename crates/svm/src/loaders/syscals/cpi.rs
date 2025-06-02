use super::*;
use crate::{
    account::BorrowedAccount,
    bpf_loader,
    bpf_loader_deprecated,
    builtins::SyscallInvokeSignedRust,
    context::{IndexOfAccount, InstructionAccount, InvokeContext},
    error::{Error, SvmError},
    helpers::{SerializedAccountMetadata, SyscallError},
    mem_ops::{
        translate,
        translate_slice,
        translate_slice_mut,
        translate_type,
        translate_type_mut,
    },
    mem_ops_original,
    native_loader,
    precompiles::is_precompile,
    ptr_size::slice_fat_ptr64::{
        reconstruct_slice,
        ElementConstraints,
        RetVal,
        SliceFatPtr64,
        SliceFatPtr64Repr,
        SpecMethods,
        STABLE_VEC_FAT_PTR64_BYTE_SIZE,
    },
    serialization::account_data_region_memory_state,
    solana_program::bpf_loader_upgradeable,
};
use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};
use core::{cell::RefCell, marker::PhantomData, ptr};
use fluentbase_sdk::{debug_log, SharedAPI};
use scopeguard::defer;
use solana_account_info::{AccountInfo, MAX_PERMITTED_DATA_INCREASE};
use solana_feature_set::{self as feature_set, enable_bpf_loader_set_authority_checked_ix};
use solana_instruction::{error::InstructionError, AccountMeta};
use solana_program_entrypoint::{BPF_ALIGN_OF_U128, SUCCESS};
use solana_pubkey::{Pubkey, MAX_SEEDS, PUBKEY_BYTES};
use solana_rbpf::{
    ebpf,
    memory_region::{MemoryMapping, MemoryRegion, MemoryState},
};
use solana_stable_layout::{stable_instruction::StableInstruction, stable_vec::StableVec};

fn check_account_info_pointer<SDK: SharedAPI>(
    _invoke_context: &InvokeContext<SDK>,
    vm_addr: u64,
    expected_vm_addr: u64,
    _field: &str,
) -> Result<(), SvmError> {
    if vm_addr != expected_vm_addr {
        // ic_msg!(
        //     invoke_context,
        //     "Invalid account info pointer `{}': {:#x} != {:#x}",
        //     field,
        //     vm_addr,
        //     expected_vm_addr
        // );
        return Err(SyscallError::InvalidPointer.into());
    }
    Ok(())
}

enum VmValue<'a, 'b, T> {
    VmAddress {
        vm_addr: u64,
        memory_mapping: &'b MemoryMapping<'a>,
        check_aligned: bool,
    },
    // Once direct mapping is activated, this variant can be removed and the
    // enum can be made a struct.
    Translated(&'a mut T),
}

impl<'a, 'b, T> VmValue<'a, 'b, T> {
    fn get(&self) -> Result<&T, SvmError> {
        match self {
            VmValue::VmAddress {
                vm_addr,
                memory_mapping,
                check_aligned,
            } => translate_type(memory_mapping, *vm_addr, *check_aligned, false),
            VmValue::Translated(addr) => Ok(*addr),
        }
    }

    fn get_mut(&mut self) -> Result<&mut T, SvmError> {
        match self {
            VmValue::VmAddress {
                vm_addr,
                memory_mapping,
                check_aligned,
            } => translate_type_mut(memory_mapping, *vm_addr, *check_aligned, false),
            VmValue::Translated(addr) => Ok(*addr),
        }
    }
}

/// Host side representation of AccountInfo or SolAccountInfo passed to the CPI syscall.
///
/// At the start of a CPI, this can be different from the data stored in the
/// corresponding BorrowedAccount, and needs to be synched.
struct CallerAccount<'a, 'b, SDK: SharedAPI> {
    lamports: &'a mut u64,
    owner: &'a mut Pubkey,
    // The original data length of the account at the start of the current
    // instruction. We use this to determine wether an account was shrunk or
    // grown before or after CPI, and to derive the vm address of the realloc
    // region.
    original_data_len: usize,
    // This points to the data section for this account, as serialized and
    // mapped inside the vm (see serialize_parameters() in
    // BpfExecutor::execute).
    //
    // This is only set when direct mapping is off (see the relevant comment in
    // CallerAccount::from_account_info).
    serialized_data: SliceFatPtr64<'a, u8>,
    // Given the corresponding input AccountInfo::data, vm_data_addr points to
    // the pointer field and ref_to_len_in_vm points to the length field.
    vm_data_addr: u64,
    ref_to_len_in_vm: VmValue<'b, 'a, u64>,
    _phantom_data: PhantomData<SDK>,
}

impl<'a, 'b, SDK: SharedAPI> CallerAccount<'a, 'b, SDK> {
    // Create a CallerAccount given an AccountInfo.
    fn from_account_info(
        invoke_context: &InvokeContext<SDK>,
        memory_mapping: &'b MemoryMapping<'a>,
        _vm_addr: u64,
        account_info: &AccountInfo,
        account_metadata: &SerializedAccountMetadata,
    ) -> Result<CallerAccount<'a, 'b, SDK>, SvmError> {
        let direct_mapping = invoke_context
            .get_feature_set()
            .is_active(&feature_set::bpf_account_data_direct_mapping::id());

        if direct_mapping {
            check_account_info_pointer(
                invoke_context,
                account_info.key as *const _ as u64,
                account_metadata.vm_key_addr,
                "key",
            )?;
            check_account_info_pointer(
                invoke_context,
                account_info.owner as *const _ as u64,
                account_metadata.vm_owner_addr,
                "owner",
            )?;
        }

        // account_info points to host memory. The addresses used internally are
        // in vm space so they need to be translated.
        let lamports = {
            // Double translate lamports out of RefCell
            let ptr = translate_type::<u64>(
                memory_mapping,
                account_info.lamports.as_ptr() as u64,
                invoke_context.get_check_aligned(),
                false,
            )?;
            if direct_mapping {
                if account_info.lamports.as_ptr() as u64 >= ebpf::MM_INPUT_START {
                    return Err(SyscallError::InvalidPointer.into());
                }

                check_account_info_pointer(
                    invoke_context,
                    *ptr,
                    account_metadata.vm_lamports_addr,
                    "lamports",
                )?;
            }
            translate_type_mut::<u64>(
                memory_mapping,
                *ptr,
                invoke_context.get_check_aligned(),
                false,
            )?
        };

        let owner = translate_type_mut::<Pubkey>(
            memory_mapping,
            account_info.owner as *const _ as u64,
            invoke_context.get_check_aligned(),
            false,
        )?;

        let (serialized_data, vm_data_addr, ref_to_len_in_vm) = {
            if direct_mapping && account_info.data.as_ptr() as u64 >= ebpf::MM_INPUT_START {
                return Err(SyscallError::InvalidPointer.into());
            }

            // Double translate data out of RefCell
            let data = *translate_type::<&[u8]>(
                memory_mapping,
                account_info.data.as_ptr() as *const _ as u64,
                invoke_context.get_check_aligned(),
                false,
            )?;
            if direct_mapping {
                check_account_info_pointer(
                    invoke_context,
                    data.as_ptr() as u64,
                    account_metadata.vm_data_addr,
                    "data",
                )?;
            }

            // consume_compute_meter(
            //     invoke_context,
            //     (data.len() as u64)
            //         .checked_div(invoke_context.get_compute_budget().cpi_bytes_per_unit)
            //         .unwrap_or(u64::MAX),
            // )?;

            let ref_to_len_in_vm = if direct_mapping {
                let vm_addr = (account_info.data.as_ptr() as *const u64 as u64)
                    .saturating_add(size_of::<u64>() as u64);
                // In the same vein as the other check_account_info_pointer() checks, we don't lock
                // this pointer to a specific address but we don't want it to be inside accounts, or
                // callees might be able to write to the pointed memory.
                if vm_addr >= ebpf::MM_INPUT_START {
                    return Err(SyscallError::InvalidPointer.into());
                }
                VmValue::VmAddress {
                    vm_addr,
                    memory_mapping,
                    check_aligned: invoke_context.get_check_aligned(),
                }
            } else {
                let translated = translate(
                    memory_mapping,
                    AccessType::Store,
                    (account_info.data.as_ptr() as *const u64 as u64)
                        .saturating_add(size_of::<u64>() as u64),
                    8,
                )? as *mut u64;
                VmValue::Translated(unsafe { &mut *translated })
            };
            let vm_data_addr = data.as_ptr() as u64;

            let serialized_data = if direct_mapping {
                // when direct mapping is enabled, the permissions on the
                // realloc region can change during CPI so we must delay
                // translating until when we know whether we're going to mutate
                // the realloc region or not. Consider this case:
                //
                // [caller can't write to an account] <- we are here
                // [callee grows and assigns account to the caller]
                // [caller can now write to the account]
                //
                // If we always translated the realloc area here, we'd get a
                // memory access violation since we can't write to the account
                // _yet_, but we will be able to once the caller returns.
                Default::default()
            } else {
                translate_slice_mut::<u8>(
                    memory_mapping,
                    vm_data_addr,
                    data.len() as u64,
                    invoke_context.get_check_aligned(),
                )?
            };
            (serialized_data, vm_data_addr, ref_to_len_in_vm)
        };

        Ok(CallerAccount {
            lamports,
            owner,
            original_data_len: account_metadata.original_data_len,
            serialized_data,
            vm_data_addr,
            ref_to_len_in_vm,
            _phantom_data: Default::default(),
        })
    }

    // Create a CallerAccount given a SolAccountInfo.
    fn from_sol_account_info(
        invoke_context: &InvokeContext<SDK>,
        memory_mapping: &'b MemoryMapping<'a>,
        vm_addr: u64,
        account_info: &SolAccountInfo,
        account_metadata: &SerializedAccountMetadata,
    ) -> Result<CallerAccount<'a, 'b, SDK>, Error> {
        let direct_mapping = invoke_context
            .get_feature_set()
            .is_active(&feature_set::bpf_account_data_direct_mapping::id());

        if direct_mapping {
            check_account_info_pointer(
                invoke_context,
                account_info.key_addr,
                account_metadata.vm_key_addr,
                "key",
            )?;

            check_account_info_pointer(
                invoke_context,
                account_info.owner_addr,
                account_metadata.vm_owner_addr,
                "owner",
            )?;

            check_account_info_pointer(
                invoke_context,
                account_info.lamports_addr,
                account_metadata.vm_lamports_addr,
                "lamports",
            )?;

            check_account_info_pointer(
                invoke_context,
                account_info.data_addr,
                account_metadata.vm_data_addr,
                "data",
            )?;
        }

        // account_info points to host memory. The addresses used internally are
        // in vm space so they need to be translated.
        let lamports = translate_type_mut::<u64>(
            memory_mapping,
            account_info.lamports_addr,
            invoke_context.get_check_aligned(),
            false,
        )?;
        let owner = translate_type_mut::<Pubkey>(
            memory_mapping,
            account_info.owner_addr,
            invoke_context.get_check_aligned(),
            false,
        )?;

        // consume_compute_meter(
        //     invoke_context,
        //     account_info
        //         .data_len
        //         .checked_div(invoke_context.get_compute_budget().cpi_bytes_per_unit)
        //         .unwrap_or(u64::MAX),
        // )?;

        let serialized_data = if direct_mapping {
            // See comment in CallerAccount::from_account_info()
            Default::default()
        } else {
            translate_slice_mut::<u8>(
                memory_mapping,
                account_info.data_addr,
                account_info.data_len,
                invoke_context.get_check_aligned(),
            )?
        };

        // we already have the host addr we want: &mut account_info.data_len.
        // The account info might be read only in the vm though, so we translate
        // to ensure we can write. This is tested by programs/sbf/rust/ro_modify
        // which puts SolAccountInfo in rodata.
        let data_len_vm_addr = vm_addr
            .saturating_add(&account_info.data_len as *const u64 as u64)
            .saturating_sub(account_info as *const _ as *const u64 as u64);

        let ref_to_len_in_vm = if direct_mapping {
            VmValue::VmAddress {
                vm_addr: data_len_vm_addr,
                memory_mapping,
                check_aligned: invoke_context.get_check_aligned(),
            }
        } else {
            let data_len_addr = translate(
                memory_mapping,
                AccessType::Store,
                data_len_vm_addr,
                size_of::<u64>() as u64,
            )?;
            VmValue::Translated(unsafe { &mut *(data_len_addr as *mut u64) })
        };

        Ok(CallerAccount {
            lamports,
            owner,
            original_data_len: account_metadata.original_data_len,
            serialized_data,
            vm_data_addr: account_info.data_addr,
            ref_to_len_in_vm,
            _phantom_data: Default::default(),
        })
    }

    fn realloc_region(
        &self,
        memory_mapping: &'b MemoryMapping<'_>,
        is_loader_deprecated: bool,
    ) -> Result<Option<&'a MemoryRegion>, Error> {
        account_realloc_region(
            memory_mapping,
            self.vm_data_addr,
            self.original_data_len,
            is_loader_deprecated,
        )
    }
}

type TranslatedAccounts<'a, 'b, SDK> = Vec<(IndexOfAccount, Option<CallerAccount<'a, 'b, SDK>>)>;

/// Implemented by language specific data structure translators
trait SyscallInvokeSigned<SDK: SharedAPI> {
    fn translate_instruction(
        addr: u64,
        memory_mapping: &MemoryMapping,
        invoke_context: &mut InvokeContext<SDK>,
    ) -> Result<StableInstruction, SvmError>;
    fn translate_accounts<'a, 'b>(
        instruction_accounts: &[InstructionAccount],
        program_indices: &[IndexOfAccount],
        account_infos_addr: u64,
        account_infos_len: u64,
        is_loader_deprecated: bool,
        memory_mapping: &'b MemoryMapping<'a>,
        invoke_context: &mut InvokeContext<SDK>,
    ) -> Result<TranslatedAccounts<'a, 'b, SDK>, SvmError>;
    fn translate_signers(
        program_id: &Pubkey,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
        memory_mapping: &MemoryMapping,
        invoke_context: &InvokeContext<SDK>,
    ) -> Result<Vec<Pubkey>, SvmError>;
}

// declare_builtin_function!(
//     /// Cross-program invocation called from Rust
//     SyscallInvokeSignedRust,
//     fn rust(
//         invoke_context: &mut InvokeContext,
//         instruction_addr: u64,
//         account_infos_addr: u64,
//         account_infos_len: u64,
//         signers_seeds_addr: u64,
//         signers_seeds_len: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         cpi_common::<Self>(
//             invoke_context,
//             instruction_addr,
//             account_infos_addr,
//             account_infos_len,
//             signers_seeds_addr,
//             signers_seeds_len,
//             memory_mapping,
//         )
//     }
// );

impl<SDK: SharedAPI> SyscallInvokeSigned<SDK> for SyscallInvokeSignedRust {
    fn translate_instruction(
        addr: u64,
        memory_mapping: &MemoryMapping,
        invoke_context: &mut InvokeContext<SDK>,
    ) -> Result<StableInstruction, SvmError> {
        debug_log!(
            "SyscallInvokeSigned::translate_instruction1: addr {} size_of::<StableInstruction>() {} size_of::<StableVec<AccountMeta>>() {} size_of::<StableVec<u8>>() {} size_of::<Pubkey>() {}",
            addr,
            size_of::<StableInstruction>(),
            size_of::<StableVec<AccountMeta>>(),
            size_of::<StableVec<u8>>(),
            size_of::<Pubkey>(),
        );
        const STABLE_INSTRUCTION_BYTES_SIZE: u64 = 24 * 2 + 32; // StableVec * 2 + Pubkey
        let host_addr = translate(
            memory_mapping,
            AccessType::Load,
            addr,
            STABLE_INSTRUCTION_BYTES_SIZE,
        )?;
        let accounts_ptr =
            SliceFatPtr64Repr::<{ size_of::<AccountMeta>() }>::from_ptr_to_fixed_slice_fat_ptr(
                host_addr as usize,
            );
        let data_ptr = SliceFatPtr64Repr::<{ size_of::<u8>() }>::from_ptr_to_fixed_slice_fat_ptr(
            host_addr as usize + STABLE_VEC_FAT_PTR64_BYTE_SIZE,
        );
        let program_id_addr = host_addr as usize + STABLE_VEC_FAT_PTR64_BYTE_SIZE * 2;
        let program_id_data = reconstruct_slice::<u8>(program_id_addr, PUBKEY_BYTES);
        let program_id = Pubkey::new_from_array(program_id_data.try_into().unwrap());
        let account_metas = translate_slice::<AccountMeta>(
            memory_mapping,
            accounts_ptr.first_item_fat_ptr_addr(),
            accounts_ptr.len(),
            invoke_context.get_check_aligned(),
        )?;
        debug_log!(
            "SyscallInvokeSigned::translate_instruction3: data_ptr.first_item_fat_ptr_addr {}",
            data_ptr.first_item_fat_ptr_addr()
        );
        let data = translate_slice::<u8>(
            memory_mapping,
            data_ptr.first_item_fat_ptr_addr(),
            data_ptr.len(),
            invoke_context.get_check_aligned(),
        )?;

        check_instruction_size(account_metas.len(), data.len(), invoke_context)?;

        // if invoke_context
        //     .get_feature_set()
        //     .is_active(&feature_set::loosen_cpi_size_restriction::id())
        // {
        //     consume_compute_meter(
        //         invoke_context,
        //         (data.len() as u64)
        //             .checked_div(invoke_context.get_compute_budget().cpi_bytes_per_unit)
        //             .unwrap_or(u64::MAX),
        //     )?;
        // }

        let mut accounts: Vec<AccountMeta> = Vec::with_capacity(account_metas.len());
        #[allow(clippy::needless_range_loop)]
        for account_index in 0..account_metas.len() {
            #[allow(clippy::indexing_slicing)]
            let account_meta_ret_val = account_metas.item_at_idx(account_index);
            let account_meta = account_meta_ret_val.as_ref();
            if unsafe {
                ptr::read_volatile(&account_meta.is_signer as *const _ as *const u8) > 1
                    || ptr::read_volatile(&account_meta.is_writable as *const _ as *const u8) > 1
            } {
                return Err(InstructionError::InvalidArgument.into());
            }
            accounts.push(account_meta.clone());
        }

        Ok(StableInstruction {
            accounts: accounts.into(),
            data: data
                .iter()
                .map(|v| v.as_ref().clone())
                .collect::<Vec<_>>()
                .into(),
            program_id,
        })
    }

    fn translate_accounts<'a, 'b>(
        instruction_accounts: &[InstructionAccount],
        program_indices: &[IndexOfAccount],
        account_infos_addr: u64,
        account_infos_len: u64,
        is_loader_deprecated: bool,
        memory_mapping: &'b MemoryMapping<'a>,
        invoke_context: &mut InvokeContext<SDK>,
    ) -> Result<TranslatedAccounts<'a, 'b, SDK>, SvmError> {
        debug_log!(
            "translate_accounts1: account_infos_addr {}",
            account_infos_addr
        );
        let (account_infos, account_info_keys) = translate_account_infos(
            account_infos_addr,
            account_infos_len,
            // |account_info: &AccountInfo| account_info.key as *const _ as u64,
            |account_infos: SliceFatPtr64<AccountInfo>, idx: usize, total_len: usize| {
                let key_addr = account_infos.item_addr_at_idx(idx);
                let addr = SliceFatPtr64Repr::<1>::ptr_elem_from_addr(key_addr);
                SliceFatPtr64Repr::<1>::map_vm_addr_to_host(
                    memory_mapping,
                    addr,
                    total_len as u64,
                    None,
                )
                .unwrap()
            },
            memory_mapping,
            invoke_context,
        )?;
        debug_log!(
            "translate_accounts2: account_info_keys {:x?}",
            account_info_keys
        );

        translate_and_update_accounts(
            instruction_accounts,
            program_indices,
            &account_info_keys,
            account_infos,
            account_infos_addr,
            is_loader_deprecated,
            invoke_context,
            memory_mapping,
            CallerAccount::from_account_info,
        )
    }

    fn translate_signers(
        program_id: &Pubkey,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
        memory_mapping: &MemoryMapping,
        invoke_context: &InvokeContext<SDK>,
    ) -> Result<Vec<Pubkey>, SvmError> {
        let mut signers = Vec::new();
        if signers_seeds_len > 0 {
            let signers_seeds = translate_slice::<SliceFatPtr64<SliceFatPtr64<u8>>>(
                memory_mapping,
                signers_seeds_addr,
                signers_seeds_len,
                invoke_context.get_check_aligned(),
            )?;
            if signers_seeds.len() > MAX_SIGNERS {
                return Err(SyscallError::TooManySigners.into());
            }
            for signer_seeds in signers_seeds.iter() {
                // let untranslated_seeds = mem_ops_original::translate_slice::<&[u8]>(
                //     memory_mapping,
                //     signer_seeds.as_ptr() as u64,
                //     signer_seeds.len() as u64,
                //     invoke_context.get_check_aligned(),
                // )?;
                let untranslated_seeds = signer_seeds.as_ref();
                if untranslated_seeds.len() > MAX_SEEDS {
                    return Err(InstructionError::MaxSeedLengthExceeded.into());
                }
                let seeds = untranslated_seeds
                    .iter()
                    .map(|untranslated_seed| {
                        // mem_ops_original::translate_slice::<u8>(
                        //     memory_mapping,
                        //     untranslated_seed.as_ptr() as u64,
                        //     untranslated_seed.len() as u64,
                        //     invoke_context.get_check_aligned(),
                        // )
                        let result = untranslated_seed.as_ref().to_vec_cloned();
                        result
                    })
                    .collect::<Vec<_>>();
                // let seeds: Vec<Vec<u8>> = seeds.iter().map(|v| v.to_vec()).collect();
                let signer = Pubkey::create_program_address(
                    // TODO check for problem with unstable memory layout
                    &seeds.iter().map(|v| v.as_slice()).collect::<Vec<&[u8]>>(),
                    program_id,
                );
                let signer = signer.map_err(SyscallError::BadSeeds)?;
                signers.push(signer);
            }
            Ok(signers)
        } else {
            Ok(vec![])
        }
    }
}

// /// Rust representation of C's SolInstruction
// #[derive(Debug)]
// #[repr(C)]
// struct SolInstruction {
//     program_id_addr: u64,
//     accounts_addr: u64,
//     accounts_len: u64,
//     data_addr: u64,
//     data_len: u64,
// }
//
// /// Rust representation of C's SolAccountMeta
// #[derive(Debug)]
// #[repr(C)]
// struct SolAccountMeta {
//     pubkey_addr: u64,
//     is_writable: bool,
//     is_signer: bool,
// }

/// Rust representation of C's SolAccountInfo
#[derive(Debug)]
#[repr(C)]
struct SolAccountInfo {
    key_addr: u64,
    lamports_addr: u64,
    data_len: u64,
    data_addr: u64,
    owner_addr: u64,
    rent_epoch: u64,
    is_signer: bool,
    is_writable: bool,
    executable: bool,
}

// /// Rust representation of C's SolSignerSeed
// #[derive(Debug)]
// #[repr(C)]
// struct SolSignerSeedC {
//     addr: u64,
//     len: u64,
// }
//
// /// Rust representation of C's SolSignerSeeds
// #[derive(Debug)]
// #[repr(C)]
// struct SolSignerSeedsC {
//     addr: u64,
//     len: u64,
// }
//
// declare_builtin_function!(
//     /// Cross-program invocation called from C
//     SyscallInvokeSignedC,
//     fn rust(
//         invoke_context: &mut InvokeContext,
//         instruction_addr: u64,
//         account_infos_addr: u64,
//         account_infos_len: u64,
//         signers_seeds_addr: u64,
//         signers_seeds_len: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         cpi_common::<Self>(
//             invoke_context,
//             instruction_addr,
//             account_infos_addr,
//             account_infos_len,
//             signers_seeds_addr,
//             signers_seeds_len,
//             memory_mapping,
//         )
//     }
// );
//
// impl SyscallInvokeSigned for SyscallInvokeSignedC {
//     fn translate_instruction(
//         addr: u64,
//         memory_mapping: &MemoryMapping,
//         invoke_context: &mut InvokeContext,
//     ) -> Result<StableInstruction, Error> {
//         let ix_c = translate_type::<SolInstruction>(
//             memory_mapping,
//             addr,
//             invoke_context.get_check_aligned(),
//         )?;
//
//         check_instruction_size(
//             ix_c.accounts_len as usize,
//             ix_c.data_len as usize,
//             invoke_context,
//         )?;
//         let program_id = translate_type::<Pubkey>(
//             memory_mapping,
//             ix_c.program_id_addr,
//             invoke_context.get_check_aligned(),
//         )?;
//         let account_metas = translate_slice::<SolAccountMeta>(
//             memory_mapping,
//             ix_c.accounts_addr,
//             ix_c.accounts_len,
//             invoke_context.get_check_aligned(),
//         )?;
//
//         let ix_data_len = ix_c.data_len;
//         if invoke_context
//             .get_feature_set()
//             .is_active(&feature_set::loosen_cpi_size_restriction::id())
//         {
//             consume_compute_meter(
//                 invoke_context,
//                 (ix_data_len)
//                     .checked_div(invoke_context.get_compute_budget().cpi_bytes_per_unit)
//                     .unwrap_or(u64::MAX),
//             )?;
//         }
//
//         let data = translate_slice::<u8>(
//             memory_mapping,
//             ix_c.data_addr,
//             ix_data_len,
//             invoke_context.get_check_aligned(),
//         )?
//         .to_vec();
//
//         let mut accounts = Vec::with_capacity(ix_c.accounts_len as usize);
//         #[allow(clippy::needless_range_loop)]
//         for account_index in 0..ix_c.accounts_len as usize {
//             #[allow(clippy::indexing_slicing)]
//             let account_meta = &account_metas[account_index];
//             if unsafe {
//                 std::ptr::read_volatile(&account_meta.is_signer as *const _ as *const u8) > 1
//                     || std::ptr::read_volatile(&account_meta.is_writable as *const _ as *const u8)
//                         > 1
//             } {
//                 return Err(Box::new(InstructionError::InvalidArgument));
//             }
//             let pubkey = translate_type::<Pubkey>(
//                 memory_mapping,
//                 account_meta.pubkey_addr,
//                 invoke_context.get_check_aligned(),
//             )?;
//             accounts.push(AccountMeta {
//                 pubkey: *pubkey,
//                 is_signer: account_meta.is_signer,
//                 is_writable: account_meta.is_writable,
//             });
//         }
//
//         Ok(StableInstruction {
//             accounts: accounts.into(),
//             data: data.into(),
//             program_id: *program_id,
//         })
//     }
//
//     fn translate_accounts<'a, 'b>(
//         instruction_accounts: &[InstructionAccount],
//         program_indices: &[IndexOfAccount],
//         account_infos_addr: u64,
//         account_infos_len: u64,
//         is_loader_deprecated: bool,
//         memory_mapping: &'b MemoryMapping<'a>,
//         invoke_context: &mut InvokeContext,
//     ) -> Result<TranslatedAccounts<'a, 'b>, Error> {
//         let (account_infos, account_info_keys) = translate_account_infos(
//             account_infos_addr,
//             account_infos_len,
//             |account_info: &SolAccountInfo| account_info.key_addr,
//             memory_mapping,
//             invoke_context,
//         )?;
//
//         translate_and_update_accounts(
//             instruction_accounts,
//             program_indices,
//             &account_info_keys,
//             account_infos,
//             account_infos_addr,
//             is_loader_deprecated,
//             invoke_context,
//             memory_mapping,
//             CallerAccount::from_sol_account_info,
//         )
//     }
//
//     fn translate_signers(
//         program_id: &Pubkey,
//         signers_seeds_addr: u64,
//         signers_seeds_len: u64,
//         memory_mapping: &MemoryMapping,
//         invoke_context: &InvokeContext,
//     ) -> Result<Vec<Pubkey>, Error> {
//         if signers_seeds_len > 0 {
//             let signers_seeds = translate_slice::<SolSignerSeedsC>(
//                 memory_mapping,
//                 signers_seeds_addr,
//                 signers_seeds_len,
//                 invoke_context.get_check_aligned(),
//             )?;
//             if signers_seeds.len() > MAX_SIGNERS {
//                 return Err(Box::new(SyscallError::TooManySigners));
//             }
//             Ok(signers_seeds
//                 .iter()
//                 .map(|signer_seeds| {
//                     let seeds = translate_slice::<SolSignerSeedC>(
//                         memory_mapping,
//                         signer_seeds.addr,
//                         signer_seeds.len,
//                         invoke_context.get_check_aligned(),
//                     )?;
//                     if seeds.len() > MAX_SEEDS {
//                         return Err(Box::new(InstructionError::MaxSeedLengthExceeded) as Error);
//                     }
//                     let seeds_bytes = seeds
//                         .iter()
//                         .map(|seed| {
//                             translate_slice::<u8>(
//                                 memory_mapping,
//                                 seed.addr,
//                                 seed.len,
//                                 invoke_context.get_check_aligned(),
//                             )
//                         })
//                         .collect::<Result<Vec<_>, Error>>()?;
//                     Pubkey::create_program_address(&seeds_bytes, program_id)
//                         .map_err(|err| Box::new(SyscallError::BadSeeds(err)) as Error)
//                 })
//                 .collect::<Result<Vec<_>, Error>>()?)
//         } else {
//             Ok(vec![])
//         }
//     }
// }

fn translate_account_infos<'a, T: ElementConstraints<'a> + 'a, F, SDK: SharedAPI>(
    account_infos_addr: u64,
    account_infos_len: u64,
    key_addr: F,
    memory_mapping: &'a MemoryMapping,
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(SliceFatPtr64<'a, T>, Vec<Pubkey>), SvmError>
where
    // F: Fn(&T) -> u64,
    F: Fn(SliceFatPtr64<'a, T>, usize, usize) -> u64,
{
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&feature_set::bpf_account_data_direct_mapping::id());
    debug_log!(
        "translate_account_infos1: account_infos_addr {}",
        account_infos_addr
    );

    // In the same vein as the other check_account_info_pointer() checks, we don't lock
    // this pointer to a specific address but we don't want it to be inside accounts, or
    // callees might be able to write to the pointed memory.
    if direct_mapping
        && account_infos_addr
            .saturating_add(account_infos_len.saturating_mul(size_of::<T>() as u64))
            >= ebpf::MM_INPUT_START
    {
        return Err(SyscallError::InvalidPointer.into());
    }
    debug_log!(
        "translate_account_infos2: account_infos_addr {}",
        account_infos_addr,
    );

    let account_infos =
        SliceFatPtr64::new::<true>(Some(memory_mapping), account_infos_addr, account_infos_len);
    check_account_infos(account_infos.len(), invoke_context)?;
    debug_log!(
        "translate_account_infos5: item_size_bytes {}",
        account_infos.item_size_bytes(),
        // <AccountInfo as SpecMethods>::ITEM_SIZE_BYTES
    );
    let mut account_info_keys = Vec::with_capacity(account_infos_len as usize);
    for account_index in 0..account_infos_len as usize {
        let addr = key_addr(account_infos.clone(), account_index, PUBKEY_BYTES);
        let pubkey_data = reconstruct_slice::<u8>(addr as usize, PUBKEY_BYTES);
        let pk = Pubkey::new_from_array(pubkey_data.try_into().unwrap());
        debug_log!(
            "translate_account_infos6.{} addr {} pk {:x?}",
            account_index,
            addr,
            &pk
        );
        account_info_keys.push(pk);
    }
    Ok((account_infos, account_info_keys))
}

// Finish translating accounts, build CallerAccount values and update callee
// accounts in preparation of executing the callee.
fn translate_and_update_accounts<'a, 'b, T: ElementConstraints<'a> + 'a, F, SDK: SharedAPI>(
    instruction_accounts: &[InstructionAccount],
    program_indices: &[IndexOfAccount],
    account_info_keys: &[Pubkey],
    account_infos: SliceFatPtr64<'a, T>,
    account_infos_addr: u64,
    is_loader_deprecated: bool,
    invoke_context: &mut InvokeContext<SDK>,
    memory_mapping: &'b MemoryMapping<'a>,
    do_translate: F,
) -> Result<TranslatedAccounts<'a, 'b, SDK>, SvmError>
where
    F: Fn(
        &InvokeContext<SDK>,
        &'b MemoryMapping<'a>,
        u64,
        &T,
        &SerializedAccountMetadata,
    ) -> Result<CallerAccount<'a, 'b, SDK>, SvmError>,
{
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut accounts = Vec::with_capacity(instruction_accounts.len().saturating_add(1));

    debug_log!("translate_and_update_accounts1");
    let program_account_index = program_indices
        .last()
        .ok_or_else(|| InstructionError::MissingAccount)?;
    accounts.push((*program_account_index, None));

    debug_log!("translate_and_update_accounts2");
    // unwrapping here is fine: we're in a syscall and the method below fails
    // only outside syscalls
    let accounts_metadata = &invoke_context
        .get_syscall_context()
        .unwrap()
        .accounts_metadata;

    debug_log!("translate_and_update_accounts3");
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&feature_set::bpf_account_data_direct_mapping::id());

    debug_log!("translate_and_update_accounts4");
    for (instruction_account_index, instruction_account) in instruction_accounts.iter().enumerate()
    {
        debug_log!(
            "translate_and_update_accounts5.{}",
            instruction_account_index
        );
        if instruction_account_index as IndexOfAccount != instruction_account.index_in_callee {
            continue; // Skip duplicate account
        }

        debug_log!(
            "translate_and_update_accounts6.{}",
            instruction_account_index
        );
        let callee_account = instruction_context.try_borrow_instruction_account(
            transaction_context,
            instruction_account.index_in_caller,
        )?;
        let account_key = invoke_context
            .transaction_context
            .get_key_of_account_at_index(instruction_account.index_in_transaction)?;

        debug_log!(
            "translate_and_update_accounts7.{}",
            instruction_account_index
        );
        if callee_account.is_executable() {
            // Use the known account
            // consume_compute_meter(
            //     invoke_context,
            //     (callee_account.get_data().len() as u64)
            //         .checked_div(invoke_context.get_compute_budget().cpi_bytes_per_unit)
            //         .unwrap_or(u64::MAX),
            // )?;

            accounts.push((instruction_account.index_in_caller, None));
        } else if let Some(caller_account_index) =
            account_info_keys.iter().position(|key| key == account_key)
        {
            let serialized_metadata = accounts_metadata
                .get(instruction_account.index_in_caller as usize)
                .ok_or_else(|| {
                    // ic_msg!(
                    //     invoke_context,
                    //     "Internal error: index mismatch for account {}",
                    //     account_key
                    // );
                    InstructionError::MissingAccount
                })?;

            // build the CallerAccount corresponding to this account.
            if caller_account_index >= account_infos.len() {
                return Err(SyscallError::InvalidLength.into());
            }
            #[allow(clippy::indexing_slicing)]
            let caller_account = do_translate(
                invoke_context,
                memory_mapping,
                account_infos_addr
                    .saturating_add(caller_account_index.saturating_mul(size_of::<T>()) as u64),
                account_infos.item_at_idx(caller_account_index).as_ref(),
                serialized_metadata,
            )?;

            // before initiating CPI, the caller may have modified the
            // account (caller_account). We need to update the corresponding
            // BorrowedAccount (callee_account) so the callee can see the
            // changes.
            let update_caller = update_callee_account(
                invoke_context,
                memory_mapping,
                is_loader_deprecated,
                &caller_account,
                callee_account,
                direct_mapping,
            )?;

            let caller_account = if instruction_account.is_writable || update_caller {
                Some(caller_account)
            } else {
                None
            };
            accounts.push((instruction_account.index_in_caller, caller_account));
        } else {
            // ic_msg!(
            //     invoke_context,
            //     "Instruction references an unknown account {}",
            //     account_key
            // );
            return Err(InstructionError::MissingAccount.into());
        }
    }

    Ok(accounts)
}

fn check_instruction_size<SDK: SharedAPI>(
    num_accounts: usize,
    data_len: usize,
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), SvmError> {
    if invoke_context
        .get_feature_set()
        .is_active(&feature_set::loosen_cpi_size_restriction::id())
    {
        let data_len = data_len as u64;
        let max_data_len = MAX_CPI_INSTRUCTION_DATA_LEN;
        if data_len > max_data_len {
            return Err(SyscallError::MaxInstructionDataLenExceeded {
                data_len,
                max_data_len,
            }
            .into());
        }

        let num_accounts = num_accounts as u64;
        let max_accounts = MAX_CPI_INSTRUCTION_ACCOUNTS as u64;
        if num_accounts > max_accounts {
            return Err(SyscallError::MaxInstructionAccountsExceeded {
                num_accounts,
                max_accounts,
            }
            .into());
        }
    } else {
        let max_size = invoke_context.get_compute_budget().max_cpi_instruction_size;
        let size = num_accounts
            .saturating_mul(size_of::<AccountMeta>())
            .saturating_add(data_len);
        if size > max_size {
            return Err(SyscallError::InstructionTooLarge(size, max_size).into());
        }
    }
    Ok(())
}

fn check_account_infos<SDK: SharedAPI>(
    num_account_infos: usize,
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), SvmError> {
    if invoke_context
        .get_feature_set()
        .is_active(&feature_set::loosen_cpi_size_restriction::id())
    {
        let max_cpi_account_infos = if invoke_context
            .get_feature_set()
            .is_active(&feature_set::increase_tx_account_lock_limit::id())
        {
            MAX_CPI_ACCOUNT_INFOS
        } else {
            64
        };
        let num_account_infos = num_account_infos as u64;
        let max_account_infos = max_cpi_account_infos as u64;
        if num_account_infos > max_account_infos {
            return Err(SyscallError::MaxInstructionAccountInfosExceeded {
                num_account_infos,
                max_account_infos,
            }
            .into());
        }
    } else {
        let adjusted_len = num_account_infos.saturating_mul(size_of::<Pubkey>());

        if adjusted_len > invoke_context.get_compute_budget().max_cpi_instruction_size {
            // Cap the number of account_infos a caller can pass to approximate
            // maximum that accounts that could be passed in an instruction
            return Err(SyscallError::TooManyAccounts.into());
        };
    }
    Ok(())
}

fn check_authorized_program<SDK: SharedAPI>(
    program_id: &Pubkey,
    instruction_data: &[u8],
    invoke_context: &InvokeContext<SDK>,
) -> Result<(), Error> {
    if native_loader::check_id(program_id)
        || bpf_loader::check_id(program_id)
        || bpf_loader_deprecated::check_id(program_id)
        || (bpf_loader_upgradeable::check_id(program_id)
            && !(bpf_loader_upgradeable::is_upgrade_instruction(instruction_data)
                || bpf_loader_upgradeable::is_set_authority_instruction(instruction_data)
                || (invoke_context
                    .get_feature_set()
                    .is_active(&enable_bpf_loader_set_authority_checked_ix::id())
                    && bpf_loader_upgradeable::is_set_authority_checked_instruction(
                        instruction_data,
                    ))
                || bpf_loader_upgradeable::is_close_instruction(instruction_data)))
        || is_precompile(program_id, |feature_id: &Pubkey| {
            invoke_context.get_feature_set().is_active(feature_id)
        })
    {
        return Err(Box::new(SyscallError::ProgramNotSupported(*program_id)));
    }
    Ok(())
}

/// Call process instruction, common to both Rust and C
pub(crate) fn cpi_common<SDK: SharedAPI, S: SyscallInvokeSigned<SDK>>(
    invoke_context: &mut InvokeContext<SDK>,
    instruction_addr: u64,
    account_infos_addr: u64,
    account_infos_len: u64,
    signers_seeds_addr: u64,
    signers_seeds_len: u64,
    memory_mapping: &MemoryMapping,
) -> Result<u64, Error> {
    // CPI entry.
    //
    // Translate the inputs to the syscall and synchronize the caller's account
    // changes so the callee can see them.
    // consume_compute_meter(
    //     invoke_context,
    //     invoke_context.get_compute_budget().invoke_units,
    // )?;
    // if let Some(execute_time) = invoke_context.execute_time.as_mut() {
    //     execute_time.stop();
    //     saturating_add_assign!(invoke_context.timings.execute_us, execute_time.as_us());
    // }

    debug_log!("cpi_common1");
    let instruction = S::translate_instruction(instruction_addr, memory_mapping, invoke_context)?;
    debug_log!("cpi_common2");
    let transaction_context = &invoke_context.transaction_context;
    debug_log!("cpi_common3");
    let instruction_context = transaction_context.get_current_instruction_context()?;
    debug_log!("cpi_common4");
    let caller_program_id = instruction_context.get_last_program_key(transaction_context)?;
    debug_log!("cpi_common5");
    let signers = S::translate_signers(
        caller_program_id,
        signers_seeds_addr,
        signers_seeds_len,
        memory_mapping,
        invoke_context,
    )?;
    debug_log!("cpi_common6");
    let is_loader_deprecated = *instruction_context
        .try_borrow_last_program_account(transaction_context)?
        .get_owner()
        == bpf_loader_deprecated::id();
    let (instruction_accounts, program_indices) =
        invoke_context.prepare_instruction(&instruction, &signers)?;
    check_authorized_program(&instruction.program_id, &instruction.data, invoke_context)?;

    debug_log!("cpi_common7: account_infos_addr {}", account_infos_addr);
    let mut accounts = S::translate_accounts(
        &instruction_accounts,
        &program_indices,
        account_infos_addr,
        account_infos_len,
        is_loader_deprecated,
        memory_mapping,
        invoke_context,
    )?;
    debug_log!("cpi_common8");

    // Process the callee instruction
    // let mut compute_units_consumed = 0;
    invoke_context.process_instruction(
        &instruction.data,
        &instruction_accounts,
        &program_indices,
        // &mut compute_units_consumed,
        // &mut ExecuteTimings::default(),
    )?;
    debug_log!("cpi_common9");

    // re-bind to please the borrow checker
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;

    // CPI exit.
    //
    // Synchronize the callee's account changes so the caller can see them.
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&feature_set::bpf_account_data_direct_mapping::id());

    if direct_mapping {
        // Update all perms at once before doing account data updates. This
        // isn't strictly required as we forbid updates to an account to touch
        // other accounts, but since we did have bugs around this in the past,
        // it's better to be safe than sorry.
        for (index_in_caller, caller_account) in accounts.iter() {
            if let Some(caller_account) = caller_account {
                let callee_account = instruction_context
                    .try_borrow_instruction_account(transaction_context, *index_in_caller)?;
                update_caller_account_perms(
                    memory_mapping,
                    caller_account,
                    &callee_account,
                    is_loader_deprecated,
                )?;
            }
        }
    }
    debug_log!("cpi_common10");

    for (index_in_caller, caller_account) in accounts.iter_mut() {
        if let Some(caller_account) = caller_account {
            let mut callee_account = instruction_context
                .try_borrow_instruction_account(transaction_context, *index_in_caller)?;
            update_caller_account(
                invoke_context,
                memory_mapping,
                is_loader_deprecated,
                caller_account,
                &mut callee_account,
                direct_mapping,
            )?;
        }
    }
    debug_log!("cpi_common11");

    // invoke_context.execute_time = Some(Measure::start("execute"));
    Ok(SUCCESS)
}

// Update the given account before executing CPI.
//
// caller_account and callee_account describe the same account. At CPI entry
// caller_account might include changes the caller has made to the account
// before executing CPI.
//
// This method updates callee_account so the CPI callee can see the caller's
// changes.
//
// When true is returned, the caller account must be updated after CPI. This
// is only set for direct mapping when the pointer may have changed.
fn update_callee_account<SDK: SharedAPI>(
    invoke_context: &InvokeContext<SDK>,
    memory_mapping: &MemoryMapping,
    is_loader_deprecated: bool,
    caller_account: &CallerAccount<SDK>,
    mut callee_account: BorrowedAccount<'_>,
    direct_mapping: bool,
) -> Result<bool, SvmError> {
    let mut must_update_caller = false;

    if callee_account.get_lamports() != *caller_account.lamports {
        callee_account.set_lamports(*caller_account.lamports)?;
    }

    if direct_mapping {
        let prev_len = callee_account.get_data().len();
        let post_len = *caller_account.ref_to_len_in_vm.get()? as usize;
        match callee_account
            .can_data_be_resized(post_len)
            .and_then(|_| callee_account.can_data_be_changed())
        {
            Ok(()) => {
                let realloc_bytes_used = post_len.saturating_sub(caller_account.original_data_len);
                // bpf_loader_deprecated programs don't have a realloc region
                if is_loader_deprecated && realloc_bytes_used > 0 {
                    return Err(InstructionError::InvalidRealloc.into());
                }
                if prev_len != post_len {
                    callee_account.set_data_length(post_len)?;
                    // pointer to data may have changed, so caller must be updated
                    must_update_caller = true;
                }
                if realloc_bytes_used > 0 {
                    let serialized_data = translate_slice::<u8>(
                        memory_mapping,
                        caller_account
                            .vm_data_addr
                            .saturating_add(caller_account.original_data_len as u64),
                        realloc_bytes_used as u64,
                        invoke_context.get_check_aligned(),
                    )?;
                    callee_account
                        .get_data_mut()?
                        .get_mut(caller_account.original_data_len..post_len)
                        .ok_or(SyscallError::InvalidLength)?
                        .copy_from_slice(
                            &serialized_data
                                .iter()
                                .map(|v| v.as_ref().clone())
                                .collect::<Vec<_>>(),
                        );
                }
            }
            Err(err) if prev_len != post_len => {
                return Err(err.into());
            }
            _ => {}
        }
    } else {
        // The redundant check helps to avoid the expensive data comparison if we can
        let caller_ser_data = caller_account
            .serialized_data
            .iter()
            .map(|v| v.as_ref().clone())
            .collect::<Vec<_>>();
        match callee_account
            .can_data_be_resized(caller_account.serialized_data.len())
            .and_then(|_| callee_account.can_data_be_changed())
        {
            Ok(()) => {
                callee_account.set_data_from_slice(&caller_ser_data)?;
            }
            Err(err) if callee_account.get_data() != &caller_ser_data => {
                return Err(err.into());
            }
            _ => {}
        }
    }

    // Change the owner at the end so that we are allowed to change the lamports and data before
    if callee_account.get_owner() != caller_account.owner {
        callee_account.set_owner(caller_account.owner.as_ref())?;
    }

    Ok(must_update_caller)
}

fn update_caller_account_perms<SDK: SharedAPI>(
    memory_mapping: &MemoryMapping,
    caller_account: &CallerAccount<SDK>,
    callee_account: &BorrowedAccount<'_>,
    is_loader_deprecated: bool,
) -> Result<(), Error> {
    let CallerAccount {
        original_data_len,
        vm_data_addr,
        ..
    } = caller_account;

    let data_region = account_data_region(memory_mapping, *vm_data_addr, *original_data_len)?;
    if let Some(region) = data_region {
        region
            .state
            .set(account_data_region_memory_state(callee_account));
    }
    let realloc_region = account_realloc_region(
        memory_mapping,
        *vm_data_addr,
        *original_data_len,
        is_loader_deprecated,
    )?;
    if let Some(region) = realloc_region {
        region
            .state
            .set(if callee_account.can_data_be_changed().is_ok() {
                MemoryState::Writable
            } else {
                MemoryState::Readable
            });
    }

    Ok(())
}

// Update the given account after executing CPI.
//
// caller_account and callee_account describe to the same account. At CPI exit
// callee_account might include changes the callee has made to the account
// after executing.
//
// This method updates caller_account so the CPI caller can see the callee's
// changes.
fn update_caller_account<'a, 'b, SDK: SharedAPI>(
    invoke_context: &InvokeContext<SDK>,
    memory_mapping: &'a MemoryMapping,
    is_loader_deprecated: bool,
    caller_account: &mut CallerAccount<'a, 'b, SDK>,
    callee_account: &mut BorrowedAccount<'_>,
    direct_mapping: bool,
) -> Result<(), Error> {
    *caller_account.lamports = callee_account.get_lamports();
    *caller_account.owner = *callee_account.get_owner();

    let mut zero_all_mapped_spare_capacity = false;
    if direct_mapping {
        if let Some(region) = account_data_region(
            memory_mapping,
            caller_account.vm_data_addr,
            caller_account.original_data_len,
        )? {
            // Since each instruction account is directly mapped in a memory region with a *fixed*
            // length, upon returning from CPI we must ensure that the current capacity is at least
            // the original length (what is mapped in memory), so that the account's memory region
            // never points to an invalid address.
            //
            // Note that the capacity can be smaller than the original length only if the account is
            // reallocated using the AccountSharedData API directly (deprecated) or using
            // BorrowedAccount::set_data_from_slice(), which implements an optimization to avoid an
            // extra allocation.
            let min_capacity = caller_account.original_data_len;
            if callee_account.capacity() < min_capacity {
                callee_account
                    .reserve(min_capacity.saturating_sub(callee_account.get_data().len()))?;
                zero_all_mapped_spare_capacity = true;
            }

            // If an account's data pointer has changed we must update the corresponding
            // MemoryRegion in the caller's address space. Address spaces are fixed so we don't need
            // to update the MemoryRegion's length.
            //
            // An account's data pointer can change if the account is reallocated because of CoW,
            // because of BorrowedAccount::make_data_mut or by a program that uses the
            // AccountSharedData API directly (deprecated).
            let callee_ptr = callee_account.get_data().as_ptr() as u64;
            if region.host_addr.get() != callee_ptr {
                region.host_addr.set(callee_ptr);
                zero_all_mapped_spare_capacity = true;
            }
        }
    }

    let prev_len = *caller_account.ref_to_len_in_vm.get()? as usize;
    let post_len = callee_account.get_data().len();
    if prev_len != post_len {
        let max_increase = if direct_mapping && !invoke_context.get_check_aligned() {
            0
        } else {
            MAX_PERMITTED_DATA_INCREASE
        };
        let data_overflow = post_len
            > caller_account
                .original_data_len
                .saturating_add(max_increase);
        if data_overflow {
            // ic_msg!(
            //     invoke_context,
            //     "Account data size realloc limited to {max_increase} in inner instructions",
            // );
            return Err(Box::new(InstructionError::InvalidRealloc));
        }

        // If the account has been shrunk, we're going to zero the unused memory
        // *that was previously used*.
        if post_len < prev_len {
            if direct_mapping {
                // We have two separate regions to zero out: the account data
                // and the realloc region. Here we zero the realloc region, the
                // data region is zeroed further down below.
                //
                // This is done for compatibility but really only necessary for
                // the fringe case of a program calling itself, see
                // TEST_CPI_ACCOUNT_UPDATE_CALLER_GROWS_CALLEE_SHRINKS.
                //
                // Zeroing the realloc region isn't necessary in the normal
                // invoke case because consider the following scenario:
                //
                // 1. Caller grows an account (prev_len > original_data_len)
                // 2. Caller assigns the account to the callee (needed for 3 to
                //    work)
                // 3. Callee shrinks the account (post_len < prev_len)
                //
                // In order for the caller to assign the account to the callee,
                // the caller _must_ either set the account length to zero,
                // therefore making prev_len > original_data_len impossible,
                // or it must zero the account data, therefore making the
                // zeroing we do here redundant.
                if prev_len > caller_account.original_data_len {
                    // If we get here and prev_len > original_data_len, then
                    // we've already returned InvalidRealloc for the
                    // bpf_loader_deprecated case.
                    debug_assert!(!is_loader_deprecated);

                    // Temporarily configure the realloc region as writable then set it back to
                    // whatever state it had.
                    let realloc_region = caller_account
                        .realloc_region(memory_mapping, is_loader_deprecated)?
                        .unwrap(); // unwrapping here is fine, we already asserted !is_loader_deprecated
                    let original_state = realloc_region.state.replace(MemoryState::Writable);
                    defer! {
                        realloc_region.state.set(original_state);
                    }

                    // We need to zero the unused space in the realloc region, starting after the
                    // last byte of the new data which might be > original_data_len.
                    let dirty_realloc_start = caller_account.original_data_len.max(post_len);
                    // and we want to zero up to the old length
                    let dirty_realloc_len = prev_len.saturating_sub(dirty_realloc_start);
                    let mut serialized_data = translate_slice_mut::<u8>(
                        memory_mapping,
                        caller_account
                            .vm_data_addr
                            .saturating_add(dirty_realloc_start as u64),
                        dirty_realloc_len as u64,
                        invoke_context.get_check_aligned(),
                    )?;
                    serialized_data.fill(&0);
                }
            } else {
                caller_account
                    .serialized_data
                    .get_mut(post_len..)
                    .ok_or_else(|| Box::new(InstructionError::AccountDataTooSmall))?
                    .fill(&0);
            }
        }

        // when direct mapping is enabled we don't cache the serialized data in
        // caller_account.serialized_data. See CallerAccount::from_account_info.
        if !direct_mapping {
            caller_account.serialized_data = translate_slice_mut::<u8>(
                memory_mapping,
                caller_account.vm_data_addr,
                post_len as u64,
                false, // Don't care since it is byte aligned
            )?;
        }
        // this is the len field in the AccountInfo::data slice
        *caller_account.ref_to_len_in_vm.get_mut()? = post_len as u64;

        // this is the len field in the serialized parameters
        let serialized_len_ptr = translate_type_mut::<u64>(
            memory_mapping,
            caller_account
                .vm_data_addr
                .saturating_sub(size_of::<u64>() as u64),
            invoke_context.get_check_aligned(),
            false,
        )?;
        *serialized_len_ptr = post_len as u64;
    }

    if direct_mapping {
        // Here we zero the account data region.
        //
        // If zero_all_mapped_spare_capacity=true, we need to zero regardless of whether the account
        // size changed, because the underlying vector holding the account might have been
        // reallocated and contain uninitialized memory in the spare capacity.
        //
        // See TEST_CPI_CHANGE_ACCOUNT_DATA_MEMORY_ALLOCATION for an example of
        // this case.
        let spare_len = if zero_all_mapped_spare_capacity {
            // In the unlikely case where the account data vector has
            // changed - which can happen during CoW - we zero the whole
            // extra capacity up to the original data length.
            //
            // The extra capacity up to original data length is
            // accessible from the vm and since it's uninitialized
            // memory, it could be a source of non determinism.
            caller_account.original_data_len
        } else {
            // If the allocation has not changed, we only zero the
            // difference between the previous and current lengths. The
            // rest of the memory contains whatever it contained before,
            // which is deterministic.
            prev_len
        }
        .saturating_sub(post_len);

        if spare_len > 0 {
            let dst = callee_account
                .spare_data_capacity_mut()?
                .get_mut(..spare_len)
                .ok_or_else(|| Box::new(InstructionError::AccountDataTooSmall))?
                .as_mut_ptr();
            // Safety: we check bounds above
            unsafe { ptr::write_bytes(dst, 0, spare_len) };
        }

        // Propagate changes to the realloc region in the callee up to the caller.
        let realloc_bytes_used = post_len.saturating_sub(caller_account.original_data_len);
        if realloc_bytes_used > 0 {
            // In the is_loader_deprecated case, we must have failed with
            // InvalidRealloc by now.
            debug_assert!(!is_loader_deprecated);

            let mut to_slice = {
                // If a callee reallocs an account, we write into the caller's
                // realloc region regardless of whether the caller has write
                // permissions to the account or not. If the callee has been able to
                // make changes, it means they had permissions to do so, and here
                // we're just going to reflect those changes to the caller's frame.
                //
                // Therefore we temporarily configure the realloc region as writable
                // then set it back to whatever state it had.
                let realloc_region = caller_account
                    .realloc_region(memory_mapping, is_loader_deprecated)?
                    .unwrap(); // unwrapping here is fine, we asserted !is_loader_deprecated
                let original_state = realloc_region.state.replace(MemoryState::Writable);
                defer! {
                    realloc_region.state.set(original_state);
                }

                translate_slice_mut::<u8>(
                    memory_mapping,
                    caller_account
                        .vm_data_addr
                        .saturating_add(caller_account.original_data_len as u64),
                    realloc_bytes_used as u64,
                    invoke_context.get_check_aligned(),
                )?
            };
            let from_slice = callee_account
                .get_data()
                .get(caller_account.original_data_len..post_len)
                .ok_or(SyscallError::InvalidLength)?;
            if to_slice.len() != from_slice.len() {
                return Err(Box::new(InstructionError::AccountDataTooSmall));
            }
            to_slice.copy_from_slice(from_slice);
        }
    } else {
        let to_slice = &mut caller_account.serialized_data;
        let from_slice = callee_account
            .get_data()
            .get(0..post_len)
            .ok_or(SyscallError::InvalidLength)?;
        if to_slice.len() != from_slice.len() {
            return Err(Box::new(InstructionError::AccountDataTooSmall));
        }
        to_slice.copy_from_slice(from_slice);
    }

    Ok(())
}

fn account_data_region<'a>(
    memory_mapping: &'a MemoryMapping<'_>,
    vm_data_addr: u64,
    original_data_len: usize,
) -> Result<Option<&'a MemoryRegion>, Error> {
    if original_data_len == 0 {
        return Ok(None);
    }

    // We can trust vm_data_addr to point to the correct region because we
    // enforce that in CallerAccount::from_(sol_)account_info.
    let data_region = memory_mapping.region(AccessType::Load, vm_data_addr)?;
    // vm_data_addr must always point to the beginning of the region
    debug_assert_eq!(data_region.vm_addr, vm_data_addr);
    Ok(Some(data_region))
}

fn account_realloc_region<'a>(
    memory_mapping: &'a MemoryMapping<'_>,
    vm_data_addr: u64,
    original_data_len: usize,
    is_loader_deprecated: bool,
) -> Result<Option<&'a MemoryRegion>, Error> {
    if is_loader_deprecated {
        return Ok(None);
    }

    let realloc_vm_addr = vm_data_addr.saturating_add(original_data_len as u64);
    let realloc_region = memory_mapping.region(AccessType::Load, realloc_vm_addr)?;
    debug_assert_eq!(realloc_region.vm_addr, realloc_vm_addr);
    debug_assert!((MAX_PERMITTED_DATA_INCREASE
        ..MAX_PERMITTED_DATA_INCREASE.saturating_add(BPF_ALIGN_OF_U128))
        .contains(&(realloc_region.len as usize)));
    debug_assert!(!matches!(realloc_region.state.get(), MemoryState::Cow(_)));
    Ok(Some(realloc_region))
}

// #[allow(clippy::indexing_slicing)]
// #[allow(clippy::arithmetic_side_effects)]
// #[cfg(test)]
// mod tests {
//     use {
//         super::*,
//         crate::mock_create_vm,
//         assert_matches::assert_matches,
//         solana_feature_set::bpf_account_data_direct_mapping,
//         solana_program_runtime::{
//             invoke_context::SerializedAccountMetadata, with_mock_invoke_context,
//         },
//         solana_rbpf::{
//             ebpf::MM_INPUT_START, memory_region::MemoryRegion, program::SBPFVersion, vm::Config,
//         },
//         solana_sdk::{
//             account::{Account, AccountSharedData, ReadableAccount},
//             clock::Epoch,
//             instruction::Instruction,
//             system_program,
//             transaction_context::TransactionAccount,
//         },
//         std::{
//             cell::{Cell, RefCell},
//             mem, ptr,
//             rc::Rc,
//             slice,
//         },
//     };
//
//     macro_rules! mock_invoke_context {
//         ($invoke_context:ident,
//          $transaction_context:ident,
//          $instruction_data:expr,
//          $transaction_accounts:expr,
//          $program_accounts:expr,
//          $instruction_accounts:expr) => {
//             let program_accounts = $program_accounts;
//             let instruction_data = $instruction_data;
//             let instruction_accounts = $instruction_accounts
//                 .iter()
//                 .enumerate()
//                 .map(
//                     |(index_in_callee, index_in_transaction)| InstructionAccount {
//                         index_in_transaction: *index_in_transaction as IndexOfAccount,
//                         index_in_caller: *index_in_transaction as IndexOfAccount,
//                         index_in_callee: index_in_callee as IndexOfAccount,
//                         is_signer: false,
//                         is_writable: $transaction_accounts[*index_in_transaction as usize].2,
//                     },
//                 )
//                 .collect::<Vec<_>>();
//             let transaction_accounts = $transaction_accounts
//                 .into_iter()
//                 .map(|a| (a.0, a.1))
//                 .collect::<Vec<TransactionAccount>>();
//             with_mock_invoke_context!($invoke_context, $transaction_context, transaction_accounts);
//             let mut feature_set = $invoke_context.get_feature_set().clone();
//             feature_set.deactivate(&bpf_account_data_direct_mapping::id());
//             $invoke_context.mock_set_feature_set(Arc::new(feature_set));
//             $invoke_context
//                 .transaction_context
//                 .get_next_instruction_context()
//                 .unwrap()
//                 .configure(program_accounts, &instruction_accounts, instruction_data);
//             $invoke_context.push().unwrap();
//         };
//     }
//
//     macro_rules! borrow_instruction_account {
//         ($invoke_context:expr, $index:expr) => {{
//             let instruction_context = $invoke_context
//                 .transaction_context
//                 .get_current_instruction_context()
//                 .unwrap();
//             instruction_context
//                 .try_borrow_instruction_account($invoke_context.transaction_context, $index)
//                 .unwrap()
//         }};
//     }
//
//     #[test]
//     fn test_translate_instruction() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foo".to_vec());
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let program_id = Pubkey::new_unique();
//         let accounts = vec![AccountMeta {
//             pubkey: Pubkey::new_unique(),
//             is_signer: true,
//             is_writable: false,
//         }];
//         let data = b"ins data".to_vec();
//         let vm_addr = MM_INPUT_START;
//         let (_mem, region) = MockInstruction {
//             program_id,
//             accounts: accounts.clone(),
//             data: data.clone(),
//         }
//         .into_region(vm_addr);
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(vec![region], &config, &SBPFVersion::V2).unwrap();
//
//         let ins = SyscallInvokeSignedRust::translate_instruction(
//             vm_addr,
//             &memory_mapping,
//             &mut invoke_context,
//         )
//         .unwrap();
//         assert_eq!(ins.program_id, program_id);
//         assert_eq!(ins.accounts, accounts);
//         assert_eq!(ins.data, data);
//     }
//
//     #[test]
//     fn test_translate_signers() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foo".to_vec());
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let program_id = Pubkey::new_unique();
//         let (derived_key, bump_seed) = Pubkey::find_program_address(&[b"foo"], &program_id);
//
//         let vm_addr = MM_INPUT_START;
//         let (_mem, region) = mock_signers(&[b"foo", &[bump_seed]], vm_addr);
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(vec![region], &config, &SBPFVersion::V2).unwrap();
//
//         let signers = SyscallInvokeSignedRust::translate_signers(
//             &program_id,
//             vm_addr,
//             1,
//             &memory_mapping,
//             &invoke_context,
//         )
//         .unwrap();
//         assert_eq!(signers[0], derived_key);
//     }
//
//     #[test]
//     fn test_caller_account_from_account_info() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foo".to_vec());
//         let account = transaction_accounts[1].1.clone();
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let key = Pubkey::new_unique();
//         let vm_addr = MM_INPUT_START;
//         let (_mem, region, account_metadata) =
//             MockAccountInfo::new(key, &account).into_region(vm_addr);
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(vec![region], &config, &SBPFVersion::V2).unwrap();
//
//         let account_info = translate_type::<AccountInfo>(&memory_mapping, vm_addr, false).unwrap();
//
//         let caller_account = CallerAccount::from_account_info(
//             &invoke_context,
//             &memory_mapping,
//             vm_addr,
//             account_info,
//             &account_metadata,
//         )
//         .unwrap();
//         assert_eq!(*caller_account.lamports, account.lamports());
//         assert_eq!(caller_account.owner, account.owner());
//         assert_eq!(caller_account.original_data_len, account.data().len());
//         assert_eq!(
//             *caller_account.ref_to_len_in_vm.get().unwrap() as usize,
//             account.data().len()
//         );
//         assert_eq!(caller_account.serialized_data, account.data());
//     }
//
//     #[test]
//     fn test_update_caller_account_lamports_owner() {
//         let transaction_accounts = transaction_with_one_writable_instruction_account(vec![]);
//         let account = transaction_accounts[1].1.clone();
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             1234,
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             false,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let mut caller_account = mock_caller_account.caller_account();
//
//         let mut callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         callee_account.set_lamports(42).unwrap();
//         callee_account
//             .set_owner(Pubkey::new_unique().as_ref())
//             .unwrap();
//
//         update_caller_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &mut caller_account,
//             &mut callee_account,
//             false,
//         )
//         .unwrap();
//
//         assert_eq!(*caller_account.lamports, 42);
//         assert_eq!(caller_account.owner, callee_account.get_owner());
//     }
//
//     #[test]
//     fn test_update_caller_account_data() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//         let original_data_len = account.data().len();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             account.lamports(),
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             false,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let data_slice = mock_caller_account.data_slice();
//         let len_ptr = unsafe {
//             data_slice
//                 .as_ptr()
//                 .offset(-(mem::size_of::<u64>() as isize))
//         };
//         let serialized_len = || unsafe { *len_ptr.cast::<u64>() as usize };
//         let mut caller_account = mock_caller_account.caller_account();
//
//         let mut callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         for (new_value, expected_realloc_size) in [
//             (b"foo".to_vec(), MAX_PERMITTED_DATA_INCREASE + 3),
//             (b"foobaz".to_vec(), MAX_PERMITTED_DATA_INCREASE),
//             (b"foobazbad".to_vec(), MAX_PERMITTED_DATA_INCREASE - 3),
//         ] {
//             assert_eq!(caller_account.serialized_data, callee_account.get_data());
//             callee_account.set_data_from_slice(&new_value).unwrap();
//
//             update_caller_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &mut caller_account,
//                 &mut callee_account,
//                 false,
//             )
//             .unwrap();
//
//             let data_len = callee_account.get_data().len();
//             assert_eq!(
//                 data_len,
//                 *caller_account.ref_to_len_in_vm.get().unwrap() as usize
//             );
//             assert_eq!(data_len, serialized_len());
//             assert_eq!(data_len, caller_account.serialized_data.len());
//             assert_eq!(
//                 callee_account.get_data(),
//                 &caller_account.serialized_data[..data_len]
//             );
//             assert_eq!(data_slice[data_len..].len(), expected_realloc_size);
//             assert!(is_zeroed(&data_slice[data_len..]));
//         }
//
//         callee_account
//             .set_data_length(original_data_len + MAX_PERMITTED_DATA_INCREASE)
//             .unwrap();
//         update_caller_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &mut caller_account,
//             &mut callee_account,
//             false,
//         )
//         .unwrap();
//         let data_len = callee_account.get_data().len();
//         assert_eq!(data_slice[data_len..].len(), 0);
//         assert!(is_zeroed(&data_slice[data_len..]));
//
//         callee_account
//             .set_data_length(original_data_len + MAX_PERMITTED_DATA_INCREASE + 1)
//             .unwrap();
//         assert_matches!(
//             update_caller_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &mut caller_account,
//                 &mut callee_account,
//                 false,
//             ),
//             Err(error) if error.downcast_ref::<InstructionError>().unwrap() == &InstructionError::InvalidRealloc
//         );
//
//         // close the account
//         callee_account.set_data_length(0).unwrap();
//         callee_account
//             .set_owner(system_program::id().as_ref())
//             .unwrap();
//         update_caller_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &mut caller_account,
//             &mut callee_account,
//             false,
//         )
//         .unwrap();
//         let data_len = callee_account.get_data().len();
//         assert_eq!(data_len, 0);
//     }
//
//     #[test]
//     fn test_update_caller_account_data_direct_mapping() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//         let original_data_len = account.data().len();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             account.lamports(),
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             true,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let data_slice = mock_caller_account.data_slice();
//         let len_ptr = unsafe {
//             data_slice
//                 .as_ptr()
//                 .offset(-(mem::size_of::<u64>() as isize))
//         };
//         let serialized_len = || unsafe { *len_ptr.cast::<u64>() as usize };
//         let mut caller_account = mock_caller_account.caller_account();
//
//         let mut callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         for change_ptr in [false, true] {
//             for (new_value, expected_realloc_used) in [
//                 (b"foobazbad".to_vec(), 3), // > original_data_len, writes into realloc
//                 (b"foo".to_vec(), 0), // < original_data_len, zeroes account capacity + realloc capacity
//                 (b"foobaz".to_vec(), 0), // = original_data_len
//                 (vec![], 0),          // check lower bound
//             ] {
//                 if change_ptr {
//                     callee_account.set_data(new_value).unwrap();
//                 } else {
//                     callee_account.set_data_from_slice(&new_value).unwrap();
//                 }
//
//                 update_caller_account(
//                     &invoke_context,
//                     &memory_mapping,
//                     false,
//                     &mut caller_account,
//                     &mut callee_account,
//                     true,
//                 )
//                 .unwrap();
//
//                 // check that the caller account data pointer always matches the callee account data pointer
//                 assert_eq!(
//                     translate_slice::<u8>(&memory_mapping, caller_account.vm_data_addr, 1, true,)
//                         .unwrap()
//                         .as_ptr(),
//                     callee_account.get_data().as_ptr()
//                 );
//
//                 let data_len = callee_account.get_data().len();
//                 // the account info length must get updated
//                 assert_eq!(
//                     data_len,
//                     *caller_account.ref_to_len_in_vm.get().unwrap() as usize
//                 );
//                 // the length slot in the serialization parameters must be updated
//                 assert_eq!(data_len, serialized_len());
//
//                 let realloc_area = translate_slice::<u8>(
//                     &memory_mapping,
//                     caller_account
//                         .vm_data_addr
//                         .saturating_add(caller_account.original_data_len as u64),
//                     MAX_PERMITTED_DATA_INCREASE as u64,
//                     invoke_context.get_check_aligned(),
//                 )
//                 .unwrap();
//
//                 if data_len < original_data_len {
//                     // if an account gets resized below its original data length,
//                     // the spare capacity is zeroed
//                     let original_data_slice = unsafe {
//                         slice::from_raw_parts(callee_account.get_data().as_ptr(), original_data_len)
//                     };
//
//                     let spare_capacity = &original_data_slice[original_data_len - data_len..];
//                     assert!(
//                         is_zeroed(spare_capacity),
//                         "dirty account spare capacity {spare_capacity:?}",
//                     );
//                 }
//
//                 // if an account gets extended past its original length, the end
//                 // gets written in the realloc padding
//                 assert_eq!(
//                     &realloc_area[..expected_realloc_used],
//                     &callee_account.get_data()[data_len - expected_realloc_used..]
//                 );
//
//                 // the unused realloc padding is always zeroed
//                 assert!(
//                     is_zeroed(&realloc_area[expected_realloc_used..]),
//                     "dirty realloc padding {realloc_area:?}",
//                 );
//             }
//         }
//
//         callee_account
//             .set_data_length(original_data_len + MAX_PERMITTED_DATA_INCREASE)
//             .unwrap();
//         update_caller_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &mut caller_account,
//             &mut callee_account,
//             true,
//         )
//         .unwrap();
//         assert!(
//             is_zeroed(caller_account.serialized_data),
//             "dirty realloc padding {:?}",
//             caller_account.serialized_data
//         );
//
//         callee_account
//             .set_data_length(original_data_len + MAX_PERMITTED_DATA_INCREASE + 1)
//             .unwrap();
//         assert_matches!(
//             update_caller_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &mut caller_account,
//                 &mut callee_account,
//                 false,
//             ),
//             Err(error) if error.downcast_ref::<InstructionError>().unwrap() == &InstructionError::InvalidRealloc
//         );
//
//         // close the account
//         callee_account.set_data_length(0).unwrap();
//         callee_account
//             .set_owner(system_program::id().as_ref())
//             .unwrap();
//         update_caller_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &mut caller_account,
//             &mut callee_account,
//             true,
//         )
//         .unwrap();
//         let data_len = callee_account.get_data().len();
//         assert_eq!(data_len, 0);
//     }
//
//     #[test]
//     fn test_update_caller_account_data_capacity_direct_mapping() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             account.lamports(),
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             true,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let mut caller_account = mock_caller_account.caller_account();
//
//         {
//             let mut account = invoke_context
//                 .transaction_context
//                 .get_account_at_index(1)
//                 .unwrap()
//                 .borrow_mut();
//             account.set_data(b"baz".to_vec());
//         }
//
//         let mut callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_eq!(callee_account.get_data().len(), 3);
//         assert_eq!(callee_account.capacity(), 3);
//
//         update_caller_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &mut caller_account,
//             &mut callee_account,
//             true,
//         )
//         .unwrap();
//
//         assert_eq!(callee_account.get_data().len(), 3);
//         assert!(callee_account.capacity() >= caller_account.original_data_len);
//         let data = translate_slice::<u8>(
//             &memory_mapping,
//             caller_account.vm_data_addr,
//             callee_account.get_data().len() as u64,
//             true,
//         )
//         .unwrap();
//         assert_eq!(data, callee_account.get_data());
//     }
//
//     #[test]
//     fn test_update_callee_account_lamports_owner() {
//         let transaction_accounts = transaction_with_one_writable_instruction_account(vec![]);
//         let account = transaction_accounts[1].1.clone();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             1234,
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             false,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let caller_account = mock_caller_account.caller_account();
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         *caller_account.lamports = 42;
//         *caller_account.owner = Pubkey::new_unique();
//
//         update_callee_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &caller_account,
//             callee_account,
//             false,
//         )
//         .unwrap();
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_eq!(callee_account.get_lamports(), 42);
//         assert_eq!(caller_account.owner, callee_account.get_owner());
//     }
//
//     #[test]
//     fn test_update_callee_account_data() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             1234,
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             false,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let mut caller_account = mock_caller_account.caller_account();
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         let mut data = b"foo".to_vec();
//         caller_account.serialized_data = &mut data;
//
//         update_callee_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &caller_account,
//             callee_account,
//             false,
//         )
//         .unwrap();
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_eq!(callee_account.get_data(), caller_account.serialized_data);
//
//         // close the account
//         let mut data = Vec::new();
//         caller_account.serialized_data = &mut data;
//         *caller_account.ref_to_len_in_vm.get_mut().unwrap() = 0;
//         let mut owner = system_program::id();
//         caller_account.owner = &mut owner;
//         update_callee_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &caller_account,
//             callee_account,
//             false,
//         )
//         .unwrap();
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_eq!(callee_account.get_data(), b"");
//     }
//
//     #[test]
//     fn test_update_callee_account_data_readonly() {
//         let transaction_accounts =
//             transaction_with_one_readonly_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             1234,
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             false,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let mut caller_account = mock_caller_account.caller_account();
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         caller_account.serialized_data[0] = b'b';
//         assert_matches!(
//             update_callee_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &caller_account,
//                 callee_account,
//                 false,
//             ),
//             Err(error) if error.downcast_ref::<InstructionError>().unwrap() == &InstructionError::ExternalAccountDataModified
//         );
//
//         // without direct mapping
//         let mut data = b"foobarbaz".to_vec();
//         *caller_account.ref_to_len_in_vm.get_mut().unwrap() = data.len() as u64;
//         caller_account.serialized_data = &mut data;
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_matches!(
//             update_callee_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &caller_account,
//                 callee_account,
//                 false,
//             ),
//             Err(error) if error.downcast_ref::<InstructionError>().unwrap() == &InstructionError::AccountDataSizeChanged
//         );
//
//         // with direct mapping
//         let mut data = b"baz".to_vec();
//         *caller_account.ref_to_len_in_vm.get_mut().unwrap() = 9;
//         caller_account.serialized_data = &mut data;
//
//         let callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_matches!(
//             update_callee_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &caller_account,
//                 callee_account,
//                 true,
//             ),
//             Err(error) if error.downcast_ref::<InstructionError>().unwrap() == &InstructionError::AccountDataSizeChanged
//         );
//     }
//
//     #[test]
//     fn test_update_callee_account_data_direct_mapping() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1]
//         );
//
//         let mut mock_caller_account = MockCallerAccount::new(
//             1234,
//             *account.owner(),
//             0xFFFFFFFF00000000,
//             account.data(),
//             true,
//         );
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(
//             mock_caller_account.regions.split_off(0),
//             &config,
//             &SBPFVersion::V2,
//         )
//         .unwrap();
//
//         let mut caller_account = mock_caller_account.caller_account();
//
//         let mut callee_account = borrow_instruction_account!(invoke_context, 0);
//
//         // this is done when a writable account is mapped, and it ensures
//         // through make_data_mut() that the account is made writable and resized
//         // with enough padding to hold the realloc padding
//         callee_account.get_data_mut().unwrap();
//
//         let serialized_data = translate_slice_mut::<u8>(
//             &memory_mapping,
//             caller_account
//                 .vm_data_addr
//                 .saturating_add(caller_account.original_data_len as u64),
//             3,
//             invoke_context.get_check_aligned(),
//         )
//         .unwrap();
//         serialized_data.copy_from_slice(b"baz");
//
//         for (len, expected) in [
//             (9, b"foobarbaz".to_vec()), // > original_data_len, copies from realloc region
//             (6, b"foobar".to_vec()),    // == original_data_len, truncates
//             (3, b"foo".to_vec()),       // < original_data_len, truncates
//         ] {
//             *caller_account.ref_to_len_in_vm.get_mut().unwrap() = len as u64;
//             update_callee_account(
//                 &invoke_context,
//                 &memory_mapping,
//                 false,
//                 &caller_account,
//                 callee_account,
//                 true,
//             )
//             .unwrap();
//             callee_account = borrow_instruction_account!(invoke_context, 0);
//             assert_eq!(callee_account.get_data(), expected);
//         }
//
//         // close the account
//         let mut data = Vec::new();
//         caller_account.serialized_data = &mut data;
//         *caller_account.ref_to_len_in_vm.get_mut().unwrap() = 0;
//         let mut owner = system_program::id();
//         caller_account.owner = &mut owner;
//         update_callee_account(
//             &invoke_context,
//             &memory_mapping,
//             false,
//             &caller_account,
//             callee_account,
//             true,
//         )
//         .unwrap();
//         callee_account = borrow_instruction_account!(invoke_context, 0);
//         assert_eq!(callee_account.get_data(), b"");
//     }
//
//     #[test]
//     fn test_translate_accounts_rust() {
//         let transaction_accounts =
//             transaction_with_one_writable_instruction_account(b"foobar".to_vec());
//         let account = transaction_accounts[1].1.clone();
//         let key = transaction_accounts[1].0;
//         let original_data_len = account.data().len();
//
//         let vm_addr = MM_INPUT_START;
//         let (_mem, region, account_metadata) =
//             MockAccountInfo::new(key, &account).into_region(vm_addr);
//
//         let config = Config {
//             aligned_memory_mapping: false,
//             ..Config::default()
//         };
//         let memory_mapping = MemoryMapping::new(vec![region], &config, &SBPFVersion::V2).unwrap();
//
//         mock_invoke_context!(
//             invoke_context,
//             transaction_context,
//             b"instruction data",
//             transaction_accounts,
//             &[0],
//             &[1, 1]
//         );
//
//         mock_create_vm!(_vm, Vec::new(), vec![account_metadata], &mut invoke_context);
//
//         let accounts = SyscallInvokeSignedRust::translate_accounts(
//             &[
//                 InstructionAccount {
//                     index_in_transaction: 1,
//                     index_in_caller: 0,
//                     index_in_callee: 0,
//                     is_signer: false,
//                     is_writable: true,
//                 },
//                 InstructionAccount {
//                     index_in_transaction: 1,
//                     index_in_caller: 0,
//                     index_in_callee: 0,
//                     is_signer: false,
//                     is_writable: true,
//                 },
//             ],
//             &[0],
//             vm_addr,
//             1,
//             false,
//             &memory_mapping,
//             &mut invoke_context,
//         )
//         .unwrap();
//         assert_eq!(accounts.len(), 2);
//         assert!(accounts[0].1.is_none());
//         let caller_account = accounts[1].1.as_ref().unwrap();
//         assert_eq!(caller_account.serialized_data, account.data());
//         assert_eq!(caller_account.original_data_len, original_data_len);
//     }
//
//     pub type TestTransactionAccount = (Pubkey, AccountSharedData, bool);
//     struct MockCallerAccount {
//         lamports: u64,
//         owner: Pubkey,
//         vm_addr: u64,
//         data: Vec<u8>,
//         len: u64,
//         regions: Vec<MemoryRegion>,
//         direct_mapping: bool,
//     }
//
//     impl MockCallerAccount {
//         fn new(
//             lamports: u64,
//             owner: Pubkey,
//             vm_addr: u64,
//             data: &[u8],
//             direct_mapping: bool,
//         ) -> MockCallerAccount {
//             let mut regions = vec![];
//
//             let mut d = vec![
//                 0;
//                 mem::size_of::<u64>()
//                     + if direct_mapping { 0 } else { data.len() }
//                     + MAX_PERMITTED_DATA_INCREASE
//             ];
//             // always write the [len] part even when direct mapping
//             unsafe { ptr::write_unaligned::<u64>(d.as_mut_ptr().cast(), data.len() as u64) };
//
//             // write the account data when not direct mapping
//             if !direct_mapping {
//                 d[mem::size_of::<u64>()..][..data.len()].copy_from_slice(data);
//             }
//
//             // create a region for [len][data+realloc if !direct_mapping]
//             let mut region_addr = vm_addr;
//             let region_len = mem::size_of::<u64>()
//                 + if direct_mapping {
//                     0
//                 } else {
//                     data.len() + MAX_PERMITTED_DATA_INCREASE
//                 };
//             regions.push(MemoryRegion::new_writable(&mut d[..region_len], vm_addr));
//             region_addr += region_len as u64;
//
//             if direct_mapping {
//                 // create a region for the directly mapped data
//                 regions.push(MemoryRegion::new_readonly(data, region_addr));
//                 region_addr += data.len() as u64;
//
//                 // create a region for the realloc padding
//                 regions.push(MemoryRegion::new_writable(
//                     &mut d[mem::size_of::<u64>()..],
//                     region_addr,
//                 ));
//             } else {
//                 // caller_account.serialized_data must have the actual data length
//                 d.truncate(mem::size_of::<u64>() + data.len());
//             }
//
//             MockCallerAccount {
//                 lamports,
//                 owner,
//                 vm_addr,
//                 data: d,
//                 len: data.len() as u64,
//                 regions,
//                 direct_mapping,
//             }
//         }
//
//         fn data_slice<'a>(&self) -> &'a [u8] {
//             // lifetime crimes
//             unsafe {
//                 slice::from_raw_parts(
//                     self.data[mem::size_of::<u64>()..].as_ptr(),
//                     self.data.capacity() - mem::size_of::<u64>(),
//                 )
//             }
//         }
//
//         fn caller_account(&mut self) -> CallerAccount<'_, '_> {
//             let data = if self.direct_mapping {
//                 &mut []
//             } else {
//                 &mut self.data[mem::size_of::<u64>()..]
//             };
//             CallerAccount {
//                 lamports: &mut self.lamports,
//                 owner: &mut self.owner,
//                 original_data_len: self.len as usize,
//                 serialized_data: data,
//                 vm_data_addr: self.vm_addr + mem::size_of::<u64>() as u64,
//                 ref_to_len_in_vm: VmValue::Translated(&mut self.len),
//             }
//         }
//     }
//
//     fn transaction_with_one_writable_instruction_account(
//         data: Vec<u8>,
//     ) -> Vec<TestTransactionAccount> {
//         let program_id = Pubkey::new_unique();
//         let account = AccountSharedData::from(Account {
//             lamports: 1,
//             data,
//             owner: program_id,
//             executable: false,
//             rent_epoch: 100,
//         });
//         vec![
//             (
//                 program_id,
//                 AccountSharedData::from(Account {
//                     lamports: 0,
//                     data: vec![],
//                     owner: bpf_loader::id(),
//                     executable: true,
//                     rent_epoch: 0,
//                 }),
//                 false,
//             ),
//             (Pubkey::new_unique(), account, true),
//         ]
//     }
//
//     fn transaction_with_one_readonly_instruction_account(
//         data: Vec<u8>,
//     ) -> Vec<TestTransactionAccount> {
//         let program_id = Pubkey::new_unique();
//         let account_owner = Pubkey::new_unique();
//         let account = AccountSharedData::from(Account {
//             lamports: 1,
//             data,
//             owner: account_owner,
//             executable: false,
//             rent_epoch: 100,
//         });
//         vec![
//             (
//                 program_id,
//                 AccountSharedData::from(Account {
//                     lamports: 0,
//                     data: vec![],
//                     owner: bpf_loader::id(),
//                     executable: true,
//                     rent_epoch: 0,
//                 }),
//                 false,
//             ),
//             (Pubkey::new_unique(), account, true),
//         ]
//     }
//
//     struct MockInstruction {
//         program_id: Pubkey,
//         accounts: Vec<AccountMeta>,
//         data: Vec<u8>,
//     }
//
//     impl MockInstruction {
//         fn into_region(self, vm_addr: u64) -> (Vec<u8>, MemoryRegion) {
//             let accounts_len = mem::size_of::<AccountMeta>() * self.accounts.len();
//
//             let size = mem::size_of::<StableInstruction>() + accounts_len + self.data.len();
//
//             let mut data = vec![0; size];
//
//             let vm_addr = vm_addr as usize;
//             let accounts_addr = vm_addr + mem::size_of::<StableInstruction>();
//             let data_addr = accounts_addr + accounts_len;
//
//             let ins = Instruction {
//                 program_id: self.program_id,
//                 accounts: unsafe {
//                     Vec::from_raw_parts(
//                         accounts_addr as *mut _,
//                         self.accounts.len(),
//                         self.accounts.len(),
//                     )
//                 },
//                 data: unsafe {
//                     Vec::from_raw_parts(data_addr as *mut _, self.data.len(), self.data.len())
//                 },
//             };
//             let ins = StableInstruction::from(ins);
//
//             unsafe {
//                 ptr::write_unaligned(data.as_mut_ptr().cast(), ins);
//                 data[accounts_addr - vm_addr..][..accounts_len].copy_from_slice(
//                     slice::from_raw_parts(self.accounts.as_ptr().cast(), accounts_len),
//                 );
//                 data[data_addr - vm_addr..].copy_from_slice(&self.data);
//             }
//
//             let region = MemoryRegion::new_writable(data.as_mut_slice(), vm_addr as u64);
//             (data, region)
//         }
//     }
//
//     fn mock_signers(signers: &[&[u8]], vm_addr: u64) -> (Vec<u8>, MemoryRegion) {
//         let vm_addr = vm_addr as usize;
//
//         // calculate size
//         let fat_ptr_size_of_slice = mem::size_of::<&[()]>(); // pointer size + length size
//         let singers_length = signers.len();
//         let sum_signers_data_length: usize = signers.iter().map(|s| s.len()).sum();
//
//         // init data vec
//         let total_size = fat_ptr_size_of_slice
//             + singers_length * fat_ptr_size_of_slice
//             + sum_signers_data_length;
//         let mut data = vec![0; total_size];
//
//         // data is composed by 3 parts
//         // A.
//         // [ singers address, singers length, ...,
//         // B.                                      |
//         //                                         signer1 address, signer1 length, signer2 address ...,
//         //                                         ^ p1 --->
//         // C.                                                                                           |
//         //                                                                                              signer1 data, signer2 data, ... ]
//         //                                                                                              ^ p2 --->
//
//         // A.
//         data[..fat_ptr_size_of_slice / 2]
//             .clone_from_slice(&(fat_ptr_size_of_slice + vm_addr).to_le_bytes());
//         data[fat_ptr_size_of_slice / 2..fat_ptr_size_of_slice]
//             .clone_from_slice(&(singers_length).to_le_bytes());
//
//         // B. + C.
//         let (mut p1, mut p2) = (
//             fat_ptr_size_of_slice,
//             fat_ptr_size_of_slice + singers_length * fat_ptr_size_of_slice,
//         );
//         for signer in signers.iter() {
//             let signer_length = signer.len();
//
//             // B.
//             data[p1..p1 + fat_ptr_size_of_slice / 2]
//                 .clone_from_slice(&(p2 + vm_addr).to_le_bytes());
//             data[p1 + fat_ptr_size_of_slice / 2..p1 + fat_ptr_size_of_slice]
//                 .clone_from_slice(&(signer_length).to_le_bytes());
//             p1 += fat_ptr_size_of_slice;
//
//             // C.
//             data[p2..p2 + signer_length].clone_from_slice(signer);
//             p2 += signer_length;
//         }
//
//         let region = MemoryRegion::new_writable(data.as_mut_slice(), vm_addr as u64);
//         (data, region)
//     }
//
//     struct MockAccountInfo<'a> {
//         key: Pubkey,
//         is_signer: bool,
//         is_writable: bool,
//         lamports: u64,
//         data: &'a [u8],
//         owner: Pubkey,
//         executable: bool,
//         rent_epoch: Epoch,
//     }
//
//     impl<'a> MockAccountInfo<'a> {
//         fn new(key: Pubkey, account: &AccountSharedData) -> MockAccountInfo {
//             MockAccountInfo {
//                 key,
//                 is_signer: false,
//                 is_writable: false,
//                 lamports: account.lamports(),
//                 data: account.data(),
//                 owner: *account.owner(),
//                 executable: account.executable(),
//                 rent_epoch: account.rent_epoch(),
//             }
//         }
//
//         fn into_region(self, vm_addr: u64) -> (Vec<u8>, MemoryRegion, SerializedAccountMetadata) {
//             let size = mem::size_of::<AccountInfo>()
//                 + mem::size_of::<Pubkey>() * 2
//                 + mem::size_of::<RcBox<RefCell<&mut u64>>>()
//                 + mem::size_of::<u64>()
//                 + mem::size_of::<RcBox<RefCell<&mut [u8]>>>()
//                 + self.data.len();
//             let mut data = vec![0; size];
//
//             let vm_addr = vm_addr as usize;
//             let key_addr = vm_addr + mem::size_of::<AccountInfo>();
//             let lamports_cell_addr = key_addr + mem::size_of::<Pubkey>();
//             let lamports_addr = lamports_cell_addr + mem::size_of::<RcBox<RefCell<&mut u64>>>();
//             let owner_addr = lamports_addr + mem::size_of::<u64>();
//             let data_cell_addr = owner_addr + mem::size_of::<Pubkey>();
//             let data_addr = data_cell_addr + mem::size_of::<RcBox<RefCell<&mut [u8]>>>();
//
//             let info = AccountInfo {
//                 key: unsafe { (key_addr as *const Pubkey).as_ref() }.unwrap(),
//                 is_signer: self.is_signer,
//                 is_writable: self.is_writable,
//                 lamports: unsafe {
//                     Rc::from_raw((lamports_cell_addr + RcBox::<&mut u64>::VALUE_OFFSET) as *const _)
//                 },
//                 data: unsafe {
//                     Rc::from_raw((data_cell_addr + RcBox::<&mut [u8]>::VALUE_OFFSET) as *const _)
//                 },
//                 owner: unsafe { (owner_addr as *const Pubkey).as_ref() }.unwrap(),
//                 executable: self.executable,
//                 rent_epoch: self.rent_epoch,
//             };
//
//             unsafe {
//                 ptr::write_unaligned(data.as_mut_ptr().cast(), info);
//                 ptr::write_unaligned(
//                     (data.as_mut_ptr() as usize + key_addr - vm_addr) as *mut _,
//                     self.key,
//                 );
//                 ptr::write_unaligned(
//                     (data.as_mut_ptr() as usize + lamports_cell_addr - vm_addr) as *mut _,
//                     RcBox::new(RefCell::new((lamports_addr as *mut u64).as_mut().unwrap())),
//                 );
//                 ptr::write_unaligned(
//                     (data.as_mut_ptr() as usize + lamports_addr - vm_addr) as *mut _,
//                     self.lamports,
//                 );
//                 ptr::write_unaligned(
//                     (data.as_mut_ptr() as usize + owner_addr - vm_addr) as *mut _,
//                     self.owner,
//                 );
//                 ptr::write_unaligned(
//                     (data.as_mut_ptr() as usize + data_cell_addr - vm_addr) as *mut _,
//                     RcBox::new(RefCell::new(slice::from_raw_parts_mut(
//                         data_addr as *mut u8,
//                         self.data.len(),
//                     ))),
//                 );
//                 data[data_addr - vm_addr..].copy_from_slice(self.data);
//             }
//
//             let region = MemoryRegion::new_writable(data.as_mut_slice(), vm_addr as u64);
//             (
//                 data,
//                 region,
//                 SerializedAccountMetadata {
//                     original_data_len: self.data.len(),
//                     vm_key_addr: key_addr as u64,
//                     vm_lamports_addr: lamports_addr as u64,
//                     vm_owner_addr: owner_addr as u64,
//                     vm_data_addr: data_addr as u64,
//                 },
//             )
//         }
//     }
//
//     #[repr(C)]
//     struct RcBox<T> {
//         strong: Cell<usize>,
//         weak: Cell<usize>,
//         value: T,
//     }
//
//     impl<T> RcBox<T> {
//         const VALUE_OFFSET: usize = mem::size_of::<Cell<usize>>() * 2;
//         fn new(value: T) -> RcBox<T> {
//             RcBox {
//                 strong: Cell::new(0),
//                 weak: Cell::new(0),
//                 value,
//             }
//         }
//     }
//
//     fn is_zeroed(data: &[u8]) -> bool {
//         data.iter().all(|b| *b == 0)
//     }
// }
