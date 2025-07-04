use crate::{
    error::RuntimeError,
    map_addr,
    word_size::{
        addr_type::AddrType,
        common::{
            typecast_bytes,
            MemoryMappingHelper,
            FAT_PTR64_ELEM_BYTE_SIZE,
            SLICE_FAT_PTR64_SIZE_BYTES,
        },
    },
};
use alloc::vec::Vec;
use core::{
    fmt::{Debug, Formatter},
    iter::Iterator,
    marker::PhantomData,
    ops::{Bound, RangeBounds},
};
use solana_account_info::AccountInfo;
use solana_instruction::AccountMeta;
use solana_rbpf::{
    error::ProgramResult,
    memory_region::{AccessType, MemoryMapping},
};

// pub trait ElementConstraints<'a> = Clone + SpecMethods<'a> + Debug;

pub enum RetVal<'a, T: Sized> {
    Instance(T),
    Reference(&'a T),
}

impl<'a, T: Sized> AsRef<T> for RetVal<'a, T> {
    fn as_ref(&self) -> &T {
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
        memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self>
    where
        Self: Sized;
}

impl<'a> SpecMethods<'a> for u8 {
    const ITEM_SIZE_BYTES: usize = size_of::<Self>();

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        _memory_mapping_helper: MemoryMappingHelper<'a>,
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
        impl<'a> $crate::word_size::slice::SpecMethods<'a> for $typ {
            const ITEM_SIZE_BYTES: usize = core::mem::size_of::<$typ>();

            fn recover_from_bytes(
                byte_repr: &'a [u8],
                _memory_mapping_helper: MemoryMappingHelper<'a>,
            ) -> RetVal<'a, Self> {
                RetVal::Reference(typecast_bytes(&byte_repr[..Self::ITEM_SIZE_BYTES]))
            }
        }
    };
}

impl_numeric_type!(u16);

#[derive(Clone, Debug, Default)]
pub struct SliceFatPtr64Repr {
    first_item_addr: AddrType,
    len: usize,
}

impl SliceFatPtr64Repr {
    pub fn new(first_item_addr: AddrType, len: usize) -> Self {
        Self {
            first_item_addr,
            len,
        }
    }

    pub fn first_item_addr(&self) -> AddrType {
        self.first_item_addr
    }

    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn idx_valid(&self, idx: usize) -> bool {
        idx < self.len
    }

    pub fn ptr_elem_from_slice(data: &[u8]) -> u64 {
        assert!(
            data.len() >= FAT_PTR64_ELEM_BYTE_SIZE,
            "data min len {}",
            FAT_PTR64_ELEM_BYTE_SIZE
        );
        u64::from_le_bytes(data[..FAT_PTR64_ELEM_BYTE_SIZE].try_into().unwrap())
    }

    pub fn from_fixed_fat_ptr_slice(slice_fat_ptr: &[u8; SLICE_FAT_PTR64_SIZE_BYTES]) -> Self {
        let first_item_addr = Self::ptr_elem_from_slice(&slice_fat_ptr[..FAT_PTR64_ELEM_BYTE_SIZE]);
        let len = Self::ptr_elem_from_slice(&slice_fat_ptr[FAT_PTR64_ELEM_BYTE_SIZE..]);
        Self::new(AddrType::new_vm(first_item_addr), len as usize)
    }

    pub fn ptr_elem_from_addr(ptr: u64) -> u64 {
        let data = reconstruct_slice(ptr as usize, FAT_PTR64_ELEM_BYTE_SIZE);
        Self::ptr_elem_from_slice(data)
    }

    pub fn from_fat_ptr_slice(ptr: &[u8]) -> Self {
        assert_eq!(
            ptr.len(),
            SLICE_FAT_PTR64_SIZE_BYTES,
            "fat ptr must have {} byte len",
            SLICE_FAT_PTR64_SIZE_BYTES
        );
        Self::from_fixed_fat_ptr_slice(ptr.try_into().unwrap())
    }

    pub fn from_ptr_to_fixed_slice_fat_ptr(ptr: usize) -> Self {
        let fat_ptr_slice =
            unsafe { core::slice::from_raw_parts(ptr as *const u8, SLICE_FAT_PTR64_SIZE_BYTES) };
        Self::from_fat_ptr_slice(fat_ptr_slice)
    }

    #[inline(always)]
    pub fn total_size_bytes(&self, item_size_bytes: usize) -> usize {
        self.len * item_size_bytes
    }

    pub fn map_vm_addr_to_host(
        memory_mapping: &MemoryMapping,
        vm_addr: u64,
        len: u64,
        access_type: Option<AccessType>,
    ) -> ProgramResult {
        memory_mapping.map(access_type.unwrap_or(AccessType::Load), vm_addr, len)
    }
}

/// Slice impl emulating 64 bit word size to support solana 64 bit programs
#[derive(Clone, Default)]
pub struct SliceFatPtr64<'a, T: SpecMethods<'a>> {
    slice_repr: SliceFatPtr64Repr,
    memory_mapping_helper: MemoryMappingHelper<'a>,
    _phantom: PhantomData<T>,
}

impl<'a, T: SpecMethods<'a>> Debug for SliceFatPtr64<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SliceFatPtr64")
            .field("slice_repr", &self.slice_repr)
            .finish()
    }
}

#[inline(always)]
pub fn reconstruct_slice<'a, T>(ptr: usize, len: usize) -> &'a [T] {
    unsafe { core::slice::from_raw_parts::<'a>(ptr as *const T, len) }
}

#[inline(always)]
pub fn reconstruct_slice_mut<'a, T>(ptr: usize, len: usize) -> &'a mut [T] {
    unsafe { core::slice::from_raw_parts_mut::<'a>(ptr as *mut T, len) }
}

impl<'a, T: Clone + SpecMethods<'a> + Debug> SpecMethods<'a> for SliceFatPtr64<'a, T> {
    const ITEM_SIZE_BYTES: usize = SLICE_FAT_PTR64_SIZE_BYTES;

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self> {
        let mut ptr = SliceFatPtr64Repr::from_fat_ptr_slice(byte_repr);
        let len = ptr.total_size_bytes(Self::ITEM_SIZE_BYTES);
        ptr.first_item_addr
            .try_transform_to_host_with(|v| map_addr!(memory_mapping_helper.clone(), v, len))
            .expect("failed to transform addr to host");

        let result = Self::new(memory_mapping_helper, ptr.first_item_addr, ptr.len);
        RetVal::Instance(result)
    }
}

impl<'a, T: Clone + SpecMethods<'a> + Debug> SliceFatPtr64<'a, T> {
    pub fn new(
        memory_mapping_helper: MemoryMappingHelper<'a>,
        first_item_addr: AddrType,
        len: usize,
    ) -> Self {
        Self {
            slice_repr: SliceFatPtr64Repr::new(first_item_addr, len),
            memory_mapping_helper,
            _phantom: Default::default(),
        }
    }

    pub fn default1(memory_mapping_helper: MemoryMappingHelper<'a>) -> Self {
        Self {
            slice_repr: Default::default(),
            memory_mapping_helper,
            _phantom: Default::default(),
        }
    }

    pub fn from_slice_repr(
        slice_repr: SliceFatPtr64Repr,
        memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> Self {
        Self {
            slice_repr,
            memory_mapping_helper,
            _phantom: Default::default(),
        }
    }

    pub fn clone_from_index(&self, idx: usize) -> Option<SliceFatPtr64<'a, T>> {
        if self.slice_repr.idx_valid(idx) {
            return Some(Self {
                slice_repr: SliceFatPtr64Repr::new(
                    self.item_addr_at_idx(idx),
                    self.slice_repr.len - idx,
                ),
                memory_mapping_helper: self.memory_mapping_helper.clone(),
                _phantom: Default::default(),
            });
        }
        None
    }

    pub fn try_get(&self, idx: usize) -> Option<RetVal<'a, T>> {
        if self.slice_repr.idx_valid(idx) {
            return Some(self.item_at_idx(idx));
        }
        None
    }

    #[inline(always)]
    pub fn first_item_addr(&self) -> AddrType {
        self.slice_repr.first_item_addr
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.slice_repr.len
    }

    #[inline(always)]
    pub fn item_size_bytes(&self) -> usize {
        T::ITEM_SIZE_BYTES
    }

    #[inline(always)]
    pub fn total_size_bytes(&self) -> usize {
        self.slice_repr.len * self.item_size_bytes()
    }

    pub fn get_mut(
        &mut self,
        range: impl RangeBounds<usize>,
    ) -> Result<SliceFatPtr64<'a, T>, RuntimeError> {
        let start = match range.start_bound().cloned() {
            Bound::Included(v) => v,
            _ => 0,
        };
        if start >= self.len() {
            return Err(RuntimeError::InvalidIdx);
        }
        let end = match range.end_bound().cloned() {
            Bound::Included(v) => v + 1,
            Bound::Excluded(v) => v,
            Bound::Unbounded => self.len(),
        };
        if end > self.len() {
            return Err(RuntimeError::InvalidIdx);
        }
        Ok(SliceFatPtr64::new(
            self.memory_mapping_helper.clone(),
            self.item_addr_at_idx(start),
            end - start,
        ))
    }

    pub fn len_as_vec(&self) -> Vec<u8> {
        self.slice_repr.len.to_le_bytes().to_vec()
    }

    pub fn item_addr_at_idx(&self, idx: usize) -> AddrType {
        self.first_item_addr() + (idx * T::ITEM_SIZE_BYTES) as u64
    }

    pub fn item_ptr_at_idx(&self, idx: usize) -> *const T {
        self.item_addr_at_idx(idx).inner() as *const T
    }

    pub fn item_ptr_at_idx_mut(&self, idx: usize) -> *mut T {
        self.item_addr_at_idx(idx).inner() as *mut T
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
        let byte_repr = reconstruct_slice::<'a, u8>(
            self.item_addr_at_idx(idx).inner() as usize,
            T::ITEM_SIZE_BYTES,
        );
        T::recover_from_bytes(byte_repr, self.memory_mapping_helper.clone())
    }

    pub fn try_as_single_item(&self) -> Option<RetVal<'a, T>> {
        if self.slice_repr.len != 1 {
            return None;
        }
        Some(self.item_at_idx(0))
    }

    pub fn to_vec(&'a self) -> Vec<RetVal<'a, T>> {
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
        assert_eq!(self.len(), slice.len(), "lengths must be equal");
        for (idx, elem) in slice.iter().enumerate() {
            // will panic for entities containing pointer fields
            unsafe { *self.item_ptr_at_idx_mut(idx) = (*elem).clone() }
        }
    }

    pub fn copy_from(&mut self, src: &'a SliceFatPtr64<'a, T>) -> Result<(), RuntimeError> {
        if self.slice_repr.len != src.slice_repr.len {
            return Err(RuntimeError::InvalidLength);
        }
        if self.slice_repr.len == 0 {
            return Ok(());
        }
        for (idx, val) in src.iter().enumerate() {
            self.try_set_item_at_idx_mut(idx, val.as_ref());
        }
        Ok(())
    }

    pub fn fill(&mut self, val: &T) {
        for idx in 0..self.slice_repr.len {
            self.try_set_item_at_idx_mut(idx, val);
        }
    }

    pub fn from_fixed_slice_fat_ptr(
        fat_ptr_slice: &[u8; SLICE_FAT_PTR64_SIZE_BYTES],
        memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> Self {
        let first_item_addr = SliceFatPtr64Repr::ptr_elem_from_slice(&fat_ptr_slice[..]);
        let len =
            SliceFatPtr64Repr::ptr_elem_from_slice(&fat_ptr_slice[FAT_PTR64_ELEM_BYTE_SIZE..]);
        Self::new(
            memory_mapping_helper,
            AddrType::new_vm(first_item_addr),
            len as usize,
        )
    }

    pub fn from_fat_ptr_slice(
        fat_ptr: &[u8],
        memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> Self {
        assert_eq!(
            fat_ptr.len(),
            SLICE_FAT_PTR64_SIZE_BYTES,
            "fat ptr must have {} byte len",
            SLICE_FAT_PTR64_SIZE_BYTES
        );
        Self::from_fixed_slice_fat_ptr(fat_ptr.try_into().unwrap(), memory_mapping_helper)
    }

    pub fn from_ptr_to_fat_ptr(
        addr: usize,
        memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> Self {
        let fat_ptr_slice =
            unsafe { core::slice::from_raw_parts(addr as *const u8, SLICE_FAT_PTR64_SIZE_BYTES) };
        Self::from_fat_ptr_slice(fat_ptr_slice, memory_mapping_helper)
    }

    pub fn iter(&'a self) -> SliceFatPtr64Iterator<'a, T> {
        self.into()
    }
}

pub struct SliceFatPtr64Iterator<'a, T: Clone + SpecMethods<'a> + Debug> {
    instance: &'a SliceFatPtr64<'a, T>,
    idx: usize,
}
impl<'a, T: Clone + SpecMethods<'a> + Debug> From<&'a SliceFatPtr64<'a, T>>
    for SliceFatPtr64Iterator<'a, T>
{
    fn from(instance: &'a SliceFatPtr64<'a, T>) -> Self {
        Self { instance, idx: 0 }
    }
}

impl<'a, T: Clone + SpecMethods<'a> + Debug> Iterator for SliceFatPtr64Iterator<'a, T> {
    type Item = RetVal<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < self.instance.slice_repr.len {
            let r = self.instance.item_at_idx(self.idx);
            self.idx += 1;
            return Some(r);
        }
        None
    }
}

impl<'a, T: Clone + SpecMethods<'a> + Debug> IntoIterator for &'a SliceFatPtr64<'a, T> {
    type Item = RetVal<'a, T>;
    type IntoIter = SliceFatPtr64Iterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

pub const ACCOUNT_META_ITEM_SIZE_64BIT_WORD: usize = 34;

impl<'a> SpecMethods<'a> for AccountMeta {
    const ITEM_SIZE_BYTES: usize = ACCOUNT_META_ITEM_SIZE_64BIT_WORD; // this value is the save as size_of::<>() in 64-bit system

    fn recover_from_bytes(
        data: &'a [u8],
        _memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self> {
        RetVal::Reference(typecast_bytes(data))
    }
}

pub const ACCOUNT_INFO_ITEM_SIZE_64BIT_WORD: usize = 48;

impl<'a> SpecMethods<'a> for AccountInfo<'a> {
    const ITEM_SIZE_BYTES: usize = ACCOUNT_INFO_ITEM_SIZE_64BIT_WORD; // this value is the save as size_of::<AccountInfo>() in 64-bit system

    fn recover_from_bytes(
        _data: &'a [u8],
        _memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self> {
        panic!(
            "cannot recover {} as it contains word size deps",
            crate::typ_name!(Self)
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::word_size::{
        common::MemoryMappingHelper,
        slice::{
            SliceFatPtr64,
            ACCOUNT_INFO_ITEM_SIZE_64BIT_WORD,
            ACCOUNT_META_ITEM_SIZE_64BIT_WORD,
        },
    };
    use solana_account_info::AccountInfo;
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;
    use solana_stable_layout::stable_vec::StableVec;

    #[test]
    fn structs_sizes_test() {
        assert_eq!(
            crate::typ_size!(AccountMeta),
            ACCOUNT_META_ITEM_SIZE_64BIT_WORD
        );
        assert_eq!(
            crate::typ_size!(AccountInfo),
            ACCOUNT_INFO_ITEM_SIZE_64BIT_WORD
        );
    }

    #[test]
    fn u8_items_test() {
        type ElemType = u8;
        let items = [1 as ElemType, 2, 3, 3, 2, 1].as_slice();
        let items_first_item_ptr = items.as_ptr() as usize;
        let items_len = items.len();

        let slice = SliceFatPtr64::<ElemType>::new(
            MemoryMappingHelper::default(),
            items_first_item_ptr.into(),
            items_len,
        );

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

        let slice_external = SliceFatPtr64::<SliceFatPtr64<u8>>::new(
            MemoryMappingHelper::default(),
            b1_first_item_ptr.into(),
            b1_len,
        );

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

        let slice = SliceFatPtr64::<SliceFatPtr64<SliceFatPtr64<u8>>>::new(
            MemoryMappingHelper::default(),
            c1_first_item_ptr.into(),
            c1_len,
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

        let mut slice = SliceFatPtr64::<ElemType>::new(
            MemoryMappingHelper::default(),
            (items_first_item_ptr as u64).into(),
            items_len,
        );
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_new[idx]);
        }

        let fill_with = 0;
        slice.fill(&fill_with);
        for (_idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.fill(&fill_with);
        for (_idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        let range = 1..3;
        slice
            .get_mut(range.clone())
            .map(|mut s| s.fill(&fill_with))
            .unwrap();
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
        let array_len = items.len();

        let slice = SliceFatPtr64::<ElemType>::new(
            MemoryMappingHelper::default(),
            items_first_item_ptr.into(),
            array_len,
        );

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
        let items_len = items_original.len();

        let mut slice = SliceFatPtr64::<ElemType>::new(
            MemoryMappingHelper::default(),
            items_first_item_ptr.into(),
            items_len,
        );
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &items_new[idx]);
        }

        let fill_with = 0;
        slice.fill(&fill_with);
        for (_idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.fill(&fill_with);
        for (_idx, item) in slice.iter().enumerate() {
            assert_eq!(item.as_ref(), &fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.try_set_item_at_idx_mut(5, &fill_with);
        assert_eq!(slice.to_vec()[5].as_ref(), &fill_with);

        let fill_with = rand::random::<_>();
        let range = 1..3;
        slice
            .get_mut(range.clone())
            .map(|mut s| s.fill(&fill_with))
            .unwrap();
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

        assert_eq!(T1_SIZE, T2_SIZE);
        assert_eq!(T2_SIZE, T3_SIZE);
    }

    #[test]
    fn stable_vec_of_account_meta_items_mutations_test() {
        // type ItemType = u64;
        type ItemType = AccountMeta;
        type VecOfItemsType = StableVec<ItemType>;
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

        let mut slice = SliceFatPtr64::<ItemType>::new(
            MemoryMappingHelper::default(),
            (items_original_fixed.as_ref().as_ptr() as u64).into(),
            items_len,
        );
        for idx in 0..slice.len() {
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
}
