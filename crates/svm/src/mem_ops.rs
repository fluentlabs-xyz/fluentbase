use super::*;
use crate::{
    context::InvokeContext,
    error::{Error, SvmError},
    helpers::SyscallError,
    word_size::{
        addr_type::AddrType,
        common::MemoryMappingHelper,
        slice::{SliceFatPtr64, SpecMethods},
    },
};
use alloc::{boxed::Box, vec::Vec};
use core::{fmt::Debug, slice, str::from_utf8};
use fluentbase_sdk::SharedAPI;
use solana_pubkey::{Pubkey, PubkeyError, MAX_SEEDS, MAX_SEED_LEN};
use solana_rbpf::{
    error::EbpfError,
    memory_region::{AccessType, MemoryMapping, MemoryRegion},
};

// declare_builtin_function!(
//     /// memcpy
//     SyscallMemcpy,
//     fn rust(
//         invoke_context: &mut InvokeContext,
//         dst_addr: u64,
//         src_addr: u64,
//         n: u64,
//         _arg4: u64,
//         _arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         mem_op_consume(invoke_context, n)?;
//
//         if !is_nonoverlapping(src_addr, n, dst_addr, n) {
//             return Err(SyscallError::CopyOverlapping.into());
//         }
//
//         // host addresses can overlap so we always invoke memmove
//         memmove(invoke_context, dst_addr, src_addr, n, memory_mapping)
//     }
// );
//
// declare_builtin_function!(
//     /// memmove
//     SyscallMemmove,
//     fn rust(
//         invoke_context: &mut InvokeContext,
//         dst_addr: u64,
//         src_addr: u64,
//         n: u64,
//         _arg4: u64,
//         _arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         mem_op_consume(invoke_context, n)?;
//
//         memmove(invoke_context, dst_addr, src_addr, n, memory_mapping)
//     }
// );
//
// declare_builtin_function!(
//     /// memcmp
//     SyscallMemcmp,
//     fn rust(
//         invoke_context: &mut InvokeContext,
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
//             .feature_set
//             .is_active(&feature_set::bpf_account_data_direct_mapping::id())
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
//             debug_assert_eq!(s1.len(), n as usize);
//             debug_assert_eq!(s2.len(), n as usize);
//             // Safety:
//             // memcmp is marked unsafe since it assumes that the inputs are at least
//             // `n` bytes long. `s1` and `s2` are guaranteed to be exactly `n` bytes
//             // long because `translate_slice` would have failed otherwise.
//             *cmp_result = unsafe { memcmp(s1, s2, n as usize) };
//         }
//
//         Ok(0)
//     }
// );
//
// declare_builtin_function!(
//     /// memset
//     SyscallMemset,
//     fn rust(
//         invoke_context: &mut InvokeContext,
//         dst_addr: u64,
//         c: u64,
//         n: u64,
//         _arg4: u64,
//         _arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         mem_op_consume(invoke_context, n)?;
//
//         if invoke_context
//             .feature_set
//             .is_active(&feature_set::bpf_account_data_direct_mapping::id())
//         {
//             memset_non_contiguous(dst_addr, c as u8, n, memory_mapping)
//         } else {
//             let s = translate_slice_mut::<u8>(
//                 memory_mapping,
//                 dst_addr,
//                 n,
//                 invoke_context.get_check_aligned(),
//             )?;
//             s.fill(c as u8);
//             Ok(0)
//         }
//     }
// );

// fn memmove(
//     invoke_context: &mut InvokeContext,
//     dst_addr: u64,
//     src_addr: u64,
//     n: u64,
//     memory_mapping: &MemoryMapping,
// ) -> Result<u64, Error> {
//     if invoke_context
//         .feature_set
//         .is_active(&feature_set::bpf_account_data_direct_mapping::id())
//     {
//         memmove_non_contiguous(dst_addr, src_addr, n, memory_mapping)
//     } else {
//         let dst_ptr = translate_slice_mut::<u8>(
//             memory_mapping,
//             dst_addr,
//             n,
//             invoke_context.get_check_aligned(),
//         )?
//             .as_mut_ptr();
//         let src_ptr = translate_slice::<u8>(
//             memory_mapping,
//             src_addr,
//             n,
//             invoke_context.get_check_aligned(),
//         )?
//             .as_ptr();
//
//         unsafe { std::ptr::copy(src_ptr, dst_ptr, n as usize) };
//         Ok(0)
//     }
// }

// Marked unsafe since it assumes that the slices are at least `n` bytes long.
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

pub fn memcmp_non_contiguous(
    src_addr: u64,
    dst_addr: u64,
    n: u64,
    memory_mapping: &MemoryMapping,
) -> Result<i32, Error> {
    let memcmp_chunk = |s1_addr, s2_addr, chunk_len| {
        let res = unsafe {
            let s1 = slice::from_raw_parts(s1_addr, chunk_len);
            let s2 = slice::from_raw_parts(s2_addr, chunk_len);
            // Safety:
            // memcmp is marked unsafe since it assumes that s1 and s2 are exactly chunk_len
            // long. The whole point of iter_memory_pair_chunks is to find same length chunks
            // across two memory regions.
            memcmp(s1, s2, chunk_len)
        };
        if res != 0 {
            return Err(MemcmpError::Diff(res).into());
        }
        Ok(0)
    };
    match iter_memory_pair_chunks(
        AccessType::Load,
        src_addr,
        AccessType::Load,
        dst_addr,
        n,
        memory_mapping,
        false,
        memcmp_chunk,
    ) {
        Ok(res) => Ok(res),
        Err(error) => match error.downcast_ref() {
            Some(MemcmpError::Diff(diff)) => Ok(*diff),
            _ => Err(error),
        },
    }
}

#[derive(Debug)]
enum MemcmpError {
    Diff(i32),
}

impl core::fmt::Display for MemcmpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MemcmpError::Diff(diff) => write!(f, "MemcmpError::Diff({diff})"),
        }
    }
}

impl core::error::Error for MemcmpError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            MemcmpError::Diff(_) => None,
        }
    }
}

fn iter_memory_pair_chunks<T, F>(
    src_access: AccessType,
    src_addr: u64,
    dst_access: AccessType,
    dst_addr: u64,
    n_bytes: u64,
    memory_mapping: &MemoryMapping,
    reverse: bool,
    mut fun: F,
) -> Result<T, Error>
where
    T: Default,
    F: FnMut(*const u8, *const u8, usize) -> Result<T, Error>,
{
    let mut src_chunk_iter =
        MemoryChunkIterator::new(memory_mapping, src_access, src_addr, n_bytes)
            .map_err(EbpfError::from)?;
    let mut dst_chunk_iter =
        MemoryChunkIterator::new(memory_mapping, dst_access, dst_addr, n_bytes)
            .map_err(EbpfError::from)?;

    let mut src_chunk = None;
    let mut dst_chunk = None;

    macro_rules! memory_chunk {
        ($chunk_iter:ident, $chunk:ident) => {
            if let Some($chunk) = &mut $chunk {
                // Keep processing the current chunk
                $chunk
            } else {
                // This is either the first call or we've processed all the bytes in the current
                // chunk. Move to the next one.
                let chunk = match if reverse {
                    $chunk_iter.next_back()
                } else {
                    $chunk_iter.next()
                } {
                    Some(item) => item?,
                    None => break,
                };
                $chunk.insert(chunk)
            }
        };
    }

    loop {
        let (src_region, src_chunk_addr, src_remaining) = memory_chunk!(src_chunk_iter, src_chunk);
        let (dst_region, dst_chunk_addr, dst_remaining) = memory_chunk!(dst_chunk_iter, dst_chunk);

        // We always process same-length pairs
        let chunk_len = *src_remaining.min(dst_remaining);

        let (src_host_addr, dst_host_addr) = {
            let (src_addr, dst_addr) = if reverse {
                // When scanning backwards not only we want to scan regions from the end,
                // we want to process the memory within regions backwards as well.
                (
                    src_chunk_addr
                        .saturating_add(*src_remaining as u64)
                        .saturating_sub(chunk_len as u64),
                    dst_chunk_addr
                        .saturating_add(*dst_remaining as u64)
                        .saturating_sub(chunk_len as u64),
                )
            } else {
                (*src_chunk_addr, *dst_chunk_addr)
            };

            (
                Result::from(src_region.vm_to_host(src_addr, chunk_len as u64))?,
                Result::from(dst_region.vm_to_host(dst_addr, chunk_len as u64))?,
            )
        };

        fun(
            src_host_addr as *const u8,
            dst_host_addr as *const u8,
            chunk_len,
        )?;

        // Update how many bytes we have left to scan in each chunk
        *src_remaining = src_remaining.saturating_sub(chunk_len);
        *dst_remaining = dst_remaining.saturating_sub(chunk_len);

        if !reverse {
            // We've scanned `chunk_len` bytes so we move the vm address forward. In reverse
            // mode we don't do this since we make progress by decreasing src_len and
            // dst_len.
            *src_chunk_addr = src_chunk_addr.saturating_add(chunk_len as u64);
            *dst_chunk_addr = dst_chunk_addr.saturating_add(chunk_len as u64);
        }

        if *src_remaining == 0 {
            src_chunk = None;
        }

        if *dst_remaining == 0 {
            dst_chunk = None;
        }
    }

    Ok(T::default())
}

struct MemoryChunkIterator<'a> {
    memory_mapping: &'a MemoryMapping<'a>,
    access_type: AccessType,
    initial_vm_addr: u64,
    vm_addr_start: u64,
    // exclusive end index (start + len, so one past the last valid address)
    vm_addr_end: u64,
    len: u64,
}

impl<'a> MemoryChunkIterator<'a> {
    fn new(
        memory_mapping: &'a MemoryMapping,
        access_type: AccessType,
        vm_addr: u64,
        len: u64,
    ) -> Result<MemoryChunkIterator<'a>, EbpfError> {
        let vm_addr_end = vm_addr.checked_add(len).ok_or(EbpfError::AccessViolation(
            access_type,
            vm_addr,
            len,
            "unknown",
        ))?;
        Ok(MemoryChunkIterator {
            memory_mapping,
            access_type,
            initial_vm_addr: vm_addr,
            len,
            vm_addr_start: vm_addr,
            vm_addr_end,
        })
    }

    fn region(&mut self, vm_addr: u64) -> Result<&'a MemoryRegion, Error> {
        match self.memory_mapping.region(self.access_type, vm_addr) {
            Ok(region) => Ok(region),
            Err(error) => match error {
                EbpfError::AccessViolation(access_type, _vm_addr, _len, name) => Err(Box::new(
                    EbpfError::AccessViolation(access_type, self.initial_vm_addr, self.len, name),
                )),
                EbpfError::StackAccessViolation(access_type, _vm_addr, _len, frame) => {
                    Err(Box::new(EbpfError::StackAccessViolation(
                        access_type,
                        self.initial_vm_addr,
                        self.len,
                        frame,
                    )))
                }
                _ => Err(error.into()),
            },
        }
    }
}

impl<'a> Iterator for MemoryChunkIterator<'a> {
    type Item = Result<(&'a MemoryRegion, u64, usize), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.vm_addr_start == self.vm_addr_end {
            return None;
        }

        let region = match self.region(self.vm_addr_start) {
            Ok(region) => region,
            Err(e) => {
                self.vm_addr_start = self.vm_addr_end;
                return Some(Err(e));
            }
        };

        let vm_addr = self.vm_addr_start;

        let chunk_len = if region.vm_addr_end <= self.vm_addr_end {
            // consume the whole region
            let len = region.vm_addr_end.saturating_sub(self.vm_addr_start);
            self.vm_addr_start = region.vm_addr_end;
            len
        } else {
            // consume part of the region
            let len = self.vm_addr_end.saturating_sub(self.vm_addr_start);
            self.vm_addr_start = self.vm_addr_end;
            len
        };

        Some(Ok((region, vm_addr, chunk_len as usize)))
    }
}

impl<'a> DoubleEndedIterator for MemoryChunkIterator<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.vm_addr_start == self.vm_addr_end {
            return None;
        }

        let region = match self.region(self.vm_addr_end.saturating_sub(1)) {
            Ok(region) => region,
            Err(e) => {
                self.vm_addr_start = self.vm_addr_end;
                return Some(Err(e));
            }
        };

        let chunk_len = if region.vm_addr >= self.vm_addr_start {
            // consume the whole region
            let len = self.vm_addr_end.saturating_sub(region.vm_addr);
            self.vm_addr_end = region.vm_addr;
            len
        } else {
            // consume part of the region
            let len = self.vm_addr_end.saturating_sub(self.vm_addr_start);
            self.vm_addr_end = self.vm_addr_start;
            len
        };

        Some(Ok((region, self.vm_addr_end, chunk_len as usize)))
    }
}

pub fn translate(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    len: u64,
) -> helpers::StdResult<u64, SvmError> {
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
    skip_addr_translation: bool,
) -> Result<&'a mut T, SvmError> {
    let host_addr = if skip_addr_translation {
        vm_addr
    } else {
        translate(memory_mapping, access_type, vm_addr, size_of::<T>() as u64)?
    };
    if !check_aligned {
        #[cfg(target_pointer_width = "64")]
        {
            Ok(unsafe { core::mem::transmute::<u64, &mut T>(host_addr) })
        }
        #[cfg(target_pointer_width = "32")]
        {
            Ok(unsafe { core::mem::transmute::<u32, &mut T>(host_addr as u32) })
        }
    } else if !helpers::address_is_aligned::<T>(host_addr) {
        Err(SyscallError::UnalignedPointer.into())
    } else {
        Ok(unsafe { &mut *(host_addr as *mut T) })
    }
}
pub fn translate_type_mut<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
    skip_addr_translation: bool,
) -> Result<&'a mut T, SvmError> {
    translate_type_inner::<T>(
        memory_mapping,
        AccessType::Store,
        vm_addr,
        check_aligned,
        skip_addr_translation,
    )
}
pub fn translate_type<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
    skip_addr_translation: bool,
) -> Result<&'a T, SvmError> {
    translate_type_inner::<T>(
        memory_mapping,
        AccessType::Load,
        vm_addr,
        check_aligned,
        skip_addr_translation,
    )
    .map(|value| &*value)
}

fn translate_slice_inner<'a, T: Clone + SpecMethods<'a>>(
    memory_mapping: &'a MemoryMapping<'a>,
    access_type: AccessType,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<SliceFatPtr64<'a, T>, SvmError> {
    let mmh: MemoryMappingHelper = memory_mapping.into();
    if len == 0 {
        return Ok(SliceFatPtr64::default1(mmh.clone()));
    }
    let size_of_t = size_of::<T>();

    let total_size = len.saturating_mul(size_of_t as u64);
    if isize::try_from(total_size).is_err() {
        return Err(SyscallError::InvalidLength.into());
    }

    let host_addr = translate(memory_mapping, access_type, vm_addr, total_size)?;

    if check_aligned && !helpers::address_is_aligned::<T>(host_addr) {
        return Err(SyscallError::UnalignedPointer.into());
    }
    let result = SliceFatPtr64::new(mmh, AddrType::new_host(host_addr), len as usize);
    Ok(result)
}

pub fn translate_slice<'a, T: Clone + SpecMethods<'a>>(
    memory_mapping: &'a MemoryMapping,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<SliceFatPtr64<'a, T>, SvmError> {
    translate_slice_inner::<T>(
        memory_mapping,
        AccessType::Load,
        vm_addr,
        len,
        check_aligned,
    )
    // .map(|value| &*value)
}

pub fn translate_slice_mut<'a, T: Clone + SpecMethods<'a>>(
    memory_mapping: &'a MemoryMapping,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<SliceFatPtr64<'a, T>, SvmError> {
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
    let buf = translate_slice::<u8>(memory_mapping, addr, len, check_aligned)?.to_vec_cloned();
    match from_utf8(buf.as_slice()) {
        Ok(message) => work(message),
        Err(err) => Err(SyscallError::InvalidString(err, buf).into()),
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
    let mut dst_ptr = translate_slice_mut::<u8>(
        memory_mapping,
        dst_addr,
        n,
        invoke_context.get_check_aligned(),
    )?;
    let src_ptr = translate_slice::<u8>(
        memory_mapping,
        src_addr,
        n,
        invoke_context.get_check_aligned(),
    )?;

    dst_ptr.copy_from(&src_ptr)?;
    Ok(0)
}
pub fn translate_and_check_program_address_inputs<'a>(
    seeds_addr: u64,
    seeds_len: u64,
    program_id_addr: u64,
    memory_mapping: &mut MemoryMapping,
    check_aligned: bool,
) -> Result<(Vec<Vec<u8>>, &'a Pubkey), SvmError> {
    let untranslated_seeds =
        translate_slice::<SliceFatPtr64<u8>>(memory_mapping, seeds_addr, seeds_len, check_aligned)?;
    if untranslated_seeds.len() > MAX_SEEDS {
        return Err(SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded).into());
    }
    let seeds = untranslated_seeds
        .iter()
        .map(|untranslated_seed| {
            if untranslated_seed.as_ref().len() > MAX_SEED_LEN {
                return Err(SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded).into());
            }
            Ok(untranslated_seed.as_ref().to_vec_cloned())
        })
        .collect::<Result<Vec<_>, SvmError>>()?;

    let program_id =
        translate_type::<Pubkey>(memory_mapping, program_id_addr, check_aligned, false)?;
    Ok((seeds, program_id))
}
