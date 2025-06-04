use crate::ptr_size::{
    common::{typecast_bytes, typecast_bytes_mut, FIXED_PTR_BYTE_SIZE},
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
    fn fetch_value(ptr: &PtrType) -> &'a Self::Elem;
    fn fetch_value_mut(ptr: &PtrType) -> &'a mut Self::Elem;
}

pub struct RcRefCellMemLayout<T> {
    pub ptr: PtrType,
    _phantom: PhantomData<T>,
}

impl<'a, T: SpecMethods<'a>> RcRefCellMemLayout<T> {
    pub fn new(ptr: PtrType) -> RcRefCellMemLayout<T> {
        Self {
            ptr,
            _phantom: Default::default(),
        }
    }

    pub fn value(&'a self) -> &'a T::Elem {
        T::fetch_value(&self.ptr)
    }

    pub fn value_mut(&'a self) -> &'a mut T::Elem {
        T::fetch_value_mut(&self.ptr)
    }
}

macro_rules! fetch_value_common {
    ($ptr:ident, $typecase_fn:ident) => {
        match $ptr {
            PtrType::PtrToValuePtr(ptr_to_value_ptr) => {
                let value_ptr = u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr.clone(), FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                $typecase_fn(
                    reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
            PtrType::RefCellStartPtr(ptr_to_refcell) => {
                let lamports_value_ptr = ptr_to_refcell + FIXED_PTR_BYTE_SIZE * 1;
                let lamports_ptr_value = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
                    reconstruct_slice(lamports_value_ptr, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                let value_ptr = u64::from_le_bytes(
                    reconstruct_slice(lamports_ptr_value as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                $typecase_fn(
                    reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
            PtrType::RcBoxStartPtr(rc_box_ptr) => {
                let ptr_to_ptr_to_value_ptr = rc_box_ptr + FIXED_PTR_BYTE_SIZE * 3;
                let ptr_to_value_ptr = SliceFatPtr64Repr::<1>::ptr_elem_from_slice(
                    reconstruct_slice(ptr_to_ptr_to_value_ptr, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                let value_ptr = u64::from_le_bytes(
                    reconstruct_slice(ptr_to_value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                );
                $typecase_fn(
                    reconstruct_slice(value_ptr as usize, FIXED_PTR_BYTE_SIZE)
                        .try_into()
                        .unwrap(),
                )
            }
        }
    };
}

impl<'a> SpecMethods<'a> for &mut u64 {
    type Elem = u64;

    fn fetch_value(ptr: &PtrType) -> &'a Self::Elem {
        fetch_value_common!(ptr, typecast_bytes)
    }

    fn fetch_value_mut(ptr: &PtrType) -> &'a mut Self::Elem {
        fetch_value_common!(ptr, typecast_bytes_mut)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        println_type_size,
        ptr_size::{
            common::FIXED_PTR_BYTE_SIZE,
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

        let container =
            RcRefCellMemLayout::<&mut u64>::new(PtrType::RcBoxStartPtr(lamports_rc_box_ptr));
        let value = container.value();
        assert_eq!(value, &lamports);

        let container =
            RcRefCellMemLayout::<&mut u64>::new(PtrType::RefCellStartPtr(lamports_rc_ptr));
        let value = container.value();
        assert_eq!(value, &lamports);

        let wrapper =
            RcRefCellMemLayout::<&mut u64>::new(PtrType::PtrToValuePtr(ptr_to_value_ptr as usize));
        let value = wrapper.value();
        assert_eq!(value, &lamports);

        let wrapper =
            RcRefCellMemLayout::<&mut u64>::new(PtrType::PtrToValuePtr(ptr_to_value_ptr as usize));
        let lamports_new_val: u64 = 43;
        *wrapper.value_mut() = lamports_new_val;
        let value = wrapper.value();
        assert_eq!(value, &lamports_new_val);
    }
}
