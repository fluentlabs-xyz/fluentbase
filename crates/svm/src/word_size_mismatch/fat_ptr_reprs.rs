use alloc::vec::Vec;
use core::{
    iter::Iterator,
    marker::PhantomData,
    ops::{Bound, Index, RangeBounds},
    ptr::NonNull,
    slice::SliceIndex,
};
use solana_account_info::AccountInfo;
use solana_instruction::AccountMeta;
use solana_stable_layout::stable_vec::StableVec;

pub const FAT_PTR64_ELEM_BYTE_SIZE: u64 = 8;
pub const SLICE_FAT_PTR64_BYTE_SIZE: u64 = FAT_PTR64_ELEM_BYTE_SIZE * 2;
pub const STABLE_VEC_FAT_PTR64_BYTE_SIZE: u64 = 8 * 3;

pub trait ElemTypeConstraints = Clone + SpecMethods;

pub trait SpecMethods {
    const BYTE_SIZE: u64;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self;
}

#[macro_export]
macro_rules! impl_numeric_type {
    ($typ: ident) => {
        impl $crate::word_size_mismatch::fat_ptr_reprs::SpecMethods for $typ {
            const BYTE_SIZE: u64 = core::mem::size_of::<$typ>() as u64;

            fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
                $typ::from_le_bytes(byte_repr[..Self::BYTE_SIZE as usize].try_into().unwrap())
            }
        }
    };
}

impl SpecMethods for u8 {
    const BYTE_SIZE: u64 = size_of::<Self>() as u64;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        byte_repr[0]
    }
}

/// Slice impl emulating 64 bit word size to support solana 64 bit programs
#[derive(Clone)]
pub struct SliceFatPtr64<T: ElemTypeConstraints> {
    first_item_fat_ptr_addr: u64,
    len: u64,
    _phantom: PhantomData<T>,
}

impl<T: ElemTypeConstraints> SpecMethods for SliceFatPtr64<T> {
    const BYTE_SIZE: u64 = SLICE_FAT_PTR64_BYTE_SIZE;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        Self::from_fat_ptr_slice(byte_repr)
    }
}

impl<T: ElemTypeConstraints> Default for SliceFatPtr64<T> {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl<T: ElemTypeConstraints> SliceFatPtr64<T> {
    pub fn new(first_item_fat_ptr_addr: u64, len: u64) -> Self {
        Self {
            first_item_fat_ptr_addr,
            len,
            _phantom: Default::default(),
        }
    }

    pub fn try_get(&self, idx: u64) -> Option<T> {
        if idx < self.len {
            return Some(self.item_at_idx(idx));
        }
        None
    }

    pub fn first_item_fat_ptr_addr(&self) -> u64 {
        self.first_item_fat_ptr_addr
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn get_mut(&mut self, range: impl RangeBounds<usize>) -> Option<SliceFatPtr64<T>> {
        let start = match range.start_bound().cloned() {
            Bound::Included(v) => v,
            _ => 0,
        };
        if start >= self.len as usize {
            return None;
        }
        let end = match range.end_bound().cloned() {
            Bound::Included(v) => v + 1,
            Bound::Excluded(v) => v,
            Bound::Unbounded => self.len as usize,
        };
        if end > self.len as usize {
            return None;
        }
        Some(SliceFatPtr64::from_first_item_ptr_and_len(
            self.item_addr_at_idx(start as u64),
            (end - start) as u64,
        ))
    }

    pub fn fat_ptr_addr_as_vec(&self) -> Vec<u8> {
        self.first_item_fat_ptr_addr.to_le_bytes().to_vec()
    }

    pub fn len_as_vec(&self) -> Vec<u8> {
        self.len.to_le_bytes().to_vec()
    }

    pub fn item_addr_at_idx(&self, idx: u64) -> u64 {
        self.first_item_fat_ptr_addr + idx * T::BYTE_SIZE
    }

    pub fn item_ptr_at_idx(&self, idx: u64) -> *const T {
        self.item_addr_at_idx(idx) as *const T
    }

    pub fn item_ptr_at_idx_mut(&self, idx: u64) -> *mut T {
        self.item_addr_at_idx(idx) as *mut T
    }

    pub fn try_set_item_at_idx_mut(&mut self, idx: u64, value: T) -> bool {
        if idx < self.len {
            unsafe {
                *self.item_ptr_at_idx_mut(idx) = value;
            }
            return true;
        }
        false
    }

    pub fn item_at_idx(&self, idx: u64) -> T {
        let byte_repr = unsafe {
            core::slice::from_raw_parts(
                self.item_addr_at_idx(idx) as *const u8,
                T::BYTE_SIZE as usize,
            )
        };
        T::recover_from_byte_repr(byte_repr)
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut r = Vec::with_capacity(self.len as usize);
        for v in self.iter() {
            r.push(v);
        }
        r
    }

    pub fn copy_from_slice(&mut self, slice: &[T]) {
        assert_eq!(
            self.len,
            slice.len() as u64,
            "lengths must be equal when copying slices"
        );
        for (idx, elem) in slice.iter().enumerate() {
            unsafe { *self.item_ptr_at_idx_mut(idx as u64) = (*elem).clone() }
        }
    }

    pub fn copy_from(&mut self, src: &SliceFatPtr64<T>) -> bool {
        if self.len != src.len {
            return false;
        }
        if self.len == 0 {
            return true;
        }
        for (idx, val) in src.iter().enumerate() {
            self.try_set_item_at_idx_mut(idx as u64, val);
        }
        true
    }

    pub fn fill(&mut self, val: T) {
        for idx in 0..self.len {
            self.try_set_item_at_idx_mut(idx, val.clone());
        }
    }

    pub fn from_fat_ptr_fixed_slice(
        fat_ptr_slice: &[u8; SLICE_FAT_PTR64_BYTE_SIZE as usize],
    ) -> Self {
        let first_item_fat_ptr_addr = u64::from_le_bytes(
            fat_ptr_slice[..FAT_PTR64_ELEM_BYTE_SIZE as usize]
                .try_into()
                .unwrap(),
        );
        let len = u64::from_le_bytes(
            fat_ptr_slice[FAT_PTR64_ELEM_BYTE_SIZE as usize..SLICE_FAT_PTR64_BYTE_SIZE as usize]
                .try_into()
                .unwrap(),
        );
        Self::new(first_item_fat_ptr_addr, len)
    }

    pub fn from_fat_ptr_slice(ptr: &[u8]) -> Self {
        assert_eq!(
            ptr.len() as u64,
            SLICE_FAT_PTR64_BYTE_SIZE,
            "fat ptr must have {} byte len",
            SLICE_FAT_PTR64_BYTE_SIZE
        );
        Self::from_fat_ptr_fixed_slice(ptr.try_into().unwrap())
    }

    pub fn from_ptr_to_fat_ptr(ptr: u64) -> Self {
        let fat_ptr_slice = unsafe {
            core::slice::from_raw_parts(ptr as *const u8, SLICE_FAT_PTR64_BYTE_SIZE as usize)
        };
        Self::from_fat_ptr_slice(fat_ptr_slice)
    }

    pub fn from_first_item_ptr_and_len(ptr: u64, len: u64) -> Self {
        Self::new(ptr, len)
    }

    pub fn iter(&self) -> SliceFatPtr64Iterator<T> {
        self.into()
    }
}

pub struct SliceFatPtr64Iterator<'a, T: ElemTypeConstraints> {
    instance: &'a SliceFatPtr64<T>,
    idx: u64,
}
impl<'a, T: ElemTypeConstraints> From<&'a SliceFatPtr64<T>> for SliceFatPtr64Iterator<'a, T> {
    fn from(instance: &'a SliceFatPtr64<T>) -> Self {
        Self { instance, idx: 0 }
    }
}

impl<'a, T: ElemTypeConstraints> Iterator for SliceFatPtr64Iterator<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < self.instance.len {
            let r = self.instance.item_at_idx(self.idx);
            self.idx += 1;
            return Some(r);
        }
        None
    }
}

impl<'a, T: ElemTypeConstraints> IntoIterator for &'a SliceFatPtr64<T> {
    type Item = T;
    type IntoIter = SliceFatPtr64Iterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

impl<T: Sized> SpecMethods for StableVec<T> {
    const BYTE_SIZE: u64 = STABLE_VEC_FAT_PTR64_BYTE_SIZE;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        let ptr_addr = u64::from_le_bytes(
            byte_repr[..FAT_PTR64_ELEM_BYTE_SIZE as usize]
                .try_into()
                .unwrap(),
        );
        let ptr = unsafe { NonNull::new_unchecked(ptr_addr as *mut T) };
        StableVec {
            ptr,
            cap: usize::from_le_bytes(
                byte_repr[FAT_PTR64_ELEM_BYTE_SIZE as usize..FAT_PTR64_ELEM_BYTE_SIZE as usize * 2]
                    .try_into()
                    .unwrap(),
            ),
            len: usize::from_le_bytes(
                byte_repr[(FAT_PTR64_ELEM_BYTE_SIZE as usize * 2)
                    ..(FAT_PTR64_ELEM_BYTE_SIZE as usize) * 3]
                    .try_into()
                    .unwrap(),
            ),
            _marker: Default::default(),
        }
    }
}

impl SpecMethods for AccountMeta {
    const BYTE_SIZE: u64 = size_of::<AccountMeta>() as u64;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        todo!()
    }
}

impl SpecMethods for AccountInfo<'_> {
    const BYTE_SIZE: u64 = size_of::<AccountInfo>() as u64;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use crate::word_size_mismatch::fat_ptr_reprs::{
        SliceFatPtr64,
        SpecMethods,
        FAT_PTR64_ELEM_BYTE_SIZE,
        SLICE_FAT_PTR64_BYTE_SIZE,
    };
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;
    use solana_stable_layout::stable_vec::StableVec;

    impl_numeric_type!(u16);

    #[test]
    fn slice_of_slices_ro_test() {
        let payer = Pubkey::new_unique();
        let seed1 = b"my_seed".as_slice();
        let seed2 = payer.as_ref();
        let items = &[seed1, seed2];
        let seeds_first_item_addr = items.as_ptr() as u64;
        let seeds_len = items.len();

        let items_fat_ptr_to_first_item = unsafe {
            core::slice::from_raw_parts::<u8>(
                seeds_first_item_addr as *const u8,
                SLICE_FAT_PTR64_BYTE_SIZE as usize,
            )
        };
        let addr = u64::from_le_bytes(
            items_fat_ptr_to_first_item[..FAT_PTR64_ELEM_BYTE_SIZE as usize]
                .try_into()
                .unwrap(),
        );
        let len = u64::from_le_bytes(
            items_fat_ptr_to_first_item[FAT_PTR64_ELEM_BYTE_SIZE as usize..]
                .try_into()
                .unwrap(),
        );
        let slice_raw = unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) };
        let slice_custom = SliceFatPtr64::<u8>::from_first_item_ptr_and_len(addr, len);

        let items_fat_ptr = unsafe {
            core::slice::from_raw_parts::<u8>(
                (seeds_first_item_addr + SLICE_FAT_PTR64_BYTE_SIZE) as *const u8,
                SLICE_FAT_PTR64_BYTE_SIZE as usize,
            )
        };

        let items_recovered = SliceFatPtr64::<SliceFatPtr64<u8>>::from_first_item_ptr_and_len(
            seeds_first_item_addr,
            seeds_len as u64,
        );

        assert_eq!(items_recovered.len() as usize, items.len());
        for (idx, item_recovered) in items_recovered.iter().enumerate() {
            assert_eq!(&item_recovered.to_vec(), items[idx]);
        }
    }

    #[test]
    fn u8_items_test() {
        type ElemType = u8;
        let items = [1 as ElemType, 2, 3, 3, 2, 1].as_slice();
        let items_first_item_ptr = items.as_ptr() as u64;
        let items_len = items.len() as u64;

        let slice =
            SliceFatPtr64::<ElemType>::from_first_item_ptr_and_len(items_first_item_ptr, items_len);

        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items[idx]);
        }
    }

    #[test]
    fn u8_items_mutations_test() {
        type ElemType = u8;
        let items_original_fixed = [1 as ElemType, 2, 3, 3, 2, 1];
        let items_new_fixed = [7 as ElemType, 3, 22, 32, 74, 12];
        let items_original = items_original_fixed.as_slice();
        let items_new = items_new_fixed.as_slice();
        let items_first_item_ptr = items_original.as_ptr() as u64;
        let items_len = items_original.len() as u64;

        let mut slice =
            SliceFatPtr64::<ElemType>::from_first_item_ptr_and_len(items_first_item_ptr, items_len);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_new[idx]);
        }

        let fill_with = 0;
        slice.fill(fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.fill(fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, fill_with);
        }

        let fill_with = rand::random::<_>();
        let range = 1..3;
        slice.get_mut(range.clone()).map(|mut s| s.fill(fill_with));
        for idx in range {
            let item = slice.item_at_idx(idx as u64);
            assert_eq!(item, fill_with);
        }
    }

    #[test]
    fn u16_items_test() {
        type ElemType = u16;
        let items = [9281 as ElemType, 2222, 3333, 3323, 12314, 14215].as_slice();
        let items_first_item_ptr = items.as_ptr() as u64;
        let array_len = items.len() as u64;

        let slice =
            SliceFatPtr64::<ElemType>::from_first_item_ptr_and_len(items_first_item_ptr, array_len);

        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items[idx]);
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

        let mut slice =
            SliceFatPtr64::<ElemType>::from_first_item_ptr_and_len(items_first_item_ptr, items_len);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_new[idx]);
        }

        let fill_with = 0;
        slice.fill(fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.fill(fill_with);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, fill_with);
        }

        let fill_with = rand::random::<_>();
        slice.try_set_item_at_idx_mut(5, fill_with);
        assert_eq!(slice.to_vec()[5], fill_with);

        let fill_with = rand::random::<_>();
        let range = 1..3;
        slice.get_mut(range.clone()).map(|mut s| s.fill(fill_with));
        for idx in range {
            let item = slice.item_at_idx(idx as u64);
            assert_eq!(item, fill_with);
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
        type ElemType = StableVec<AccountMeta>;
        let items_original_fixed = [
            ElemType::from([AccountMeta::new(Pubkey::new_unique(), false)].to_vec()),
            // ElemType::from([AccountMeta::new(Pubkey::new_unique(), true)].to_vec()),
        ];
        let items_new_fixed = [
            ElemType::from([AccountMeta::new(Pubkey::new_unique(), false)].to_vec()),
            // ElemType::from([AccountMeta::new(Pubkey::new_unique(), true)].to_vec()),
        ];
        let items_original = items_original_fixed.as_slice();
        let items_new = items_new_fixed.as_slice();
        let items_first_item_ptr = items_original.as_ptr() as u64;
        let items_len = items_original.len() as u64;

        let mut slice =
            SliceFatPtr64::<ElemType>::from_first_item_ptr_and_len(items_first_item_ptr, items_len);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_original[idx]);
        }

        slice.copy_from_slice(items_new);
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_new[idx]);
        }
    }
}
