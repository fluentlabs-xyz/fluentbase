use crate::word_size::{
    common::{typecast_bytes, typecast_bytes_mut, MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
    ptr_type::PtrType,
    slice::{reconstruct_slice, SliceFatPtr64Repr},
};
use core::{
    fmt::Display,
    marker::PhantomData,
    ops::{Add, Index, RangeBounds},
    slice::SliceIndex,
};
use num_traits::ToPrimitive;

pub trait SpecMethods<'a> {
    type Elem;
    fn addr_to_value_addr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType)
        -> u64;
    fn value_addr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64;
    fn value(addr: usize) -> &'a Self::Elem;
    fn value_mut(addr: usize) -> &'a mut Self::Elem;
}

pub struct RcRefCellMemLayout<'a, T> {
    memory_mapping_helper: MemoryMappingHelper<'a>,
    ptr: PtrType,
    _phantom: PhantomData<T>,
}

impl<'a, T: SpecMethods<'a>> RcRefCellMemLayout<'a, T> {
    pub fn new(
        memory_mapping_helper: MemoryMappingHelper<'a>,
        ptr: PtrType,
    ) -> RcRefCellMemLayout<'a, T> {
        Self {
            memory_mapping_helper,
            ptr,
            _phantom: Default::default(),
        }
    }

    pub fn addr_to_value_addr<const PRE_MAP: bool, const POST_MAP: bool>(&'a self) -> u64 {
        let mut ptr = self.ptr;
        if PRE_MAP {
            ptr.visit_inner_mut(|v| {
                *v = self
                    .memory_mapping_helper
                    .map_vm_addr_to_host(*v, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap();
            });
        };
        let addr = T::addr_to_value_addr(&self.memory_mapping_helper, &ptr);
        crate::map_addr!(POST_MAP, self.memory_mapping_helper, addr)
    }

    pub fn value_addr<const PRE_MAP: bool, const POST_MAP: bool>(&'a self) -> u64 {
        let mut ptr = self.ptr;
        if PRE_MAP {
            ptr.visit_inner_mut(|v| {
                *v = self
                    .memory_mapping_helper
                    .map_vm_addr_to_host(*v, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap();
            });
        };
        let addr = T::value_addr(&self.memory_mapping_helper, &ptr);
        crate::map_addr!(POST_MAP, self.memory_mapping_helper, addr)
    }

    pub fn value<const PRE_MAP: bool>(&'a self) -> &'a T::Elem {
        T::value(self.value_addr::<PRE_MAP, true>() as usize)
    }

    pub fn value_mut<const PRE_MAP: bool>(&'a self) -> &'a mut T::Elem {
        T::value_mut(self.value_addr::<PRE_MAP, true>() as usize)
    }
}

fn fetch_addr_to_value_addr_common(ptr_type: PtrType) -> u64 {
    match ptr_type {
        PtrType::PtrToValuePtr(ptr_to_value_ptr) => ptr_to_value_ptr,
        PtrType::RcBoxStartPtr(rc_box_start_ptr) => {
            let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE as u64 * 3;
            ptr_to_value_ptr
        }
        PtrType::RcStartPtr(rc_start_ptr) => {
            let rc_box_ptr = SliceFatPtr64Repr::ptr_elem_from_addr(rc_start_ptr);
            let ptr = PtrType::RcBoxStartPtr(rc_box_ptr);
            fetch_addr_to_value_addr_common(ptr)
        }
    }
}

pub fn fetch_value_addr_common(ptr_type: PtrType) -> u64 {
    match ptr_type {
        PtrType::PtrToValuePtr(ptr_to_value_ptr) => {
            SliceFatPtr64Repr::ptr_elem_from_addr(ptr_to_value_ptr)
        }
        PtrType::RcBoxStartPtr(rc_box_start_ptr) => {
            let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE as u64 * 3;
            u64::from_le_bytes(
                reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                    .try_into()
                    .unwrap(),
            )
        }
        PtrType::RcStartPtr(rc_start_ptr) => {
            let rc_box_ptr = SliceFatPtr64Repr::ptr_elem_from_addr(rc_start_ptr);
            let ptr_to_value_ptr = rc_box_ptr + FIXED_PTR_BYTE_SIZE as u64 * 3;
            ptr_to_value_ptr
        }
    }
}

macro_rules! fetch_value_common {
    ($value_addr:ident, $typecase_fn:ident) => {{
        $typecase_fn(
            reconstruct_slice($value_addr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        )
    }};
}

impl<'a> SpecMethods<'a> for &mut u64 {
    type Elem = u64;

    fn addr_to_value_addr(
        _memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64 {
        fetch_addr_to_value_addr_common(*ptr)
    }

    fn value_addr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64 {
        let addr = Self::addr_to_value_addr(memory_mapping_helper, ptr);
        crate::remap_addr!(memory_mapping_helper, addr);
        u64::from_le_bytes(
            reconstruct_slice(addr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        )
    }

    fn value(addr: usize) -> &'a Self::Elem {
        fetch_value_common!(addr, typecast_bytes)
    }

    fn value_mut(addr: usize) -> &'a mut Self::Elem {
        fetch_value_common!(addr, typecast_bytes_mut)
    }
}

impl<'a> SpecMethods<'a> for &mut [u8] {
    type Elem = &'a [u8];

    fn addr_to_value_addr(_mmh: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64 {
        fetch_addr_to_value_addr_common(*ptr)
    }

    fn value_addr(mmh: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64 {
        let addr = Self::addr_to_value_addr(mmh, ptr);
        crate::remap_addr!(mmh, addr);
        let ptr = PtrType::PtrToValuePtr(addr);
        fetch_value_addr_common(ptr)
    }

    fn value(addr: usize) -> &'a Self::Elem {
        fetch_value_common!(addr, typecast_bytes)
    }

    fn value_mut(addr: usize) -> &'a mut Self::Elem {
        fetch_value_common!(addr, typecast_bytes_mut)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        println_typ_size,
        word_size::{
            addr_type::AddrType,
            common::{MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
            primitives::RcRefCellMemLayout,
            ptr_type::PtrType,
            slice::{reconstruct_slice, SliceFatPtr64, SliceFatPtr64Repr},
        },
    };
    use alloc::rc::Rc;
    use core::cell::RefCell;
    use fluentbase_sdk::debug_log;
    use solana_account_info::AccountInfo;
    use solana_pubkey::Pubkey;
    use solana_stable_layout::stable_vec::StableVec;
    use std::ops::Deref;

    #[test]
    fn structs_sizes_test() {
        println_typ_size!(&mut u64);
        println_typ_size!(RefCell<&mut u64>);
        println_typ_size!(RefCell<&mut [u8]>);
        println_typ_size!(Rc<RefCell<&mut u64>>);
        println_typ_size!(Rc<RefCell<&mut [u8]>>);
        println_typ_size!(&mut [u8]);
    }

    #[test]
    fn mut_u64_test() {
        let mut lamports: u64 = 13;
        struct TestStruct<'a> {
            lamports_rc: Rc<RefCell<&'a mut u64>>,
        }
        let test_struct = TestStruct {
            lamports_rc: Rc::new(RefCell::new(&mut lamports)),
        };
        assert_eq!(size_of::<TestStruct>(), 8);

        // let lamports_rc: Rc<RefCell<&u64>> = Rc::new(RefCell::new(&lamports));
        let rc_as_ptr = test_struct.lamports_rc.as_ptr() as u64;
        let lamports_rc_const_ptr: *const Rc<RefCell<&mut u64>> =
            &test_struct.lamports_rc as *const _;
        assert_eq!(
            &test_struct.lamports_rc as *const _ as usize,
            &test_struct as *const _ as usize
        );
        let rc_start_ptr = lamports_rc_const_ptr as u64;

        let mmh = MemoryMappingHelper::default();

        let rc_box_start_ptr = SliceFatPtr64Repr::ptr_elem_from_slice(
            reconstruct_slice(rc_start_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        );
        let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE as u64 * 3;
        let value_ptr = u64::from_le_bytes(
            reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        );
        let value = u64::from_le_bytes(
            reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        );
        assert_eq!(value, lamports);

        let container =
            RcRefCellMemLayout::<&mut u64>::new(mmh.clone(), PtrType::RcStartPtr(rc_start_ptr));
        let value = container.value::<true>();
        assert_eq!(value, &lamports);
        let value_vm_addr = container.value_addr::<true, true>();
        assert_eq!(
            u64::from_le_bytes(
                reconstruct_slice::<u8>(value_vm_addr as usize, 8)
                    .try_into()
                    .unwrap()
            ),
            lamports
        );

        let container = RcRefCellMemLayout::<&mut u64>::new(
            mmh.clone(),
            PtrType::RcBoxStartPtr(rc_box_start_ptr),
        );
        let value = container.value::<true>();
        assert_eq!(value, &lamports);
        let value_vm_addr = container.value_addr::<true, true>();
        assert_eq!(
            u64::from_le_bytes(
                reconstruct_slice::<u8>(value_vm_addr as usize, 8)
                    .try_into()
                    .unwrap()
            ),
            lamports
        );

        let container =
            RcRefCellMemLayout::<&mut u64>::new(mmh.clone(), PtrType::PtrToValuePtr(rc_as_ptr));
        let value = container.value::<true>();
        assert_eq!(value, &lamports);
    }

    #[test]
    fn mut_slice_u8_test() {
        let mut data = [1, 2, 3, 4, 5, 6].to_vec();
        let data_as_mut_slice: &mut [u8] = data.as_mut_slice();
        let data_as_mut_slice_ptr = data_as_mut_slice.as_mut_ptr();
        let data_as_mut_slice_addr = data_as_mut_slice_ptr as u64;
        struct TestStruct<'a> {
            data: Rc<RefCell<&'a mut [u8]>>,
        }
        let test_struct = TestStruct {
            data: Rc::new(RefCell::new(data_as_mut_slice)),
        };
        let test_struct_const_ptr = &test_struct as *const _;
        assert_eq!(size_of::<TestStruct>(), 8);

        let rc_const_ptr: *const Rc<RefCell<&mut [u8]>> = &test_struct.data as *const _;
        assert_eq!(
            &test_struct.data as *const _ as usize,
            &test_struct as *const _ as usize
        );
        let rc_start_ptr = rc_const_ptr as u64;

        let mm = MemoryMappingHelper::default();

        let container =
            RcRefCellMemLayout::<&mut [u8]>::new(mm.clone(), PtrType::RcStartPtr(rc_start_ptr));
        let value_addr = container.value_addr::<true, true>();
        assert_eq!(value_addr, data_as_mut_slice_addr);
        let addr_to_value_addr = container.addr_to_value_addr::<true, true>();
        let slice =
            SliceFatPtr64::<u8>::from_ptr_to_fat_ptr(addr_to_value_addr as usize, mm.clone());
        // let slice = reconstruct_slice::<u8>(value_addr as usize, data_as_mut_slice.len());
        assert_eq!(slice.to_vec_cloned(), data_as_mut_slice.to_vec());
        // let value = container.value::<true, true>();
        // assert_eq!(value, &data);

        // let container = RcRefCellMemLayout::<&mut [u8]>::new(
        //     mm.clone(),
        //     PtrType::RcBoxStartPtr(rc_box_start_ptr),
        // );
        // let value = container.value::<true, true>();
        // assert_eq!(value, &data);
        // let value_vm_addr = container.value_addr::<true, true>();
        // assert_eq!(
        //     u64::from_le_bytes(
        //         reconstruct_slice::<u8>(value_vm_addr as usize, 8)
        //             .try_into()
        //             .unwrap()
        //     ),
        //     data
        // );
        //
        // let container =
        //     RcRefCellMemLayout::<&mut [u8]>::new(mm.clone(), PtrType::PtrToValuePtr(rc_as_ptr));
        // let value = container.value::<true, true>();
        // assert_eq!(value, &data);
    }

    #[test]
    fn stable_vec_of_account_infos_mutations_test() {
        // type ItemType = u64;
        type ItemType<'a> = AccountInfo<'a>;
        type VecOfItemsType<'a> = StableVec<ItemType<'a>>;
        const ITEM_SIZE: usize = size_of::<ItemType>();
        const VEC_OF_ITEMS_TYPE_SIZE: usize = size_of::<VecOfItemsType>();
        debug_log!("ITEM_SIZE: {}", ITEM_SIZE);
        debug_log!("VEC_OF_ITEMS_TYPE_SIZE: {}", VEC_OF_ITEMS_TYPE_SIZE);

        let mmh = MemoryMappingHelper::default();

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

        let mut slice = SliceFatPtr64::<ItemType>::new::<false>(
            mmh.clone(),
            AddrType::Vm(items_original_fixed.as_ref().as_ptr() as u64),
            items_len,
        );
        debug_log!("vec_of_items_bytes_size {}", vec_of_items_bytes_size);
        let vec_of_items_start_ptr = (&items_original_fixed) as *const _ as u64;
        let first_item_start_ptr = items_original_fixed.as_ptr() as u64;
        debug_log!(
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
        debug_log!(
            "vec_of_items_as_raw_bytes ({}): {:x?}",
            vec_of_items_bytes_size,
            vec_of_items_as_raw_bytes
        );
        let items_as_raw_bytes = unsafe {
            alloc::slice::from_raw_parts(first_item_start_ptr as *const u8, items_only_bytes_size)
        };
        debug_log!(
            "items_as_raw_bytes ({}): {:x?}",
            items_only_bytes_size,
            items_as_raw_bytes
        );
        for idx in 0..slice.len() {
            let item_original = &items_original_fixed[idx];
            let item_original_ptr = item_original as *const _ as u64;
            let item_original_lamports_rc_ptr =
                item_original_ptr + crate::typ_size!(&Pubkey) as u64;
            let mem_layout = RcRefCellMemLayout::<&mut u64>::new(
                mmh.clone(),
                PtrType::RcStartPtr(item_original_lamports_rc_ptr),
            );
            let lamports_original_ref = item_original.lamports.borrow();
            let lamports_original = lamports_original_ref.deref();
            assert_eq!(*lamports_original, mem_layout.value::<false>())
        }
    }
}
