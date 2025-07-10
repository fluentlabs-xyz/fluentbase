use super::*;
use crate::{
    account::BorrowedAccount,
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
    native_loader,
    precompiles::is_precompile,
    serialization::account_data_region_memory_state,
    word_size::{
        addr_type::AddrType,
        common::{MemoryMappingHelper, STABLE_VEC_FAT_PTR64_BYTE_SIZE},
        primitives::RcRefCellMemLayout,
        ptr_type::PtrType,
        slice::{reconstruct_slice, SliceFatPtr64, SliceFatPtr64Repr, SpecMethods},
    },
};
use alloc::{boxed::Box, vec, vec::Vec};
use core::{fmt::Debug, marker::PhantomData, ptr};
use fluentbase_sdk::SharedAPI;
use scopeguard::defer;
use solana_account_info::{AccountInfo, MAX_PERMITTED_DATA_INCREASE};
use solana_instruction::{error::InstructionError, AccountMeta};
use solana_program_entrypoint::{BPF_ALIGN_OF_U128, SUCCESS};
use solana_pubkey::{Pubkey, MAX_SEEDS, PUBKEY_BYTES};
use solana_rbpf::{
    ebpf,
    memory_region::{MemoryMapping, MemoryRegion, MemoryState},
};
use solana_stable_layout::stable_instruction::StableInstruction;

fn check_account_info_pointer<SDK: SharedAPI>(
    _invoke_context: &InvokeContext<SDK>,
    vm_addr: u64,
    expected_vm_addr: u64,
    _field: &str,
) -> Result<(), SvmError> {
    if vm_addr != expected_vm_addr {
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
    // Once direct mapping is activated, this variant can be removed and the enum can be made a struct.
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
pub struct CallerAccount<'a, 'b, SDK: SharedAPI> {
    lamports: &'a mut u64,
    owner: &'a mut Pubkey,
    // The original data length of the account at the start of the current
    // instruction. We use this to determine wether an account was shrunk or
    // grown before or after CPI, and to derive the vm address of the realloc
    // region.
    original_data_len: usize,
    // This points to the data section for this account, as serialized and
    // mapped inside the vm (see serialize_parameters() in BpfExecutor::execute).
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
        account_info: SliceFatPtr64<'a, AccountInfo<'b>>,
        account_metadata: &SerializedAccountMetadata,
    ) -> Result<CallerAccount<'a, 'b, SDK>, SvmError> {
        let direct_mapping = invoke_context
            .get_feature_set()
            .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id());

        let account_info_first_item_addr = account_info.first_item_addr().inner();
        let addr_to_key_addr = account_info_first_item_addr;
        let addr_to_lamports_rc_addr = account_info_first_item_addr.saturating_add(8);
        let addr_to_data_addr = account_info_first_item_addr.saturating_add(8 * 2);
        let addr_to_owner_addr = account_info_first_item_addr.saturating_add(8 * 3);

        let mmh: MemoryMappingHelper = memory_mapping.into();

        if direct_mapping {
            check_account_info_pointer(
                invoke_context,
                addr_to_key_addr,
                account_metadata.vm_key_addr,
                "key",
            )?;

            check_account_info_pointer(
                invoke_context,
                addr_to_owner_addr,
                account_metadata.vm_owner_addr,
                "owner",
            )?;
        }

        // account_info points to host memory. The addresses used internally are
        // in vm space so they need to be translated.
        let lamports = {
            // Double translate lamports out of RefCell
            let lamports_mem_layout_ptr = RcRefCellMemLayout::<&mut u64>::new(
                mmh.clone(),
                PtrType::RcStartPtr(addr_to_lamports_rc_addr),
            );

            if direct_mapping {
                if addr_to_lamports_rc_addr >= ebpf::MM_INPUT_START {
                    return Err(SyscallError::InvalidPointer.into());
                }

                check_account_info_pointer(
                    invoke_context,
                    lamports_mem_layout_ptr.value_addr::<false, false>(),
                    account_metadata.vm_lamports_addr,
                    "lamports",
                )?;
            }
            translate_type_mut::<u64>(
                memory_mapping,
                lamports_mem_layout_ptr.value_addr::<false, false>(),
                invoke_context.get_check_aligned(),
                false,
            )?
        };

        let owner_addr = SliceFatPtr64Repr::ptr_elem_from_addr(addr_to_owner_addr);
        let owner = translate_type_mut::<Pubkey>(
            memory_mapping,
            owner_addr,
            invoke_context.get_check_aligned(),
            false,
        )?;

        let (serialized_data, vm_data_addr, ref_to_len_in_vm) = {
            if direct_mapping && addr_to_data_addr >= ebpf::MM_INPUT_START {
                return Err(SyscallError::InvalidPointer.into());
            }

            // Double translate data out of RefCell
            let data_mem_layout = RcRefCellMemLayout::<&mut [u8]>::new(
                mmh.clone(),
                PtrType::RcStartPtr(addr_to_data_addr),
            );
            let data_fat_ptr_vm_addr = data_mem_layout.addr_to_value_addr::<false, false>();
            let data_fat_ptr_addr = data_mem_layout.addr_to_value_addr::<false, true>();
            let data =
                SliceFatPtr64::<u8>::from_ptr_to_fat_ptr(data_fat_ptr_addr as usize, mmh.clone());

            if direct_mapping {
                check_account_info_pointer(
                    invoke_context,
                    data.first_item_addr().inner(),
                    account_metadata.vm_data_addr,
                    "data",
                )?;
            }

            let ref_to_len_in_vm = if direct_mapping {
                // let vm_addr = (account_info.data.as_ptr() as *const u64 as u64)
                //     .saturating_add(size_of::<u64>() as u64);
                let data_len_fat_ptr_vm_addr = data
                    .first_item_addr()
                    .inner()
                    .saturating_add(size_of::<u64>() as u64);
                let vm_addr = data_len_fat_ptr_vm_addr;
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
                let data_len_fat_ptr_addr =
                    data_fat_ptr_addr.saturating_add(size_of::<u64>() as u64);
                let translated = data_len_fat_ptr_addr as *mut u64;

                let val = unsafe { &mut *translated };
                VmValue::Translated(val)
            };
            let vm_data_addr = data.first_item_addr();

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
                    data_fat_ptr_vm_addr,
                    data.len() as u64,
                    invoke_context.get_check_aligned(),
                )?
            };

            (serialized_data, vm_data_addr.inner(), ref_to_len_in_vm)
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
    // fn from_sol_account_info(
    //     invoke_context: &InvokeContext<SDK>,
    //     memory_mapping: &'b MemoryMapping<'a>,
    //     vm_addr: u64,
    //     account_info: &SolAccountInfo,
    //     account_metadata: &SerializedAccountMetadata,
    // ) -> Result<CallerAccount<'a, 'b, SDK>, Error> {
    //     let direct_mapping = invoke_context
    //         .get_feature_set()
    //         .is_active(&feature_set::bpf_account_data_direct_mapping::id());
    //
    //     if direct_mapping {
    //         check_account_info_pointer(
    //             invoke_context,
    //             account_info.key_addr,
    //             account_metadata.vm_key_addr,
    //             "key",
    //         )?;
    //
    //         check_account_info_pointer(
    //             invoke_context,
    //             account_info.owner_addr,
    //             account_metadata.vm_owner_addr,
    //             "owner",
    //         )?;
    //
    //         check_account_info_pointer(
    //             invoke_context,
    //             account_info.lamports_addr,
    //             account_metadata.vm_lamports_addr,
    //             "lamports",
    //         )?;
    //
    //         check_account_info_pointer(
    //             invoke_context,
    //             account_info.data_addr,
    //             account_metadata.vm_data_addr,
    //             "data",
    //         )?;
    //     }
    //
    //     // account_info points to host memory. The addresses used internally are
    //     // in vm space so they need to be translated.
    //     let lamports = translate_type_mut::<u64>(
    //         memory_mapping,
    //         account_info.lamports_addr,
    //         invoke_context.get_check_aligned(),
    //         false,
    //     )?;
    //     let owner = translate_type_mut::<Pubkey>(
    //         memory_mapping,
    //         account_info.owner_addr,
    //         invoke_context.get_check_aligned(),
    //         false,
    //     )?;
    //
    //     let serialized_data = if direct_mapping {
    //         // See comment in CallerAccount::from_account_info()
    //         Default::default()
    //     } else {
    //         translate_slice_mut::<u8>(
    //             memory_mapping,
    //             account_info.data_addr,
    //             account_info.data_len,
    //             invoke_context.get_check_aligned(),
    //         )?
    //     };
    //
    //     // we already have the host addr we want: &mut account_info.data_len.
    //     // The account info might be read only in the vm though, so we translate
    //     // to ensure we can write. This is tested by programs/sbf/rust/ro_modify
    //     // which puts SolAccountInfo in rodata.
    //     let data_len_vm_addr = vm_addr
    //         .saturating_add(&account_info.data_len as *const u64 as u64)
    //         .saturating_sub(account_info as *const _ as *const u64 as u64);
    //
    //     let ref_to_len_in_vm = if direct_mapping {
    //         VmValue::VmAddress {
    //             vm_addr: data_len_vm_addr,
    //             memory_mapping,
    //             check_aligned: invoke_context.get_check_aligned(),
    //         }
    //     } else {
    //         let data_len_addr = translate(
    //             memory_mapping,
    //             AccessType::Store,
    //             data_len_vm_addr,
    //             size_of::<u64>() as u64,
    //         )?;
    //         VmValue::Translated(unsafe { &mut *(data_len_addr as *mut u64) })
    //     };
    //
    //     Ok(CallerAccount {
    //         lamports,
    //         owner,
    //         original_data_len: account_metadata.original_data_len,
    //         serialized_data,
    //         vm_data_addr: account_info.data_addr,
    //         ref_to_len_in_vm,
    //         _phantom_data: Default::default(),
    //     })
    // }

    fn realloc_region(
        &self,
        memory_mapping: &'b MemoryMapping<'_>,
    ) -> Result<Option<&'a MemoryRegion>, Error> {
        account_realloc_region(memory_mapping, self.vm_data_addr, self.original_data_len)
    }
}

type TranslatedAccounts<'a, 'b, SDK> = Vec<(IndexOfAccount, Option<CallerAccount<'a, 'b, SDK>>)>;

/// Implemented by language specific data structure translators
pub trait SyscallInvokeSigned<SDK: SharedAPI> {
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

impl<SDK: SharedAPI> SyscallInvokeSigned<SDK> for SyscallInvokeSignedRust {
    fn translate_instruction(
        addr: u64,
        memory_mapping: &MemoryMapping,
        invoke_context: &mut InvokeContext<SDK>,
    ) -> Result<StableInstruction, SvmError> {
        const STABLE_INSTRUCTION_BYTES_SIZE: u64 = 24 * 2 + 32; // StableVec * 2 + Pubkey
        let host_addr = translate(
            memory_mapping,
            AccessType::Load,
            addr,
            STABLE_INSTRUCTION_BYTES_SIZE,
        )?;
        let accounts_ptr = SliceFatPtr64Repr::from_ptr_to_fixed_slice_fat_ptr(host_addr as usize);
        let data_ptr = SliceFatPtr64Repr::from_ptr_to_fixed_slice_fat_ptr(
            host_addr as usize + STABLE_VEC_FAT_PTR64_BYTE_SIZE,
        );
        let program_id_addr = host_addr as usize + STABLE_VEC_FAT_PTR64_BYTE_SIZE * 2;
        let program_id_data = reconstruct_slice::<u8>(program_id_addr, PUBKEY_BYTES);
        let program_id = Pubkey::new_from_array(program_id_data.try_into().unwrap());
        let account_metas = translate_slice::<AccountMeta>(
            memory_mapping,
            accounts_ptr.first_item_addr().inner(),
            accounts_ptr.len() as u64,
            invoke_context.get_check_aligned(),
        )?;
        let data = translate_slice::<u8>(
            memory_mapping,
            data_ptr.first_item_addr().inner(),
            data_ptr.len() as u64,
            invoke_context.get_check_aligned(),
        )?;

        check_instruction_size(account_metas.len(), data.len(), invoke_context)?;

        let mut accounts: Vec<AccountMeta> = Vec::with_capacity(account_metas.len());
        for account_index in 0..account_metas.len() {
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
        memory_mapping: &'b MemoryMapping<'a>,
        invoke_context: &mut InvokeContext<SDK>,
    ) -> Result<TranslatedAccounts<'a, 'b, SDK>, SvmError> {
        let (account_infos, account_info_keys) = translate_account_infos(
            account_infos_addr,
            account_infos_len,
            // |account_info: &AccountInfo| account_info.key as *const _ as u64,
            |account_infos: SliceFatPtr64<AccountInfo>, idx: usize, total_len: usize| {
                let key_addr = account_infos.item_addr_at_idx(idx);
                let addr = SliceFatPtr64Repr::ptr_elem_from_addr(key_addr.inner());
                SliceFatPtr64Repr::map_vm_addr_to_host(memory_mapping, addr, total_len as u64, None)
                    .unwrap()
            },
            memory_mapping,
            invoke_context,
        )?;

        translate_and_update_accounts(
            instruction_accounts,
            program_indices,
            &account_info_keys,
            account_infos,
            account_infos_addr,
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
                let untranslated_seeds = signer_seeds.as_ref();
                if untranslated_seeds.len() > MAX_SEEDS {
                    return Err(InstructionError::MaxSeedLengthExceeded.into());
                }
                let seeds = untranslated_seeds
                    .iter()
                    .map(|untranslated_seed| untranslated_seed.as_ref().to_vec_cloned())
                    .collect::<Vec<_>>();
                let signer = Pubkey::create_program_address(
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

// /// Rust representation of C's SolAccountInfo
// #[derive(Debug)]
// #[repr(C)]
// struct SolAccountInfo {
//     key_addr: u64,
//     lamports_addr: u64,
//     data_len: u64,
//     data_addr: u64,
//     owner_addr: u64,
//     rent_epoch: u64,
//     is_signer: bool,
//     is_writable: bool,
//     executable: bool,
// }

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

fn translate_account_infos<'a, T: Clone + SpecMethods<'a> + Debug + 'a, F, SDK: SharedAPI>(
    account_infos_addr: u64,
    account_infos_len: u64,
    key_addr: F,
    memory_mapping: &'a MemoryMapping,
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(SliceFatPtr64<'a, T>, Vec<Pubkey>), SvmError>
where
    F: Fn(SliceFatPtr64<'a, T>, usize, usize) -> u64,
{
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id());

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

    let mmh: MemoryMappingHelper = memory_mapping.into();
    crate::remap_addr!(mmh, account_infos_addr);
    let account_infos = SliceFatPtr64::new(
        mmh,
        AddrType::new_vm(account_infos_addr),
        account_infos_len as usize,
    );
    check_account_infos(account_infos.len(), invoke_context)?;
    let mut account_info_keys = Vec::with_capacity(account_infos_len as usize);
    for account_index in 0..account_infos_len as usize {
        let addr = key_addr(account_infos.clone(), account_index, PUBKEY_BYTES);
        let pubkey_data = reconstruct_slice::<u8>(addr as usize, PUBKEY_BYTES);
        let pk = Pubkey::new_from_array(pubkey_data.try_into().unwrap());
        account_info_keys.push(pk);
    }
    Ok((account_infos, account_info_keys))
}

// Finish translating accounts, build CallerAccount values and update callee
// accounts in preparation of executing the callee.
fn translate_and_update_accounts<
    'a,
    'b,
    T: Clone + SpecMethods<'a> + Debug + 'a,
    F,
    SDK: SharedAPI,
>(
    instruction_accounts: &[InstructionAccount],
    program_indices: &[IndexOfAccount],
    account_info_keys: &[Pubkey],
    account_infos: SliceFatPtr64<'a, T>,
    _vm_addr: u64,
    invoke_context: &mut InvokeContext<SDK>,
    memory_mapping: &'b MemoryMapping<'a>,
    do_translate: F,
) -> Result<TranslatedAccounts<'a, 'b, SDK>, SvmError>
where
    F: Fn(
        &InvokeContext<SDK>,
        &'b MemoryMapping<'a>,
        u64,
        SliceFatPtr64<'a, T>,
        &SerializedAccountMetadata,
    ) -> Result<CallerAccount<'a, 'b, SDK>, SvmError>,
{
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut accounts = Vec::with_capacity(instruction_accounts.len().saturating_add(1));

    let program_account_index = program_indices
        .last()
        .ok_or_else(|| InstructionError::MissingAccount)?;
    accounts.push((*program_account_index, None));

    // unwrapping here is fine: we're in a syscall and the method below fails
    // only outside syscalls
    let accounts_metadata = &invoke_context.get_syscall_context()?.accounts_metadata;

    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id());

    for (instruction_account_index, instruction_account) in instruction_accounts.iter().enumerate()
    {
        if instruction_account_index as IndexOfAccount != instruction_account.index_in_callee {
            continue; // Skip duplicate account
        }

        let callee_account = instruction_context.try_borrow_instruction_account(
            transaction_context,
            instruction_account.index_in_caller,
        )?;
        let account_key = invoke_context
            .transaction_context
            .get_key_of_account_at_index(instruction_account.index_in_transaction)?;

        if callee_account.is_executable() {
            // Use the known account
            accounts.push((instruction_account.index_in_caller, None));
        } else if let Some(caller_account_index) =
            account_info_keys.iter().position(|key| key == account_key)
        {
            let serialized_metadata = accounts_metadata
                .get(instruction_account.index_in_caller as usize)
                .ok_or_else(|| InstructionError::MissingAccount)?;

            // build the CallerAccount corresponding to this account.
            if caller_account_index >= account_infos.len() {
                return Err(SyscallError::InvalidLength.into());
            }
            let account_info = account_infos
                .clone_from_index(caller_account_index)
                .expect("caller account doesnt exist at the index");
            #[cfg(test)]
            if caller_account_index == 0 {
                if account_info.first_item_addr() != account_infos.first_item_addr() {
                    panic!("addresses must match")
                }
            }
            let caller_account = do_translate(
                invoke_context,
                memory_mapping,
                account_infos.item_addr_at_idx(caller_account_index).inner(),
                account_info,
                serialized_metadata,
            )?;

            // before initiating CPI, the caller may have modified the
            // account (caller_account). We need to update the corresponding
            // BorrowedAccount (callee_account) so the callee can see the changes.
            let update_caller = update_callee_account(
                invoke_context,
                memory_mapping,
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
        .is_active(&solana_feature_set::loosen_cpi_size_restriction::id())
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
        .is_active(&solana_feature_set::loosen_cpi_size_restriction::id())
    {
        let max_cpi_account_infos = if invoke_context
            .get_feature_set()
            .is_active(&solana_feature_set::increase_tx_account_lock_limit::id())
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
    _instruction_data: &[u8],
    invoke_context: &InvokeContext<SDK>,
) -> Result<(), Error> {
    if native_loader::check_id(program_id)
        // || bpf_loader::check_id(program_id)
        || is_precompile(program_id, |feature_id: &Pubkey| {
            invoke_context.get_feature_set().is_active(feature_id)
        })
    {
        return Err(Box::new(SyscallError::ProgramNotSupported(*program_id)));
    }
    Ok(())
}

/// Call process instruction, common to both Rust and C
pub fn cpi_common<SDK: SharedAPI, S: SyscallInvokeSigned<SDK>>(
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
    let instruction: StableInstruction =
        S::translate_instruction(instruction_addr, memory_mapping, invoke_context)?;
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let caller_program_id = instruction_context.get_last_program_key(transaction_context)?;
    let signers = S::translate_signers(
        caller_program_id,
        signers_seeds_addr,
        signers_seeds_len,
        memory_mapping,
        invoke_context,
    )?;
    let (instruction_accounts, program_indices): (Vec<InstructionAccount>, Vec<IndexOfAccount>) =
        invoke_context.prepare_instruction(&instruction, &signers)?;
    check_authorized_program(&instruction.program_id, &instruction.data, invoke_context)?;

    let mut accounts: TranslatedAccounts<SDK> = S::translate_accounts(
        &instruction_accounts,
        &program_indices,
        account_infos_addr,
        account_infos_len,
        memory_mapping,
        invoke_context,
    )?;

    // Process the callee instruction
    // let mut compute_units_consumed = 0;
    invoke_context.process_instruction(
        &instruction.data,
        &instruction_accounts,
        &program_indices,
    )?;

    // re-bind to please the borrow checker
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;

    // CPI exit.
    //
    // Synchronize the callee's account changes so the caller can see them.
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&solana_feature_set::bpf_account_data_direct_mapping::id());

    if direct_mapping {
        // Update all perms at once before doing account data updates. This
        // isn't strictly required as we forbid updates to an account to touch
        // other accounts, but since we did have bugs around this in the past,
        // it's better to be safe than sorry.
        for (index_in_caller, caller_account) in accounts.iter() {
            if let Some(caller_account) = caller_account {
                let callee_account = instruction_context
                    .try_borrow_instruction_account(transaction_context, *index_in_caller)?;
                update_caller_account_perms(memory_mapping, caller_account, &callee_account)?;
            }
        }
    }

    for (index_in_caller, caller_account) in accounts.iter_mut() {
        if let Some(caller_account) = caller_account {
            let mut callee_account = instruction_context
                .try_borrow_instruction_account(transaction_context, *index_in_caller)?;
            update_caller_account(
                invoke_context,
                memory_mapping,
                caller_account,
                &mut callee_account,
                direct_mapping,
            )?;
        }
    }

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
    caller_account: &CallerAccount<SDK>,
    mut callee_account: BorrowedAccount<'_>,
    direct_mapping: bool,
) -> Result<bool, SvmError> {
    let mut must_update_caller = false;

    if callee_account.get_lamports() != *caller_account.lamports {
        callee_account.set_lamports(*caller_account.lamports)?;
    }

    // TODO this flag must be always false so maybe we can get rid of the other branch?
    if direct_mapping {
        let prev_len = callee_account.get_data().len();
        let post_len = *caller_account.ref_to_len_in_vm.get()? as usize;
        match callee_account
            .can_data_be_resized(post_len)
            .and_then(|_| callee_account.can_data_be_changed())
        {
            Ok(()) => {
                let realloc_bytes_used = post_len.saturating_sub(caller_account.original_data_len);
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
    let callee_account_owner = callee_account.get_owner();
    if callee_account_owner != caller_account.owner {
        callee_account.set_owner(caller_account.owner.as_ref())?;
    }

    Ok(must_update_caller)
}

fn update_caller_account_perms<SDK: SharedAPI>(
    memory_mapping: &MemoryMapping,
    caller_account: &CallerAccount<SDK>,
    callee_account: &BorrowedAccount<'_>,
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
    let realloc_region = account_realloc_region(memory_mapping, *vm_data_addr, *original_data_len)?;
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
                    // Temporarily configure the realloc region as writable then set it back to
                    // whatever state it had.
                    let realloc_region = caller_account.realloc_region(memory_mapping)?.unwrap(); // unwrapping here is fine, we already asserted !is_loader_deprecated
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
                    .map_err(|_e| InstructionError::AccountDataTooSmall)?
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
            // memory, it could be a source of non-determinism.
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
            let mut to_slice = {
                // If a callee reallocs an account, we write into the caller's
                // realloc region regardless of whether the caller has write
                // permissions to the account or not. If the callee has been able to
                // make changes, it means they had permissions to do so, and here
                // we're just going to reflect those changes to the caller's frame.
                //
                // Therefore we temporarily configure the realloc region as writable
                // then set it back to whatever state it had.
                let realloc_region = caller_account.realloc_region(memory_mapping)?.unwrap(); // unwrapping here is fine, we asserted !is_loader_deprecated
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
) -> Result<Option<&'a MemoryRegion>, Error> {
    let realloc_vm_addr = vm_data_addr.saturating_add(original_data_len as u64);
    let realloc_region = memory_mapping.region(AccessType::Load, realloc_vm_addr)?;
    debug_assert_eq!(realloc_region.vm_addr, realloc_vm_addr);
    debug_assert!((MAX_PERMITTED_DATA_INCREASE
        ..MAX_PERMITTED_DATA_INCREASE.saturating_add(BPF_ALIGN_OF_U128))
        .contains(&(realloc_region.len as usize)));
    debug_assert!(!matches!(realloc_region.state.get(), MemoryState::Cow(_)));
    Ok(Some(realloc_region))
}
