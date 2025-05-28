use crate::{
    context::InvokeContext,
    error::{Error, SvmError},
    helpers::SyscallError,
};
use alloc::vec::Vec;
use core::{any::type_name, str::from_utf8};
use fluentbase_sdk::debug_log;
use fluentbase_types::SharedAPI;
use solana_pubkey::{Pubkey, PubkeyError, MAX_SEEDS, MAX_SEED_LEN};
use solana_rbpf::memory_region::{AccessType, MemoryMapping};
pub fn translate(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    len: u64,
) -> crate::helpers::StdResult<u64, SvmError> {
    let result = memory_mapping
        .map(access_type, vm_addr, len)
        .map_err(|err| err.into())
        .into();
    result
}

fn translate_type_inner<'a, T>(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a mut T, SvmError> {
    let host_addr = translate(memory_mapping, access_type, vm_addr, size_of::<T>() as u64)?;
    if !check_aligned {
        #[cfg(target_pointer_width = "64")]
        {
            Ok(unsafe { core::mem::transmute::<u64, &mut T>(host_addr) })
        }
        #[cfg(target_pointer_width = "32")]
        {
            Ok(unsafe { core::mem::transmute::<u32, &mut T>(host_addr as u32) })
        }
    } else if !crate::helpers::address_is_aligned::<T>(host_addr) {
        // Err(EbpfError::SyscallError::UnalignedPointer.into())
        Err(SyscallError::UnalignedPointer.into())
    } else {
        Ok(unsafe { &mut *(host_addr as *mut T) })
    }
}
pub fn translate_type_mut<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a mut T, SvmError> {
    translate_type_inner::<T>(memory_mapping, AccessType::Store, vm_addr, check_aligned)
}
pub fn translate_type<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a T, SvmError> {
    translate_type_inner::<T>(memory_mapping, AccessType::Load, vm_addr, check_aligned)
        .map(|value| &*value)
}

fn translate_slice_inner<'a, T>(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<&'a mut [T], SvmError> {
    if len == 0 {
        return Ok(&mut []);
    }
    let type_name = type_name::<T>();
    let size_of_t = size_of::<T>();
    debug_log!(
        "translate_slice_inner 1: len {} item type '{}' size_of_t {}",
        len,
        type_name,
        size_of_t,
    );

    let total_size = len.saturating_mul(size_of_t as u64);
    if isize::try_from(total_size).is_err() {
        return Err(SyscallError::InvalidLength.into());
    }

    debug_log!(
        "translate_slice_inner 2: access_type {:?} vm_addr {} total_size {}",
        access_type,
        vm_addr,
        total_size
    );

    let host_addr = translate(memory_mapping, access_type, vm_addr, total_size)?;
    debug_log!(
        "translate_slice_inner 3: vm_addr {} host_addr {} ({} in GB)",
        vm_addr,
        host_addr,
        host_addr / (1024 * 1024 * 1024)
    );

    if check_aligned && !crate::helpers::address_is_aligned::<T>(host_addr) {
        return Err(SyscallError::UnalignedPointer.into());
    }
    debug_log!("translate_slice_inner 4");
    let result = unsafe { core::slice::from_raw_parts_mut(host_addr as *mut T, len as usize) };
    Ok(result)
}

pub fn translate_slice<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<&'a [T], SvmError> {
    translate_slice_inner::<T>(
        memory_mapping,
        AccessType::Load,
        vm_addr,
        len,
        check_aligned,
    )
    .map(|value| &*value)
}

pub fn translate_slice_mut<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<&'a mut [T], SvmError> {
    translate_slice_inner::<T>(
        memory_mapping,
        AccessType::Store,
        vm_addr,
        len,
        check_aligned,
    )
}

/// Take a virtual pointer to a string (points to SBF VM memory space), translate it
/// pass it to a user-defined work function
pub fn translate_string_and_do(
    memory_mapping: &MemoryMapping,
    addr: u64,
    len: u64,
    check_aligned: bool,
    work: &mut dyn FnMut(&str) -> Result<u64, Error>,
) -> Result<u64, Error> {
    let buf = translate_slice::<u8>(memory_mapping, addr, len, check_aligned)?;
    match from_utf8(buf) {
        Ok(message) => work(message),
        Err(err) => Err(SyscallError::InvalidString(err, buf.to_vec()).into()),
    }
}

/// Check that two regions do not overlap.
///
/// Hidden to share with bpf_loader without being part of the API surface.
#[doc(hidden)]
pub fn is_nonoverlapping<N>(src: N, src_len: N, dst: N, dst_len: N) -> bool
where
    N: Ord + num_traits::SaturatingSub,
{
    // If the absolute distance between the ptrs is at least as big as the size of the other,
    // they do not overlap.
    if src > dst {
        src.saturating_sub(&dst) >= dst_len
    } else {
        dst.saturating_sub(&src) >= src_len
    }
}

pub fn memmove<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
    dst_addr: u64,
    src_addr: u64,
    n: u64,
    memory_mapping: &MemoryMapping,
) -> Result<u64, Error> {
    // if invoke_context
    //     .feature_set
    //     .is_active(&feature_set::bpf_account_data_direct_mapping::id())
    // {
    //     memmove_non_contiguous(dst_addr, src_addr, n, memory_mapping)
    // } else {
    let dst_ptr = translate_slice_mut::<u8>(
        memory_mapping,
        dst_addr,
        n,
        // invoke_context.get_check_aligned(),
        true,
    )?
    .as_mut_ptr();
    let src_ptr = translate_slice::<u8>(
        memory_mapping,
        src_addr,
        n,
        // invoke_context.get_check_aligned(),
        true,
    )?
    .as_ptr();

    unsafe { core::ptr::copy(src_ptr, dst_ptr, n as usize) };
    Ok(0)
    // }
}

pub unsafe fn memcmp(s1: &[u8], s2: &[u8], n: usize) -> i32 {
    for i in 0..n {
        let a = *s1.get_unchecked(i);
        let b = *s2.get_unchecked(i);
        if a != b {
            return (a as i32).saturating_sub(b as i32);
        };
    }

    0
}

pub fn translate_and_check_program_address_inputs<'a>(
    seeds_addr: u64,
    seeds_len: u64,
    program_id_addr: u64,
    memory_mapping: &mut MemoryMapping,
    check_aligned: bool,
) -> Result<(Vec<&'a [u8]>, &'a Pubkey), SvmError> {
    let untranslated_seeds =
        translate_slice::<&[u8]>(memory_mapping, seeds_addr, seeds_len, check_aligned)?;
    debug_log!(
        "translate_and_check_program_address_inputs 1: seeds_addr {} seeds_len {} untranslated_seeds.len {}",
        seeds_addr,
        seeds_len,
        untranslated_seeds.len(),
    );
    for (idx, us) in untranslated_seeds.iter().enumerate() {
        debug_log!("untranslated_seed {}: len {}", idx, us.len());
    }
    if untranslated_seeds.len() > MAX_SEEDS {
        return Err(SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded).into());
    }
    let seeds = untranslated_seeds
        .iter()
        .map(|untranslated_seed| {
            if untranslated_seed.len() > MAX_SEED_LEN {
                return Err(SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded).into());
            }
            // debug_log!(
            //     "untranslated_seed: {:x?} ptr {}",
            //     untranslated_seed,
            //     untranslated_seed.as_ptr() as u64
            // );
            translate_slice::<u8>(
                memory_mapping,
                untranslated_seed.as_ptr() as *const _ as u64,
                untranslated_seed.len() as u64,
                check_aligned,
            )
        })
        .collect::<Result<Vec<_>, SvmError>>()?;
    debug_log!("translate_and_check_program_address_inputs 2");
    let program_id = translate_type::<Pubkey>(memory_mapping, program_id_addr, check_aligned)?;
    debug_log!("translate_and_check_program_address_inputs 3");
    Ok((seeds, program_id))
}
