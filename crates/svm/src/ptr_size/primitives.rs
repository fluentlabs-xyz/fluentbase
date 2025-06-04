use crate::ptr_size::{
    common::{typecast_bytes, typecast_bytes_mut, MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
    slice_fat_ptr64::{reconstruct_slice, SliceFatPtr64Repr},
};
use core::{
    fmt::Debug,
    iter::Iterator,
    marker::PhantomData,
    ops::{Index, RangeBounds},
    slice::SliceIndex,
};

pub enum PtrType {
    PtrToValuePtr(usize),
    RcBoxStartPtr(usize),
    RefCellStartPtr(usize),
}

pub trait SpecMethods<'a> {
    type Elem;
    fn fetch_value_ptr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64;
    fn fetch_value(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> &'a Self::Elem;
    fn fetch_value_mut(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> &'a mut Self::Elem;
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
    ) -> RcRefCellMemLayout<T> {
        Self {
            memory_mapping_helper,
            ptr,
            _phantom: Default::default(),
        }
    }

    pub fn value_ptr(&'a self) -> u64 {
        T::fetch_value_ptr(&self.memory_mapping_helper, &self.ptr)
    }

    pub fn value(&'a self) -> &'a T::Elem {
        T::fetch_value(&self.memory_mapping_helper, &self.ptr)
    }

    pub fn value_mut(&'a self) -> &'a mut T::Elem {
        T::fetch_value_mut(&self.memory_mapping_helper, &self.ptr)
    }
}

macro_rules! fetch_value_ptr_common {
    ($mm:ident, $ptr:ident) => {
        match $ptr {
            PtrType::PtrToValuePtr(ptr_to_value_ptr) => u64::from_le_bytes(
                reconstruct_slice(ptr_to_value_ptr.clone(), FIXED_PTR_BYTE_SIZE)
                    .try_into()
                    .unwrap(),
            ),
            PtrType::RefCellStartPtr(ptr_to_refcell) => {
                let ptr_to_ptr_to_value_ptr = ptr_to_refcell + FIXED_PTR_BYTE_SIZE * 1;
                let ptr_value = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
                    reconstruct_slice(ptr_to_ptr_to_value_ptr, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                let ptr_to_value_ptr = $mm
                    .map_vm_addr_to_host(ptr_value, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap();
                let value_ptr = u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                $mm.map_vm_addr_to_host(value_ptr, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap()
            }
            PtrType::RcBoxStartPtr(rc_box_ptr) => {
                let ptr_to_ptr_to_value_ptr = rc_box_ptr + FIXED_PTR_BYTE_SIZE * 3;
                let ptr_value = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
                    reconstruct_slice(ptr_to_ptr_to_value_ptr, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                let ptr_to_value_ptr = $mm
                    .map_vm_addr_to_host(ptr_value, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap();
                let value_ptr = u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                $mm.map_vm_addr_to_host(value_ptr, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap()
            }
        };
    };
}

macro_rules! fetch_value_common {
    ($mm:ident, $ptr:ident, $typecase_fn:ident) => {{
        let value_ptr = fetch_value_ptr_common!($mm, $ptr);
        $typecase_fn(
            reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        )
    }};
}

impl<'a> SpecMethods<'a> for &mut u64 {
    type Elem = u64;

    fn fetch_value_ptr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64 {
        fetch_value_ptr_common!(memory_mapping_helper, ptr)
    }

    fn fetch_value(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> &'a Self::Elem {
        fetch_value_common!(memory_mapping_helper, ptr, typecast_bytes)
    }

    fn fetch_value_mut(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> &'a mut Self::Elem {
        fetch_value_common!(memory_mapping_helper, ptr, typecast_bytes_mut)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        println_type_size,
        ptr_size::{
            common::{MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
            primitives::{PtrType, RcRefCellMemLayout},
            slice_fat_ptr64::{reconstruct_slice, SliceFatPtr64Repr},
        },
    };
    use alloc::rc::Rc;
    use core::cell::RefCell;

    #[test]
    fn structs_sizes_test() {
        println_type_size!(&mut u64);
        println_type_size!(RefCell<&mut u64>);
        println_type_size!(Rc<RefCell<&mut u64>>);
    }

    #[test]
    fn rc_refcell_test() {
        let mut lamports: u64 = 13;
        let lamports_rc: Rc<RefCell<&mut u64>> = Rc::new(RefCell::new(&mut lamports));
        let lamports_rc_as_ptr = lamports_rc.as_ptr() as usize;
        let lamports_rc_ptr = (&lamports_rc) as *const _ as usize;
        let lamports_rc_box_ptr = lamports_rc_ptr - FIXED_PTR_BYTE_SIZE * 2;

        // RcBox {
        //     strong: usize,          // ref count
        //     weak: usize,            // weak count
        //     value: RefCell<&mut u64> {
        //         borrow: Cell<isize>, // borrow flag
        //         value: *mut &mut u64      // 8-byte pointer
        //     }
        // }

        let ptr_to_ptr_to_value_ptr = lamports_rc_box_ptr + FIXED_PTR_BYTE_SIZE * 3;
        let ptr_to_value_ptr = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
            reconstruct_slice(ptr_to_ptr_to_value_ptr, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        );
        assert_eq!(ptr_to_value_ptr as usize, lamports_rc_as_ptr);
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

        let container = RcRefCellMemLayout::<&mut u64>::new(
            MemoryMappingHelper::new(None, None),
            PtrType::RcBoxStartPtr(lamports_rc_box_ptr),
        );
        let value = container.value();
        assert_eq!(value, &lamports);
        let value_ptr = container.value_ptr();
        assert_eq!(
            u64::from_le_bytes(
                reconstruct_slice::<u8>(value_ptr as usize, 8)
                    .try_into()
                    .unwrap()
            ),
            lamports
        );

        let container = RcRefCellMemLayout::<&mut u64>::new(
            MemoryMappingHelper::new(None, None),
            PtrType::RefCellStartPtr(lamports_rc_ptr),
        );
        let value = container.value();
        assert_eq!(value, &lamports);

        let wrapper = RcRefCellMemLayout::<&mut u64>::new(
            MemoryMappingHelper::new(None, None),
            PtrType::PtrToValuePtr(ptr_to_value_ptr as usize),
        );
        let value = wrapper.value();
        assert_eq!(value, &lamports);
        let lamports_new_val: u64 = 43;
        *wrapper.value_mut() = lamports_new_val;
        let value = wrapper.value();
        assert_eq!(value, &lamports_new_val);
    }
}
