use crate::ptr_size::{
    common::{typecast_bytes, typecast_bytes_mut, MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
    slice_fat_ptr64::{reconstruct_slice, SliceFatPtr64Repr},
};
use core::{
    marker::PhantomData,
    ops::{Index, RangeBounds},
    slice::SliceIndex,
};

macro_rules! map_addr {
    ($mm:expr, $ptr:ident) => {
        $mm.map_vm_addr_to_host($ptr as u64, FIXED_PTR_BYTE_SIZE as u64)
            .unwrap() as usize
    };
}

macro_rules! remap_addr {
    ($mm:expr, $ptr:ident) => {
        let $ptr = map_addr!($mm, $ptr);
    };
}

#[derive(Clone, Copy)]
pub enum PtrType {
    RcStartPtr(usize),
    RcBoxStartPtr(usize),
    PtrToValuePtr(usize),
}

impl PtrType {
    pub fn as_mut(&mut self) -> &mut usize {
        match self {
            PtrType::RcStartPtr(v) => v,
            PtrType::RcBoxStartPtr(v) => v,
            PtrType::PtrToValuePtr(v) => v,
        }
    }
    pub fn inner_value(&self) -> usize {
        match self {
            PtrType::RcStartPtr(v) => v.clone(),
            PtrType::RcBoxStartPtr(v) => v.clone(),
            PtrType::PtrToValuePtr(v) => v.clone(),
        }
    }
    pub fn map<F: Fn(&mut usize)>(mut self, f: F) -> Self {
        f(self.as_mut());
        self
    }
}

pub trait SpecMethods<'a> {
    type Elem;
    fn fetch_addr_to_value_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64;
    fn fetch_value_addr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64;
    fn fetch_value(addr: usize) -> &'a Self::Elem;
    fn fetch_value_mut(addr: usize) -> &'a mut Self::Elem;
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

    pub fn addr_to_value_addr<const PRE_REMAP: bool, const POST_REMAP: bool>(&'a self) -> u64 {
        let ptr = if PRE_REMAP {
            self.ptr.map(|v| {
                *v = self
                    .memory_mapping_helper
                    .map_vm_addr_to_host((*v) as u64, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap() as usize;
            })
        } else {
            self.ptr
        };
        let addr = T::fetch_addr_to_value_addr(&self.memory_mapping_helper, &ptr) as usize;
        if POST_REMAP {
            map_addr!(self.memory_mapping_helper, addr) as u64
        } else {
            addr as u64
        }
    }

    pub fn value_addr<const PRE_REMAP: bool, const POST_REMAP: bool>(&'a self) -> u64 {
        let ptr = if PRE_REMAP {
            self.ptr.map(|v| {
                *v = self
                    .memory_mapping_helper
                    .map_vm_addr_to_host((*v) as u64, FIXED_PTR_BYTE_SIZE as u64)
                    .unwrap() as usize;
            })
        } else {
            self.ptr
        };
        let addr = T::fetch_value_addr(&self.memory_mapping_helper, &ptr) as usize;
        if POST_REMAP {
            map_addr!(self.memory_mapping_helper, addr) as u64
        } else {
            addr as u64
        }
    }

    pub fn value<const PRE_REMAP: bool, const POST_REMAP: bool>(&'a self) -> &'a T::Elem {
        T::fetch_value(self.value_addr::<PRE_REMAP, POST_REMAP>() as usize)
    }

    pub fn value_mut<const PRE_REMAP: bool, const POST_REMAP: bool>(&'a self) -> &'a mut T::Elem {
        T::fetch_value_mut(self.value_addr::<PRE_REMAP, POST_REMAP>() as usize)
    }
}

macro_rules! fetch_addr_to_value_addr_common {
    ($mm:ident, $addr:expr) => {
        match $addr {
            PtrType::PtrToValuePtr(ptr_to_value_ptr) => ptr_to_value_ptr,
            PtrType::RcBoxStartPtr(rc_box_start_ptr) => {
                let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE * 3;
                ptr_to_value_ptr
            }
            PtrType::RcStartPtr(rc_start_ptr) => {
                let rc_box_ptr = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
                    reconstruct_slice(rc_start_ptr, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                ) as usize;
                remap_addr!($mm, rc_box_ptr);
                let ptr_to_value_ptr = rc_box_ptr + FIXED_PTR_BYTE_SIZE * 3;
                ptr_to_value_ptr
            }
        };
    };
}

macro_rules! fetch_value_addr_common {
    ($mm:ident, $addr:expr) => {
        match $addr {
            PtrType::PtrToValuePtr(ptr_to_value_ptr) => u64::from_le_bytes(
                reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                    .try_into()
                    .unwrap(),
            ),
            PtrType::RcBoxStartPtr(rc_box_start_ptr) => {
                let ptr_to_value_ptr = rc_box_start_ptr + FIXED_PTR_BYTE_SIZE * 3;
                u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
            PtrType::RcStartPtr(rc_start_ptr) => {
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

    fn fetch_addr_to_value_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64 {
        fetch_addr_to_value_addr_common!(memory_mapping_helper, *ptr) as u64
    }

    fn fetch_value_addr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64 {
        fetch_value_addr_common!(memory_mapping_helper, *ptr)
    }

    fn fetch_value(addr: usize) -> &'a Self::Elem {
        fetch_value_common!(addr, typecast_bytes)
    }

    fn fetch_value_mut(addr: usize) -> &'a mut Self::Elem {
        fetch_value_common!(addr, typecast_bytes_mut)
    }
}

impl<'a> SpecMethods<'a> for &mut [u8] {
    type Elem = &'a [u8];

    fn fetch_addr_to_value_addr(
        memory_mapping_helper: &'a MemoryMappingHelper<'a>,
        ptr: &PtrType,
    ) -> u64 {
        fetch_addr_to_value_addr_common!(memory_mapping_helper, *ptr) as u64
    }

    fn fetch_value_addr(memory_mapping_helper: &'a MemoryMappingHelper<'a>, ptr: &PtrType) -> u64 {
        fetch_value_addr_common!(memory_mapping_helper, *ptr)
    }

    fn fetch_value(addr: usize) -> &'a Self::Elem {
        fetch_value_common!(addr, typecast_bytes)
    }

    fn fetch_value_mut(addr: usize) -> &'a mut Self::Elem {
        fetch_value_common!(addr, typecast_bytes_mut)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        println_typ_size,
        ptr_size::{
            common::{MemoryMappingHelper, FIXED_PTR_BYTE_SIZE},
            primitives::{PtrType, RcRefCellMemLayout},
            slice_fat_ptr64::{reconstruct_slice, SliceFatPtr64, SliceFatPtr64Repr},
        },
    };
    use alloc::rc::Rc;
    use core::cell::RefCell;

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
            reconstruct_slice(ptr_to_value_ptr, FIXED_PTR_BYTE_SIZE)
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
            RcRefCellMemLayout::<&mut u64>::new(mm.clone(), PtrType::RcStartPtr(rc_start_ptr));
        let value = container.value::<true, true>();
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
            mm.clone(),
            PtrType::RcBoxStartPtr(rc_box_start_ptr),
        );
        let value = container.value::<true, true>();
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
            RcRefCellMemLayout::<&mut u64>::new(mm.clone(), PtrType::PtrToValuePtr(rc_as_ptr));
        let value = container.value::<true, true>();
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
        let rc_start_ptr = rc_const_ptr as usize;

        let mm = MemoryMappingHelper::new(None, None);

        let container =
            RcRefCellMemLayout::<&mut [u8]>::new(mm.clone(), PtrType::RcStartPtr(rc_start_ptr));
        let value_addr = container.value_addr::<true, true>();
        assert_eq!(value_addr, data_as_mut_slice_addr);
        let addr_to_value_addr = container.addr_to_value_addr::<true, true>();
        let slice = SliceFatPtr64::<u8>::from_ptr_to_fat_ptr(
            addr_to_value_addr as usize,
            mm.memory_mapping(),
        );
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
}
