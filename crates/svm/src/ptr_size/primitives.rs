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

#[derive(Clone, Copy)]
pub enum PtrType {
    RcStartPtr(usize),
    RcBoxStartPtr(usize),
    PtrToValuePtr(usize),
}

pub trait SpecMethods<'a> {
    type Elem;
    fn fetch_value_vm_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64;
    fn fetch_value_host_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64;
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

    pub fn value_vm_addr(&'a self) -> u64 {
        T::fetch_value_vm_addr(&self.memory_mapping_helper, &self.ptr)
    }

    pub fn value_host_addr(&'a self) -> u64 {
        T::fetch_value_host_addr(&self.memory_mapping_helper, &self.ptr)
    }

    pub fn value(&'a self) -> &'a T::Elem {
        T::fetch_value(&self.memory_mapping_helper, &self.ptr)
    }

    pub fn value_mut(&'a self) -> &'a mut T::Elem {
        T::fetch_value_mut(&self.memory_mapping_helper, &self.ptr)
    }
}

macro_rules! remap_addr {
    ($mm:ident, $ptr:ident) => {
        let $ptr = $mm
            .map_vm_addr_to_host($ptr as u64, FIXED_PTR_BYTE_SIZE as u64)
            .unwrap() as usize;
    };
}

macro_rules! fetch_value_vm_addr_common {
    ($mm:ident, $addr:expr) => {
        match $addr {
            PtrType::PtrToValuePtr(ptr_to_value_ptr) => {
                remap_addr!($mm, ptr_to_value_ptr);
                u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
            PtrType::RcBoxStartPtr(rc_box_start_ptr) => {
                remap_addr!($mm, rc_box_start_ptr);
                let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE * 3;
                u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
            PtrType::RcStartPtr(rc_start_ptr) => {
                remap_addr!($mm, rc_start_ptr);
                let rc_box_ptr = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
                    reconstruct_slice(rc_start_ptr, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                ) as usize;
                remap_addr!($mm, rc_box_ptr);
                let ptr_to_value_ptr = rc_box_ptr + FIXED_PTR_BYTE_SIZE * 3;
                u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
        };
    };
}

macro_rules! fetch_value_common {
    ($mm:ident, $ptr:ident, $typecase_fn:ident) => {{
        let value_ptr = fetch_value_vm_addr_common!($mm, *$ptr);
        $typecase_fn(
            reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        )
    }};
}

impl<'a> SpecMethods<'a> for &mut u64 {
    type Elem = u64;

    fn fetch_value_vm_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64 {
        fetch_value_vm_addr_common!(memory_mapping_helper, *ptr)
    }

    fn fetch_value_host_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64 {
        let addr = Self::fetch_value_vm_addr(memory_mapping_helper, ptr);
        memory_mapping_helper
            .map_vm_addr_to_host(addr, FIXED_PTR_BYTE_SIZE as u64)
            .unwrap()
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
            common::{typecast_bytes, MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
            primitives::{PtrType, RcRefCellMemLayout},
            slice_fat_ptr64::{reconstruct_slice, SliceFatPtr64Repr},
        },
    };
    use alloc::rc::Rc;
    use core::{cell::RefCell, ptr::NonNull};

    #[test]
    fn structs_sizes_test() {
        println_type_size!(&mut u64);
        println_type_size!(RefCell<&mut u64>);
        println_type_size!(Rc<RefCell<&mut u64>>);
    }

    #[test]
    fn rc_refcell_test() {
        let mut lamports: u64 = 13;
        struct TestStruct<'a> {
            lamports_rc: Rc<RefCell<&'a mut u64>>,
        }
        let test_struct = TestStruct {
            lamports_rc: Rc::new(RefCell::new(&mut lamports)),
        };
        let test_struct_const_ptr = &test_struct as *const _;
        assert_eq!(size_of::<TestStruct>(), 8);

        // let lamports_rc: Rc<RefCell<&u64>> = Rc::new(RefCell::new(&lamports));
        let rc_as_ptr = test_struct.lamports_rc.as_ptr() as usize;
        let lamports_rc_const_ptr: *const Rc<RefCell<&mut u64>> =
            &test_struct.lamports_rc as *const _;
        assert_eq!(
            &test_struct.lamports_rc as *const _ as usize,
            &test_struct as *const _ as usize
        );
        let rc_start_ptr = lamports_rc_const_ptr as usize;

        let mm = MemoryMappingHelper::new(None, None);

        let rc_box_start_ptr = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
            reconstruct_slice(rc_start_ptr, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        ) as usize;
        let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE * 3;
        let value_ptr = u64::from_le_bytes(
            reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        );
        // assert_eq!(value_ptr as usize, lamports_rc_as_ptr);
        let value = u64::from_le_bytes(
            reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                .try_into()
                .unwrap(),
        );
        assert_eq!(value, lamports);

        let container =
            RcRefCellMemLayout::<&mut u64>::new(mm.clone(), PtrType::RcStartPtr(rc_start_ptr));
        let value = container.value();
        assert_eq!(value, &lamports);
        let value_vm_addr = container.value_vm_addr();
        assert_eq!(
            u64::from_le_bytes(
                reconstruct_slice::<u8>(value_vm_addr as usize, 8)
                    .try_into()
                    .unwrap()
            ),
            lamports
        );

        let container = RcRefCellMemLayout::<&mut u64>::new(
            mm.clone(),
            PtrType::RcBoxStartPtr(rc_box_start_ptr),
        );
        let value = container.value();
        assert_eq!(value, &lamports);
        let value_vm_addr = container.value_vm_addr();
        assert_eq!(
            u64::from_le_bytes(
                reconstruct_slice::<u8>(value_vm_addr as usize, 8)
                    .try_into()
                    .unwrap()
            ),
            lamports
        );

        let container =
            RcRefCellMemLayout::<&mut u64>::new(mm.clone(), PtrType::PtrToValuePtr(rc_as_ptr));
        let value = container.value();
        assert_eq!(value, &lamports);
    }
}
