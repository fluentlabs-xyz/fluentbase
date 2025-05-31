use alloc::{sync::Arc, vec::Vec};
use core::{
    any::type_name,
    fmt::{Debug, Formatter},
    iter::Iterator,
    marker::PhantomData,
    ops::{Bound, Index, RangeBounds},
    slice::SliceIndex,
};
use fluentbase_sdk::debug_log;
use lazy_static::lazy_static;
use solana_account_info::AccountInfo;
use solana_instruction::AccountMeta;
use solana_rbpf::{
    error::ProgramResult,
    memory_region::{AccessType, MemoryMapping},
};

pub const FAT_PTR64_ELEM_BYTE_SIZE: usize = 8;
pub const SLICE_FAT_PTR64_SIZE_BYTES: usize = FAT_PTR64_ELEM_BYTE_SIZE * 2;
pub const STABLE_VEC_FAT_PTR64_BYTE_SIZE: usize = 8 * 3;

#[inline(always)]
pub fn typecast_bytes<T: Clone>(data: &[u8]) -> &T {
    let data = data.as_ref();
    let type_name = type_name::<T>();
    if data.len() < size_of::<T>() {
        panic!("failed to typecase to {}: invalid size", type_name);
    }

    let ptr = data.as_ptr() as *const T;

    // Check alignment
    if (ptr as usize) % align_of::<T>() != 0 {
        panic!("failed to typecase to {}: misaligned", type_name);
    }

    unsafe { &*ptr }
}

pub trait ElementConstraints<'a> = Clone + SpecMethods<'a> + Debug;

pub fn translate_addr(
    memory_mapping: Option<&MemoryMapping>,
    vm_addr: u64,
    len: u64,
    access_type: Option<AccessType>,
) -> ProgramResult {
    memory_mapping
        .map(|v| v.map(access_type.unwrap_or(AccessType::Load), vm_addr, len))
        .unwrap_or(ProgramResult::Ok(vm_addr))
}
pub fn addr_translator_default(addr: u64) -> u64 {
    addr
}
lazy_static! {
    static ref ADDR_TRANSLATOR_DEFAULT: Arc<dyn Fn(u64) -> u64 + Send + Sync> =
        Arc::new(|v| addr_translator_default(v));
}

pub enum RetVal<'a, T: Sized> {
    Instance(T),
    Reference(&'a T),
}

impl<'a, T: Sized> RetVal<'a, T> {
    pub fn as_ref(&self) -> &T {
        match self {
            RetVal::Instance(v) => v,
            RetVal::Reference(v) => *v,
        }
    }
}

pub trait SpecMethods<'a> {
    const ITEM_SIZE_BYTES: usize;

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        memory_mapping: Option<&'a MemoryMapping<'a>>,
    ) -> RetVal<'a, Self>
    where
        Self: Sized;
}

#[derive(Clone, Debug)]
pub struct SliceFatPtr64Repr<const ITEM_SIZE_BYTES: usize> {
    first_item_fat_ptr_addr: u64,
    len: u64,
}

impl<const ITEM_SIZE_BYTES: usize> SliceFatPtr64Repr<ITEM_SIZE_BYTES> {
    pub fn new(first_item_fat_ptr_addr: u64, len: u64) -> Self {
        Self {
            first_item_fat_ptr_addr,
            len,
        }
    }
    pub fn from_fat_ptr_fixed_slice(fat_ptr_slice: &[u8; SLICE_FAT_PTR64_SIZE_BYTES]) -> Self {
        let first_item_fat_ptr_addr = u64::from_le_bytes(
            fat_ptr_slice[..FAT_PTR64_ELEM_BYTE_SIZE]
                .try_into()
                .unwrap(),
        );
        let len = u64::from_le_bytes(
            fat_ptr_slice[FAT_PTR64_ELEM_BYTE_SIZE..SLICE_FAT_PTR64_SIZE_BYTES]
                .try_into()
                .unwrap(),
        );
        Self::new(first_item_fat_ptr_addr, len)
    }

    pub fn from_fat_ptr_slice(ptr: &[u8]) -> Self {
        assert_eq!(
            ptr.len(),
            SLICE_FAT_PTR64_SIZE_BYTES,
            "fat ptr must have {} byte len",
            SLICE_FAT_PTR64_SIZE_BYTES
        );
        Self::from_fat_ptr_fixed_slice(ptr.try_into().unwrap())
    }

    pub fn from_ptr_to_fat_ptr(ptr: usize) -> Self {
        let fat_ptr_slice =
            unsafe { core::slice::from_raw_parts(ptr as *const u8, SLICE_FAT_PTR64_SIZE_BYTES) };
        Self::from_fat_ptr_slice(fat_ptr_slice)
    }

    #[inline(always)]
    pub fn item_size_bytes(&self) -> u64 {
        ITEM_SIZE_BYTES as u64
    }

    #[inline(always)]
    pub fn total_size_bytes(&self) -> usize {
        (self.len * self.item_size_bytes()) as usize
    }
}

/// Slice impl emulating 64 bit word size to support solana 64 bit programs
#[derive(Clone, Default)]
pub struct SliceFatPtr64<'a, T: SpecMethods<'a>> {
    first_item_fat_ptr_addr: u64,
    len: u64,
    memory_mapping: Option<&'a MemoryMapping<'a>>,
    _phantom: PhantomData<T>,
}

impl<'a, T: SpecMethods<'a>> Debug for SliceFatPtr64<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SliceFatPtr64")
            .field("first_item_fat_ptr_addr", &self.first_item_fat_ptr_addr)
            .field("len", &self.len)
            .finish()
    }
}

impl<'a> SpecMethods<'a> for u8 {
    const ITEM_SIZE_BYTES: usize = size_of::<Self>();

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        _memory_mapping: Option<&MemoryMapping>,
    ) -> RetVal<'a, Self> {
        let len = byte_repr.len() / Self::ITEM_SIZE_BYTES;
        let recovered_bytes_len = len * Self::ITEM_SIZE_BYTES;
        assert_eq!(
            recovered_bytes_len,
            byte_repr.len(),
            "invalid byte repr: {} != {}",
            recovered_bytes_len,
            byte_repr.len()
        );
        RetVal::Reference(typecast_bytes(byte_repr))
    }
}

macro_rules! impl_numeric_type {
    ($typ: ident) => {
        impl<'a> $crate::ptr_size::slice_fat_ptr_v3::SpecMethods<'a> for $typ {
            const ITEM_SIZE_BYTES: usize = core::mem::size_of::<$typ>();

            fn recover_from_bytes(
                byte_repr: &'a [u8],
                memory_mapping: Option<&MemoryMapping>,
            ) -> RetVal<'a, Self> {
                RetVal::Reference(typecast_bytes(&byte_repr[..Self::ITEM_SIZE_BYTES]))
            }
        }
    };
}

impl_numeric_type!(u16);

impl<'a> SpecMethods<'a> for &[u8] {
    // const ITEM_SIZE_BYTES: usize = size_of::<Self>();
    const ITEM_SIZE_BYTES: usize = SLICE_FAT_PTR64_SIZE_BYTES;
    fn recover_from_bytes(
        byte_repr: &'a [u8],
        memory_mapping: Option<&MemoryMapping>,
    ) -> RetVal<'a, Self> {
        let len = byte_repr.len() / Self::ITEM_SIZE_BYTES;
        let recovered_bytes_len = len * Self::ITEM_SIZE_BYTES;
        // TODO extract vm_addr and real len
        // TODO convert vm_addr into host_addr
        // TODO translate into slice using host addr
        let slice =
            SliceFatPtr64Repr::<{ SLICE_FAT_PTR64_SIZE_BYTES }>::from_fat_ptr_slice(byte_repr);
        let vm_addr = slice.first_item_fat_ptr_addr;
        let host_addr =
            translate_addr(memory_mapping, vm_addr, recovered_bytes_len as u64, None).unwrap();
        debug_log!(
            "reconstructing '{}' (vm_addr:{} host_addr:{} ITEM_SIZE_BYTES:{}) from byte_repr: {:x?}",
            type_name::<Self>(),
            vm_addr,
            host_addr,
            Self::ITEM_SIZE_BYTES,
            byte_repr,
        );
        assert_eq!(
            recovered_bytes_len,
            byte_repr.len(),
            "invalid byte repr: {} != {}",
            recovered_bytes_len,
            byte_repr.len()
        );
        let result =
            unsafe { core::slice::from_raw_parts(host_addr as *const u8, slice.len as usize) };
        // let result_ptr = unsafe { core::mem::transmute::<&[u8], &Self>(result) };
        // debug_log!(
        //     "result.ptr {} result_ptr {}",
        //     result.as_ptr() as u64,
        //     result_ptr.as_ptr() as u64
        // );
        RetVal::Instance(result)
    }
}

impl<'a> SpecMethods<'a> for &[&[u8]] {
    const ITEM_SIZE_BYTES: usize = SLICE_FAT_PTR64_SIZE_BYTES;
    fn recover_from_bytes(
        byte_repr: &'a [u8],
        memory_mapping: Option<&MemoryMapping>,
    ) -> RetVal<'a, Self> {
        let len = byte_repr.len() / Self::ITEM_SIZE_BYTES;
        let recovered_bytes_len = len * Self::ITEM_SIZE_BYTES;
        // TODO extract vm_addr and real len
        // TODO convert vm_addr into host_addr
        // TODO translate into slice using host addr
        let slice =
            SliceFatPtr64Repr::<{ SLICE_FAT_PTR64_SIZE_BYTES }>::from_fat_ptr_slice(byte_repr);
        let vm_addr = slice.first_item_fat_ptr_addr;
        let host_addr =
            translate_addr(memory_mapping, vm_addr, recovered_bytes_len as u64, None).unwrap();
        debug_log!(
            "reconstructing '{}' (vm_addr:{} host_addr:{} ITEM_SIZE_BYTES:{}) from byte_repr: {:x?}",
            type_name::<Self>(),
            vm_addr,
            host_addr,
            Self::ITEM_SIZE_BYTES,
            byte_repr,
        );
        assert_eq!(
            recovered_bytes_len,
            byte_repr.len(),
            "invalid byte repr: {} != {}",
            recovered_bytes_len,
            byte_repr.len()
        );
        let result =
            unsafe { core::slice::from_raw_parts(host_addr as *const &[u8], slice.len as usize) };
        // let result_ptr = unsafe { core::mem::transmute::<&[u8], &Self>(result) };
        // debug_log!(
        //     "result.ptr {} result_ptr {}",
        //     result.as_ptr() as u64,
        //     result_ptr.as_ptr() as u64
        // );
        RetVal::Instance(result)
    }
}

#[inline(always)]
pub fn reconstruct_slice<'a, T>(ptr: usize, len: usize) -> &'a [T] {
    unsafe { core::slice::from_raw_parts::<'a>(ptr as *const T, len) }
}

impl<'a, T: ElementConstraints<'a>> SpecMethods<'a> for SliceFatPtr64<'a, T> {
    const ITEM_SIZE_BYTES: usize = SLICE_FAT_PTR64_SIZE_BYTES;

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        memory_mapping: Option<&'a MemoryMapping<'a>>,
    ) -> RetVal<'a, Self> {
        let mut ptr =
            SliceFatPtr64Repr::<SLICE_FAT_PTR64_SIZE_BYTES>::from_fat_ptr_slice(byte_repr);
        memory_mapping.map(|v| {
            ptr.first_item_fat_ptr_addr = translate_addr(
                memory_mapping,
                ptr.first_item_fat_ptr_addr,
                ptr.total_size_bytes() as u64,
                None,
            )
            .unwrap()
        });
        // let result = Self::from_repr(&ptr, memory_mapping);
        let result = Self::new(ptr.first_item_fat_ptr_addr, ptr.len, memory_mapping);
        // let data = reconstruct_slice::<T>(ptr.first_item_fat_ptr_addr as usize, ptr.len as usize);
        debug_log!(
            "recover_from_bytes: ptr {:?} data for '{}': {:x?}",
            &ptr,
            type_name::<Self>(),
            &result
        );
        RetVal::Instance(result)
    }
}

impl<'a, T: ElementConstraints<'a>> SliceFatPtr64<'a, T> {
    pub fn new(
        first_item_fat_ptr_addr: u64,
        len: u64,
        memory_mapping: Option<&'a MemoryMapping<'a>>,
    ) -> Self {
        Self {
            first_item_fat_ptr_addr,
            len,
            memory_mapping,
            // addr_translator,
            _phantom: Default::default(),
        }
    }

    pub fn default(memory_mapping: Option<&'a MemoryMapping<'a>>) -> Self {
        Self {
            first_item_fat_ptr_addr: 0,
            len: 0,
            memory_mapping,
            _phantom: Default::default(),
        }
    }

    pub fn from_repr<const ITEM_SIZE_BYTES: usize>(
        ptr: &'a SliceFatPtr64Repr<ITEM_SIZE_BYTES>,
        memory_mapping: Option<&'a MemoryMapping<'a>>,
    ) -> Self {
        Self {
            first_item_fat_ptr_addr: ptr.first_item_fat_ptr_addr,
            len: ptr.len,
            memory_mapping,
            _phantom: Default::default(),
        }
    }

    pub fn try_get(&self, idx: usize) -> Option<RetVal<'a, T>> {
        if idx < self.len() {
            return Some(self.item_at_idx(idx));
        }
        None
    }

    #[inline(always)]
    pub fn first_item_fat_ptr_addr(&self) -> u64 {
        self.first_item_fat_ptr_addr
    }

    #[inline(always)]
    pub fn first_item_fat_ptr_addr_usize(&self) -> usize {
        self.first_item_fat_ptr_addr() as usize
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline(always)]
    pub fn item_size_bytes(&self) -> u64 {
        Self::ITEM_SIZE_BYTES as u64
    }

    #[inline(always)]
    pub fn total_size_bytes(&self) -> usize {
        (self.len * self.item_size_bytes()) as usize
    }

    pub fn get_mut(&mut self, range: impl RangeBounds<usize>) -> Option<SliceFatPtr64<'a, T>> {
        let start = match range.start_bound().cloned() {
            Bound::Included(v) => v,
            _ => 0,
        };
        if start >= self.len() {
            return None;
        }
        let end = match range.end_bound().cloned() {
            Bound::Included(v) => v + 1,
            Bound::Excluded(v) => v,
            Bound::Unbounded => self.len(),
        };
        if end > self.len() {
            return None;
        }
        Some(SliceFatPtr64::new(
            self.item_addr_at_idx(start) as u64,
            (end - start) as u64,
            self.memory_mapping,
            // ADDR_TRANSLATOR_DEFAULT.clone(),
        ))
    }

    pub fn fat_ptr_addr_as_vec(&self) -> Vec<u8> {
        self.first_item_fat_ptr_addr.to_le_bytes().to_vec()
    }

    pub fn len_as_vec(&self) -> Vec<u8> {
        self.len.to_le_bytes().to_vec()
    }

    pub fn item_addr_at_idx(&self, idx: usize) -> usize {
        self.first_item_fat_ptr_addr_usize() + idx * T::ITEM_SIZE_BYTES
    }

    pub fn item_ptr_at_idx(&self, idx: usize) -> *const T {
        self.item_addr_at_idx(idx) as *const T
    }

    pub fn item_ptr_at_idx_mut(&self, idx: usize) -> *mut T {
        self.item_addr_at_idx(idx) as *mut T
    }

    pub fn try_set_item_at_idx_mut(&mut self, idx: usize, value: &T) -> bool {
        if idx < self.len() {
            unsafe {
                *self.item_ptr_at_idx_mut(idx) = value.clone();
            }
            return true;
        }
        false
    }

    pub fn item_at_idx(&self, idx: usize) -> RetVal<'a, T> {
        let byte_repr =
            reconstruct_slice::<'a, u8>(self.item_addr_at_idx(idx), T::ITEM_SIZE_BYTES as usize);
        T::recover_from_bytes(byte_repr, self.memory_mapping)
    }

    pub fn as_single_item(&self) -> Option<RetVal<'a, T>> {
        if self.len != 0 {
            return None;
        }
        Some(self.item_at_idx(0))
    }

    pub fn to_vec(&'a self) -> Vec<RetVal<T>> {
        let mut r = Vec::with_capacity(self.len());
        for v in self.iter() {
            r.push(v);
        }
        r
    }

    pub fn to_vec_cloned(&'a self) -> Vec<T> {
        let mut r = Vec::with_capacity(self.len());
        for v in self.iter() {
            r.push((*v.as_ref()).clone());
        }
        r
    }

    pub fn copy_from_slice(&mut self, slice: &[T]) {
        assert_eq!(
            self.len(),
            slice.len(),
            "lengths must be equal when copying slices"
        );
        for (idx, elem) in slice.iter().enumerate() {
            // TODO this wont work for entities containing virtual pointer fields
            unsafe { *self.item_ptr_at_idx_mut(idx) = (*elem).clone() }
        }
    }

    pub fn copy_from(&mut self, src: &'a SliceFatPtr64<'a, T>) -> bool {
        if self.len != src.len {
            return false;
        }
        if self.len == 0 {
            return true;
        }
        for (idx, val) in src.iter().enumerate() {
            self.try_set_item_at_idx_mut(idx, val.as_ref());
        }
        true
    }

    pub fn fill(&mut self, val: &T) {
        for idx in 0..self.len {
            self.try_set_item_at_idx_mut(idx as usize, val);
        }
    }

    pub fn from_fat_ptr_fixed_slice(
        fat_ptr_slice: &[u8; SLICE_FAT_PTR64_SIZE_BYTES],
        memory_mapping: &'a MemoryMapping<'a>,
    ) -> Self {
        let first_item_fat_ptr_addr = u64::from_le_bytes(
            fat_ptr_slice[..FAT_PTR64_ELEM_BYTE_SIZE]
                .try_into()
                .unwrap(),
        );
        let len = u64::from_le_bytes(
            fat_ptr_slice[FAT_PTR64_ELEM_BYTE_SIZE..SLICE_FAT_PTR64_SIZE_BYTES]
                .try_into()
                .unwrap(),
        );
        Self::new(
            first_item_fat_ptr_addr,
            len,
            Some(memory_mapping), // ADDR_TRANSLATOR_DEFAULT.clone(),
        )
    }

    pub fn from_fat_ptr_slice(ptr: &[u8], memory_mapping: &'a MemoryMapping<'a>) -> Self {
        assert_eq!(
            ptr.len(),
            SLICE_FAT_PTR64_SIZE_BYTES,
            "fat ptr must have {} byte len",
            SLICE_FAT_PTR64_SIZE_BYTES
        );
        Self::from_fat_ptr_fixed_slice(ptr.try_into().unwrap(), memory_mapping)
    }

    pub fn from_ptr_to_fat_ptr(ptr: usize, memory_mapping: &'a MemoryMapping<'a>) -> Self {
        let fat_ptr_slice =
            unsafe { core::slice::from_raw_parts(ptr as *const u8, SLICE_FAT_PTR64_SIZE_BYTES) };
        Self::from_fat_ptr_slice(fat_ptr_slice, memory_mapping)
    }

    pub fn iter(&'a self) -> SliceFatPtr64Iterator<'a, T> {
        self.into()
    }
}

pub struct SliceFatPtr64Iterator<'a, T: ElementConstraints<'a>> {
    instance: &'a SliceFatPtr64<'a, T>,
    idx: usize,
}
impl<'a, T: ElementConstraints<'a>> From<&'a SliceFatPtr64<'a, T>>
    for SliceFatPtr64Iterator<'a, T>
{
    fn from(instance: &'a SliceFatPtr64<'a, T>) -> Self {
        Self { instance, idx: 0 }
    }
}

impl<'a, T: ElementConstraints<'a>> Iterator for SliceFatPtr64Iterator<'a, T> {
    type Item = RetVal<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < self.instance.len as usize {
            let r = self.instance.item_at_idx(self.idx);
            self.idx += 1;
            return Some(r);
        }
        None
    }
}

impl<'a, T: ElementConstraints<'a>> IntoIterator for &'a SliceFatPtr64<'a, T> {
    type Item = RetVal<'a, T>;
    type IntoIter = SliceFatPtr64Iterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

// impl<'a, T: Sized> SpecMethods for StableVec<T> {
//     const ITEM_SIZE_BYTES: usize = STABLE_VEC_FAT_PTR64_BYTE_SIZE;
//
//     fn recover_from_bytes(byte_repr: &[u8]) -> Self {
//         let ptr_addr = u64::from_le_bytes(
//             byte_repr[..FAT_PTR64_ELEM_BYTE_SIZE as usize]
//                 .try_into()
//                 .unwrap(),
//         );
//         let ptr = unsafe { NonNull::new_unchecked(ptr_addr as *mut T) };
//         StableVec {
//             ptr,
//             cap: usize::from_le_bytes(
//                 byte_repr[FAT_PTR64_ELEM_BYTE_SIZE..FAT_PTR64_ELEM_BYTE_SIZE * 2]
//                     .try_into()
//                     .unwrap(),
//             ),
//             len: usize::from_le_bytes(
//                 byte_repr[(FAT_PTR64_ELEM_BYTE_SIZE * 2)..(FAT_PTR64_ELEM_BYTE_SIZE as usize) * 3]
//                     .try_into()
//                     .unwrap(),
//             ),
//             _marker: Default::default(),
//         }
//     }
// }

impl<'a> SpecMethods<'a> for AccountMeta {
    const ITEM_SIZE_BYTES: usize = size_of::<Self>();

    fn recover_from_bytes(
        data: &'a [u8],
        memory_mapping: Option<&MemoryMapping>,
    ) -> RetVal<'a, Self> {
        RetVal::Reference(typecast_bytes(data))
    }
}

impl<'a> SpecMethods<'a> for AccountInfo<'a> {
    const ITEM_SIZE_BYTES: usize = size_of::<AccountInfo>();

    fn recover_from_bytes(
        data: &'a [u8],
        memory_mapping: Option<&MemoryMapping>,
    ) -> RetVal<'a, Self> {
        RetVal::Reference(typecast_bytes(data))
    }
}

#[cfg(test)]
mod tests {
    use crate::{mem_ops, ptr_size::slice_fat_ptr_v3::SliceFatPtr64};
    use solana_account_info::AccountInfo;
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;
    use solana_rbpf::memory_region::{AlignedMemoryMapping, MemoryMapping};
    use solana_stable_layout::stable_vec::StableVec;
    // #[test]
    // fn slice_of_slices_ro_test() {
    //     let payer = Pubkey::new_unique();
    //     let seed1 = b"my_seed".as_slice();
    //     let seed2 = payer.as_ref();
    //     let items = &[seed1, seed2];
    //     let seeds_first_item_addr = items.as_ptr() as usize;
    //     let seeds_len = items.len();
    //
    //     let items_fat_ptr_to_first_item = unsafe {
    //         core::slice::from_raw_parts::<u8>(
    //             seeds_first_item_addr as *const u8,
    //             SLICE_FAT_PTR64_BYTE_SIZE,
    //         )
    //     };
    //     let addr = typecase_bytes::<u64>(&items_fat_ptr_to_first_item[..FAT_PTR64_ELEM_BYTE_SIZE]);
    //     let len = typecase_bytes::<u64>(&items_fat_ptr_to_first_item[FAT_PTR64_ELEM_BYTE_SIZE..]);
    //     let slice_raw = unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) };
    //     let slice_custom =
    //         SliceFatPtr64::<u8>::from_first_item_ptr_and_len(addr as usize, len as usize);
    //
    //     let items_fat_ptr = unsafe {
    //         core::slice::from_raw_parts::<u8>(
    //             (seeds_first_item_addr + SLICE_FAT_PTR64_BYTE_SIZE) as *const u8,
    //             SLICE_FAT_PTR64_BYTE_SIZE as usize,
    //         )
    //     };
    //
    //     let items_recovered = SliceFatPtr64::<SliceFatPtr64<u8>>::from_first_item_ptr_and_len(
    //         seeds_first_item_addr,
    //         seeds_len,
    //     );
    //
    //     assert_eq!(items_recovered.len(), items.len());
    //     for (idx, item_recovered) in items_recovered.iter().enumerate() {
    //         assert_eq!(&item_recovered.to_vec(), items[idx]);
    //     }
    // }

    #[test]
    fn u8_items_test() {
        type ElemType = u8;
        let items = [1 as ElemType, 2, 3, 3, 2, 1].as_slice();
        let items_first_item_ptr = items.as_ptr() as usize;
        let items_len = items.len();

        let slice =
            SliceFatPtr64::<ElemType>::new(items_first_item_ptr as u64, items_len as u64, None);

        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items[idx]);
        }
    }

    #[test]
    fn u8_slice_of_slices_test() {
        type ItemType = u8;
        let a1: &[u8] = &[0x12 as ItemType, 0x2, 0x23, 0x3, 0x74, 0x1, 0x2];
        let a2: &[u8] = &[0x14 as ItemType, 0x41, 0x3, 0x3];
        let a3: &[u8] = &[
            0x12 as ItemType,
            0x83,
            0x3,
            0x23,
            0x12,
            0x1,
            0x32,
            0x65,
            0x54,
            0x12,
            0x65,
        ];
        let b1: &[&[u8]] = &[a1, a2, a3];
        let b1_first_item_ptr = b1.as_ptr() as usize;
        let b1_len = b1.len();

        let slice_external =
            SliceFatPtr64::<SliceFatPtr64<u8>>::new(b1_first_item_ptr as u64, b1_len as u64, None);

        for (idx_external, slice_internal) in slice_external.iter().enumerate() {
            for (idx_internal, item_internal) in slice_internal.as_ref().iter().enumerate() {
                assert_eq!(item_internal.as_ref(), &b1[idx_external][idx_internal]);
            }
        }
    }

    #[test]
    fn u8_slice_of_slices_of_slices_test() {
        type ItemType = u8;
        let a1: &[u8] = &[12 as ItemType, 2, 123, 3, 74, 1, 2];
        let a2: &[u8] = &[14 as ItemType, 41, 3, 3];
        let a3: &[u8] = &[12 as ItemType, 83, 3, 23, 12, 1, 32, 65, 54, 12, 65];
        let a4: &[u8] = &[4 as ItemType, 42, 33, 12, 17, 41];
        let a5: &[u8] = &[75 as ItemType, 32, 65, 54, 12, 65];
        let b1: &[&[u8]] = &[a1, a2, a3];
        let b2: &[&[u8]] = &[a4, a5];
        let c1: &[&[&[u8]]] = &[b1, b2];
        let c1_first_item_ptr = c1.as_ptr() as usize;
        let c1_len = c1.len();

        let type_name = core::any::type_name::<SliceFatPtr64<'_, u8>>();
        println!("type_name: {}", type_name);

        let slice = SliceFatPtr64::<SliceFatPtr64<SliceFatPtr64<u8>>>::new(
            c1_first_item_ptr as u64,
            c1_len as u64,
            None,
        );

        for (idx_1, slice_1) in slice.iter().enumerate() {
            for (idx_2, slice_2) in slice_1.as_ref().iter().enumerate() {
                for (idx_3, item_3) in slice_2.as_ref().iter().enumerate() {
                    assert_eq!(item_3.as_ref(), &c1[idx_1][idx_2][idx_3]);
                }
            }
        }
    }

    #[test]
    fn u8_items_mutations_test() {
        type ElemType = u8;
        let items_original_fixed = [1 as ElemType, 2, 3, 3, 2, 1];
        let items_new_fixed = [7 as ElemType, 3, 22, 32, 74, 12];
        let items_original = items_original_fixed.as_slice();
        let items_new = items_new_fixed.as_slice();
        let items_first_item_ptr = items_original.as_ptr();
        let items_len = items_original.len();

        let mut slice =
            SliceFatPtr64::<ElemType>::new(items_first_item_ptr as u64, items_len as u64, None);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_new[idx]);
        }

        let fill_with = 0;
        slice.fill(&fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.fill(&fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        let range = 1..3;
        slice.get_mut(range.clone()).map(|mut s| s.fill(&fill_with));
        for idx in range {
            let item = slice.item_at_idx(idx);
            assert_eq!(item.as_ref(), &fill_with);
        }
    }

    #[test]
    fn u16_items_test() {
        type ElemType = u16;
        let items = [9281 as ElemType, 2222, 3333, 3323, 12314, 14215].as_slice();
        let items_first_item_ptr = items.as_ptr() as u64;
        let array_len = items.len() as u64;

        let slice = SliceFatPtr64::<ElemType>::new(items_first_item_ptr, array_len, None);

        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items[idx]);
        }
    }

    #[test]
    fn u16_items_mutations_test() {
        type ElemType = u16;
        let items_original_fixed = [9281 as ElemType, 2222, 3333, 3323, 12314, 14215];
        let items_new_fixed = [63234 as ElemType, 14654, 28653, 12315, 51957, 34618];
        let items_original = items_original_fixed.as_slice();
        let items_new = items_new_fixed.as_slice();
        let items_first_item_ptr = items_original.as_ptr() as u64;
        let items_len = items_original.len() as u64;

        let mut slice = SliceFatPtr64::<ElemType>::new(items_first_item_ptr, items_len, None);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_new[idx]);
        }

        let fill_with = 0;
        slice.fill(&fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.fill(&fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.try_set_item_at_idx_mut(5, &fill_with);
        assert_eq!(slice.to_vec()[5].as_ref(), &fill_with);

        let fill_with = rand::random::<_>();
        let range = 1..3;
        slice.get_mut(range.clone()).map(|mut s| s.fill(&fill_with));
        for idx in range {
            let item = slice.item_at_idx(idx);
            assert_eq!(item.as_ref(), &fill_with);
        }
    }

    #[test]
    fn stable_vector_sizes_test() {
        macro_rules! define_symbols {
            ($postfix:expr, $typ:ty) => {
                paste::paste! {
                    type [<T $postfix>] = StableVec<$typ>;
                    const [<T $postfix _SIZE>]: usize = size_of::<[<T $postfix>]>();
                }
            };
        }
        define_symbols!(1, u8);
        define_symbols!(2, u16);
        define_symbols!(3, StableVec<AccountMeta>);

        let vec = [
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), true),
        ]
        .to_vec();
        let stable_vec = StableVec::from(vec);

        assert_eq!(T1_SIZE, T2_SIZE);
        assert_eq!(T2_SIZE, T3_SIZE);
        println!("T1_SIZE: {}", T1_SIZE);
    }

    #[test]
    fn stable_vec_of_account_meta_items_mutations_test() {
        // type ItemType = u64;
        type ItemType = AccountMeta;
        type VecOfItemsType = StableVec<ItemType>;
        const ITEM_SIZE: usize = size_of::<ItemType>();
        const VEC_OF_ITEMS_TYPE_SIZE: usize = size_of::<VecOfItemsType>();
        println!("ITEM_SIZE: {}", ITEM_SIZE);
        println!("VEC_OF_ITEMS_TYPE_SIZE: {}", VEC_OF_ITEMS_TYPE_SIZE);
        let items_original_fixed = VecOfItemsType::from(
            [
                ItemType::new(Pubkey::new_from_array([1; 32]), false),
                ItemType::new(Pubkey::new_from_array([2; 32]), true),
            ]
            .to_vec(),
        );
        let items_new_fixed = VecOfItemsType::from(
            [
                ItemType::new(Pubkey::new_from_array([3; 32]), true),
                ItemType::new(Pubkey::new_from_array([4; 32]), false),
            ]
            .to_vec(),
        );
        assert_eq!(items_original_fixed.len(), items_new_fixed.len());
        let items_len = items_original_fixed.len();
        let vec_of_items_bytes_size = VEC_OF_ITEMS_TYPE_SIZE;
        let items_only_bytes_size = ITEM_SIZE * items_original_fixed.len();

        let mut slice = SliceFatPtr64::<ItemType>::new(
            items_original_fixed.as_ref().as_ptr() as u64,
            items_len as u64,
            None,
        );
        println!("vec_of_items_bytes_size {}", vec_of_items_bytes_size);
        let vec_of_items_start_ptr = unsafe { (&items_original_fixed) as *const _ } as u64;
        let first_item_start_ptr = items_original_fixed.as_ptr() as u64;
        println!(
            "vec_of_items_start_ptr {} ({:x?}) first_item_start_ptr {} ({:x?})",
            vec_of_items_start_ptr,
            &vec_of_items_start_ptr.to_le_bytes(),
            first_item_start_ptr,
            &first_item_start_ptr.to_le_bytes()
        );
        let vec_of_items_as_raw_bytes = unsafe {
            alloc::slice::from_raw_parts(
                vec_of_items_start_ptr as *const u8,
                vec_of_items_bytes_size,
            )
        };
        println!(
            "vec_of_items_as_raw_bytes ({}): {:x?}",
            vec_of_items_bytes_size, vec_of_items_as_raw_bytes
        );
        let items_as_raw_bytes = unsafe {
            alloc::slice::from_raw_parts(first_item_start_ptr as *const u8, items_only_bytes_size)
        };
        println!(
            "items_as_raw_bytes ({}): {:x?}",
            items_only_bytes_size, items_as_raw_bytes
        );
        for idx in 0..slice.len() as usize {
            assert_eq!(slice.item_at_idx(idx).as_ref(), &items_original_fixed[idx]);
        }
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_original_fixed[idx]);
        }

        slice.copy_from_slice(items_new_fixed.as_ref());
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_new_fixed[idx]);
        }
    }

    #[test]
    fn stable_vec_of_account_infos_mutations_test() {
        // type ItemType = u64;
        type ItemType<'a> = AccountInfo<'a>;
        type VecOfItemsType<'a> = StableVec<ItemType<'a>>;
        const ITEM_SIZE: usize = size_of::<ItemType>();
        const VEC_OF_ITEMS_TYPE_SIZE: usize = size_of::<VecOfItemsType>();
        println!("ITEM_SIZE: {}", ITEM_SIZE);
        println!("VEC_OF_ITEMS_TYPE_SIZE: {}", VEC_OF_ITEMS_TYPE_SIZE);

        let num: u64 = 1;
        let key_1 = Pubkey::new_from_array([num as u8; 32]);
        let owner_1 = Pubkey::new_from_array([num as u8 + 10; 32]);
        let mut lamports_1 = num + 20;
        let rent_epoch_1 = num + 30;
        let mut data_1 = [1, 2, 3].to_vec();

        let num: u64 = 2;
        let key_2 = Pubkey::new_from_array([num as u8; 32]);
        let owner_2 = Pubkey::new_from_array([num as u8 + 10; 32]);
        let mut lamports_2 = num + 20;
        let rent_epoch_2 = num + 30;
        let mut data_2 = [1, 2, 3, 4].to_vec();

        let num: u64 = 4;
        let key_3 = Pubkey::new_from_array([num as u8; 32]);
        let owner_3 = Pubkey::new_from_array([num as u8 + 10; 32]);
        let mut lamports_3 = num + 20;
        let rent_epoch_3 = num + 30;
        let mut data_3 = [1, 2, 3, 4].to_vec();

        let num: u64 = 3;
        let key_4 = Pubkey::new_from_array([num as u8; 32]);
        let owner_4 = Pubkey::new_from_array([num as u8 + 10; 32]);
        let mut lamports_4 = num + 20;
        let rent_epoch_4 = num + 30;
        let mut data_4 = [1, 2, 3, 4].to_vec();

        let items_original_fixed: StableVec<ItemType> = VecOfItemsType::from(
            [
                ItemType::new(
                    &key_1,
                    true,
                    true,
                    &mut lamports_1,
                    &mut data_1,
                    &owner_1,
                    true,
                    rent_epoch_1,
                ),
                ItemType::new(
                    &key_2,
                    true,
                    true,
                    &mut lamports_2,
                    &mut data_2,
                    &owner_2,
                    true,
                    rent_epoch_2,
                ),
            ]
            .to_vec(),
        );
        let items_new_fixed: StableVec<ItemType> = VecOfItemsType::from(
            [
                ItemType::new(
                    &key_3,
                    true,
                    true,
                    &mut lamports_3,
                    &mut data_3,
                    &owner_3,
                    true,
                    rent_epoch_3,
                ),
                ItemType::new(
                    &key_4,
                    true,
                    true,
                    &mut lamports_4,
                    &mut data_4,
                    &owner_4,
                    true,
                    rent_epoch_4,
                ),
            ]
            .to_vec(),
        );
        assert_eq!(items_original_fixed.len(), items_new_fixed.len());
        let items_len = items_original_fixed.len();
        let vec_of_items_bytes_size = VEC_OF_ITEMS_TYPE_SIZE;
        let items_only_bytes_size = ITEM_SIZE * items_original_fixed.len();

        let mut slice = SliceFatPtr64::<ItemType>::new(
            items_original_fixed.as_ref().as_ptr() as u64,
            items_len as u64,
            None,
        );
        println!("vec_of_items_bytes_size {}", vec_of_items_bytes_size);
        let vec_of_items_start_ptr = (&items_original_fixed) as *const _ as u64;
        let first_item_start_ptr = items_original_fixed.as_ptr() as u64;
        println!(
            "vec_of_items_start_ptr {} ({:x?}) first_item_start_ptr {} ({:x?})",
            vec_of_items_start_ptr,
            &vec_of_items_start_ptr.to_le_bytes(),
            first_item_start_ptr,
            &first_item_start_ptr.to_le_bytes()
        );
        let vec_of_items_as_raw_bytes = unsafe {
            alloc::slice::from_raw_parts(
                vec_of_items_start_ptr as *const u8,
                vec_of_items_bytes_size,
            )
        };
        println!(
            "vec_of_items_as_raw_bytes ({}): {:x?}",
            vec_of_items_bytes_size, vec_of_items_as_raw_bytes
        );
        let items_as_raw_bytes = unsafe {
            alloc::slice::from_raw_parts(first_item_start_ptr as *const u8, items_only_bytes_size)
        };
        println!(
            "items_as_raw_bytes ({}): {:x?}",
            items_only_bytes_size, items_as_raw_bytes
        );
        macro_rules! assert_fields {
            ($original:expr, $recovered:expr, $field:ident) => {
                assert_eq!($original.$field, $recovered.$field);
            };
        }
        for idx in 0..slice.len() {
            let item_original = &items_original_fixed[idx];
            let item_restored = slice.item_at_idx(idx);
            let item_recovered = item_restored.as_ref();
            let item_original_cloned = (*item_original).clone();
            assert_fields!(item_original, item_recovered, data);
            assert_fields!(item_original, item_recovered, executable);
            assert_fields!(item_original, item_recovered, is_signer);
            assert_fields!(item_original, item_recovered, is_writable);
            assert_fields!(item_original, item_recovered, key);
            assert_fields!(item_original, item_recovered, lamports);
            assert_fields!(item_original, item_recovered, owner);
            assert_fields!(item_original, item_recovered, rent_epoch);
        }
        slice.copy_from_slice(items_new_fixed.as_ref());
        for idx in 0..slice.len() {
            let item_original = &items_new_fixed[idx];
            let item_restored = slice.item_at_idx(idx);
            let item_recovered = item_restored.as_ref();
            let item_original_cloned = (*item_original).clone();
            assert_fields!(item_original, item_recovered, data);
            assert_fields!(item_original, item_recovered, executable);
            assert_fields!(item_original, item_recovered, is_signer);
            assert_fields!(item_original, item_recovered, is_writable);
            assert_fields!(item_original, item_recovered, key);
            assert_fields!(item_original, item_recovered, lamports);
            assert_fields!(item_original, item_recovered, owner);
            assert_fields!(item_original, item_recovered, rent_epoch);
        }
    }
}
