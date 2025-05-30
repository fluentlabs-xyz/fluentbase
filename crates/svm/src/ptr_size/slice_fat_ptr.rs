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

pub const FAT_PTR64_ELEM_BYTE_SIZE: usize = 8;
pub const SLICE_FAT_PTR64_BYTE_SIZE: usize = FAT_PTR64_ELEM_BYTE_SIZE * 2;
pub const STABLE_VEC_FAT_PTR64_BYTE_SIZE: usize = 8 * 3;

pub enum ArrayFatPtr<'a> {
    Slice(&'a [u8; SLICE_FAT_PTR64_BYTE_SIZE]),
    StableVec(&'a [u8; STABLE_VEC_FAT_PTR64_BYTE_SIZE]),
}

pub trait ElementConstraints = Clone + SpecMethods;

pub trait SpecMethods {
    const SIZE_IN_BYTES: usize;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self;
}

macro_rules! impl_numeric_type {
    ($typ: ident) => {
        impl $crate::ptr_size::slice_fat_ptr::SpecMethods for $typ {
            const SIZE_IN_BYTES: usize = core::mem::size_of::<$typ>();

            fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
                $typ::from_le_bytes(byte_repr[..Self::SIZE_IN_BYTES].try_into().unwrap())
            }
        }
    };
}

impl SpecMethods for u8 {
    const SIZE_IN_BYTES: usize = size_of::<Self>();

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        byte_repr[0]
    }
}

impl_numeric_type!(u16);
impl_numeric_type!(u32);
impl_numeric_type!(u64);

/// Slice impl emulating 64 bit word size to support solana 64 bit programs
#[derive(Clone)]
pub struct SliceFatPtr64<T: ElementConstraints> {
    first_item_fat_ptr_addr: usize,
    len: usize,
    _phantom: PhantomData<T>,
}

impl<T: ElementConstraints> SpecMethods for SliceFatPtr64<T> {
    const SIZE_IN_BYTES: usize = SLICE_FAT_PTR64_BYTE_SIZE;

    fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
        Self::from_fat_ptr_slice(byte_repr)
    }
}

impl<'a, T: ElementConstraints> Default for SliceFatPtr64<T> {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl<T: ElementConstraints> SliceFatPtr64<T> {
    pub fn new(first_item_fat_ptr_addr: usize, len: usize) -> Self {
        Self {
            first_item_fat_ptr_addr,
            len,
            _phantom: Default::default(),
        }
    }

    pub fn try_get(&self, idx: usize) -> Option<T> {
        if idx < self.len {
            return Some(self.item_at_idx(idx));
        }
        None
    }

    pub fn first_item_fat_ptr_addr(&self) -> usize {
        self.first_item_fat_ptr_addr
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn get_mut(&mut self, range: impl RangeBounds<usize>) -> Option<SliceFatPtr64<T>> {
        let start = match range.start_bound().cloned() {
            Bound::Included(v) => v,
            _ => 0,
        };
        if start >= self.len {
            return None;
        }
        let end = match range.end_bound().cloned() {
            Bound::Included(v) => v + 1,
            Bound::Excluded(v) => v,
            Bound::Unbounded => self.len,
        };
        if end > self.len {
            return None;
        }
        Some(SliceFatPtr64::from_first_item_ptr_and_len(
            self.item_addr_at_idx(start),
            end - start,
        ))
    }

    pub fn fat_ptr_addr_as_vec(&self) -> Vec<u8> {
        self.first_item_fat_ptr_addr.to_le_bytes().to_vec()
    }

    pub fn len_as_vec(&self) -> Vec<u8> {
        self.len.to_le_bytes().to_vec()
    }

    pub fn item_addr_at_idx(&self, idx: usize) -> usize {
        self.first_item_fat_ptr_addr + idx * T::SIZE_IN_BYTES
    }

    pub fn item_ptr_at_idx(&self, idx: usize) -> *const T {
        self.item_addr_at_idx(idx) as *const T
    }

    pub fn item_ptr_at_idx_mut(&self, idx: usize) -> *mut T {
        self.item_addr_at_idx(idx) as *mut T
    }

    pub fn try_set_item_at_idx_mut(&mut self, idx: usize, value: T) -> bool {
        if idx < self.len {
            unsafe {
                *self.item_ptr_at_idx_mut(idx) = value;
            }
            return true;
        }
        false
    }

    pub fn item_at_idx(&self, idx: usize) -> T {
        let byte_repr = unsafe {
            core::slice::from_raw_parts(
                self.item_addr_at_idx(idx) as *const u8,
                T::SIZE_IN_BYTES as usize,
            )
        };
        T::recover_from_byte_repr(byte_repr)
    }

    pub fn size_in_bytes(&self) -> usize {
        T::SIZE_IN_BYTES
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut r = Vec::with_capacity(self.len);
        for v in self.iter() {
            r.push(v.clone());
        }
        r
    }

    pub fn copy_from_slice(&mut self, slice: &[T]) {
        assert_eq!(
            self.len,
            slice.len(),
            "lengths must be equal when copying slices"
        );
        for (idx, elem) in slice.iter().enumerate() {
            unsafe { *self.item_ptr_at_idx_mut(idx) = (*elem).clone() }
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
            self.try_set_item_at_idx_mut(idx, val.clone());
        }
        true
    }

    pub fn fill(&mut self, val: T) {
        for idx in 0..self.len {
            self.try_set_item_at_idx_mut(idx, val.clone());
        }
    }

    pub fn from_fat_ptr_fixed_slice(fat_ptr_slice: &[u8; SLICE_FAT_PTR64_BYTE_SIZE]) -> Self {
        let first_item_fat_ptr_addr = u64::from_le_bytes(
            fat_ptr_slice[..FAT_PTR64_ELEM_BYTE_SIZE]
                .try_into()
                .unwrap(),
        );
        let len = u64::from_le_bytes(
            fat_ptr_slice[FAT_PTR64_ELEM_BYTE_SIZE..SLICE_FAT_PTR64_BYTE_SIZE]
                .try_into()
                .unwrap(),
        );
        Self::new(first_item_fat_ptr_addr as usize, len as usize)
    }

    pub fn from_fat_ptr_slice(ptr: &[u8]) -> Self {
        assert_eq!(
            ptr.len(),
            SLICE_FAT_PTR64_BYTE_SIZE,
            "fat ptr must have {} byte len",
            SLICE_FAT_PTR64_BYTE_SIZE
        );
        Self::from_fat_ptr_fixed_slice(ptr.try_into().unwrap())
    }

    pub fn from_ptr_to_fat_ptr(ptr: usize) -> Self {
        let fat_ptr_slice =
            unsafe { core::slice::from_raw_parts(ptr as *const u8, SLICE_FAT_PTR64_BYTE_SIZE) };
        Self::from_fat_ptr_slice(fat_ptr_slice)
    }

    pub fn from_first_item_ptr_and_len(ptr: usize, len: usize) -> Self {
        Self::new(ptr, len)
    }

    pub fn iter(&self) -> SliceFatPtr64Iterator<T> {
        self.into()
    }
}

pub struct SliceFatPtr64Iterator<'a, T: ElementConstraints> {
    instance: &'a SliceFatPtr64<T>,
    idx: usize,
}
impl<'a, T: ElementConstraints> From<&'a SliceFatPtr64<T>> for SliceFatPtr64Iterator<'a, T> {
    fn from(instance: &'a SliceFatPtr64<T>) -> Self {
        Self { instance, idx: 0 }
    }
}

impl<'a, T: ElementConstraints> Iterator for SliceFatPtr64Iterator<'a, T> {
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

impl<'a, T: ElementConstraints> IntoIterator for &'a SliceFatPtr64<T> {
    type Item = T;
    type IntoIter = SliceFatPtr64Iterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

// impl<'a, T: Sized> SpecMethods for StableVec<T> {
//     const SIZE_IN_BYTES: usize = STABLE_VEC_FAT_PTR64_BYTE_SIZE;
//
//     fn recover_from_byte_repr(byte_repr: &[u8]) -> Self {
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

pub fn typecase_slice<T: Clone>(data: &[u8]) -> T {
    let data = data.as_ref();
    let type_name = core::any::type_name::<T>();
    if data.len() < size_of::<T>() {
        panic!("failed to typecase to {}: invalid size", type_name);
    }

    let ptr = data.as_ptr() as *const T;

    // Check alignment
    if (ptr as usize) % align_of::<T>() != 0 {
        panic!("failed to typecase to {}: misaligned", type_name);
    }

    unsafe { (*ptr).clone() }
}

impl<'a> SpecMethods for AccountMeta {
    const SIZE_IN_BYTES: usize = size_of::<Self>();

    fn recover_from_byte_repr(data: &[u8]) -> Self {
        typecase_slice::<Self>(data)
    }
}

impl SpecMethods for AccountInfo<'_> {
    const SIZE_IN_BYTES: usize = size_of::<AccountInfo>();

    fn recover_from_byte_repr(data: &[u8]) -> Self {
        typecase_slice::<Self>(data)
        // todo!()
    }
}

#[cfg(test)]
mod tests {
    use crate::ptr_size::slice_fat_ptr::{
        SliceFatPtr64,
        FAT_PTR64_ELEM_BYTE_SIZE,
        SLICE_FAT_PTR64_BYTE_SIZE,
    };
    use paste::paste;
    use solana_account_info::AccountInfo;
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;
    use solana_stable_layout::stable_vec::StableVec;

    pub fn typecase_bytes<T: Clone>(data: &[u8]) -> &T {
        let data = data.as_ref();
        let type_name = core::any::type_name::<T>();
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

    #[test]
    fn recover_ref() {
        struct SomeStruct<'a> {
            value: &'a u8,
        }
        impl<'a> SomeStruct<'a> {
            pub fn new(value: &'a u8) -> Self {
                Self { value }
            }

            pub fn from_slice(v: &'a [u8]) -> Self {
                Self::new(typecase_bytes(v))
            }
        }

        let value = 14;
        let mut buf: Vec<u8> = Vec::new();
        buf.push(value);

        let value_restored = SomeStruct::from_slice(&buf);

        assert_eq!(*value_restored.value, value);
    }

    #[test]
    fn slice_of_slices_ro_test() {
        let payer = Pubkey::new_unique();
        let seed1 = b"my_seed".as_slice();
        let seed2 = payer.as_ref();
        let items = &[seed1, seed2];
        let seeds_first_item_addr = items.as_ptr() as usize;
        let seeds_len = items.len();

        let items_fat_ptr_to_first_item = unsafe {
            core::slice::from_raw_parts::<u8>(
                seeds_first_item_addr as *const u8,
                SLICE_FAT_PTR64_BYTE_SIZE,
            )
        };
        let addr = u64::from_le_bytes(
            items_fat_ptr_to_first_item[..FAT_PTR64_ELEM_BYTE_SIZE]
                .try_into()
                .unwrap(),
        );
        let len = u64::from_le_bytes(
            items_fat_ptr_to_first_item[FAT_PTR64_ELEM_BYTE_SIZE..]
                .try_into()
                .unwrap(),
        );
        let slice_raw = unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) };
        let slice_custom =
            SliceFatPtr64::<u8>::from_first_item_ptr_and_len(addr as usize, len as usize);

        let items_fat_ptr = unsafe {
            core::slice::from_raw_parts::<u8>(
                (seeds_first_item_addr + SLICE_FAT_PTR64_BYTE_SIZE) as *const u8,
                SLICE_FAT_PTR64_BYTE_SIZE as usize,
            )
        };

        let items_recovered = SliceFatPtr64::<SliceFatPtr64<u8>>::from_first_item_ptr_and_len(
            seeds_first_item_addr,
            seeds_len,
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
        let items_first_item_ptr = items.as_ptr() as usize;
        let items_len = items.len();

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
        let items_first_item_ptr = items_original.as_ptr() as usize;
        let items_len = items_original.len();

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
            let item = slice.item_at_idx(idx);
            assert_eq!(item, fill_with);
        }
    }

    #[test]
    fn u16_items_test() {
        type ElemType = u16;
        let items = [9281 as ElemType, 2222, 3333, 3323, 12314, 14215].as_slice();
        let items_first_item_ptr = items.as_ptr() as usize;
        let array_len = items.len();

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
        let items_first_item_ptr = items_original.as_ptr() as usize;
        let items_len = items_original.len();

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
            let item = slice.item_at_idx(idx);
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

        let mut slice = SliceFatPtr64::<ItemType>::from_first_item_ptr_and_len(
            items_original_fixed.as_ref().as_ptr() as usize,
            items_len,
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
        for idx in 0..slice.len() {
            assert_eq!(slice.item_at_idx(idx), items_original_fixed[idx]);
        }
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_original_fixed[idx]);
        }

        slice.copy_from_slice(items_new_fixed.as_ref());
        for (idx, item) in slice.iter().enumerate() {
            assert_eq!(item, items_new_fixed[idx]);
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

        const NUM1: u64 = 1;
        let key_1 = Pubkey::new_from_array([NUM1 as u8; 32]);
        let owner_1 = Pubkey::new_from_array([NUM1 as u8 + 10; 32]);
        let mut lamports_1 = NUM1 + 20;
        let rent_epoch_1 = NUM1 + 30;
        let mut data_1 = [1, 2, 3].to_vec();

        const NUM2: u64 = 2;
        let key_2 = Pubkey::new_from_array([NUM2 as u8; 32]);
        let owner_2 = Pubkey::new_from_array([NUM2 as u8 + 10; 32]);
        let mut lamports_2 = NUM2 + 20;
        let rent_epoch_2 = NUM2 + 30;
        let mut data_2 = [1, 2, 3, 4].to_vec();

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
        // let items_new_fixed = VecOfItemsType::from(
        //     [
        //         ItemType::new(Pubkey::new_from_array([3; 32]), true),
        //         ItemType::new(Pubkey::new_from_array([4; 32]), false),
        //     ]
        //     .to_vec(),
        // );
        // assert_eq!(items_original_fixed.len(), items_new_fixed.len());
        let items_len = items_original_fixed.len();
        let vec_of_items_bytes_size = VEC_OF_ITEMS_TYPE_SIZE;
        let items_only_bytes_size = ITEM_SIZE * items_original_fixed.len();

        let mut slice = SliceFatPtr64::<ItemType>::from_first_item_ptr_and_len(
            items_original_fixed.as_ref().as_ptr() as usize,
            items_len,
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
        for idx in 0..slice.len() {
            let item_recovered = &slice.item_at_idx(idx);
            let item_original = &items_original_fixed[idx];
            let item_original_cloned = (*item_original).clone();
            macro_rules! assert_fields {
                ($field:ident) => {
                    assert_eq!(item_original.$field, item_recovered.$field);
                };
            }
            assert_fields!(data);
            assert_fields!(executable);
            assert_fields!(is_signer);
            assert_fields!(is_writable);
            assert_fields!(key);
            assert_fields!(lamports);
            assert_fields!(owner);
            assert_fields!(rent_epoch);
        }
        // for (idx, item) in slice.iter().enumerate() {
        //     assert_eq!(item, items_original_fixed[idx]);
        // }
        //
        // slice.copy_from_slice(items_new_fixed.as_ref());
        // for (idx, item) in slice.iter().enumerate() {
        //     assert_eq!(item, items_new_fixed[idx]);
        // }
    }
}
