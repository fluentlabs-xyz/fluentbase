pub const FIXED_MACHINE_WORD_BYTE_SIZE: usize = 8;
pub const FIXED_PTR_BYTE_SIZE: usize = FIXED_MACHINE_WORD_BYTE_SIZE;
pub const FAT_PTR64_ELEM_BYTE_SIZE: usize = FIXED_MACHINE_WORD_BYTE_SIZE;
pub const SLICE_FAT_PTR64_SIZE_BYTES: usize = FAT_PTR64_ELEM_BYTE_SIZE * 2;
pub const STABLE_VEC_FAT_PTR64_BYTE_SIZE: usize = FAT_PTR64_ELEM_BYTE_SIZE * 3;

#[inline(always)]
fn validate_typecast<T: Clone>(data: &[u8]) {
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
}

#[inline(always)]
pub fn typecast_bytes<T: Clone>(data: &[u8]) -> &T {
    validate_typecast::<T>(data);

    unsafe { &*(data.as_ptr() as *const T) }
}

#[inline(always)]
pub fn typecast_bytes_mut<T: Clone>(data: &[u8]) -> &mut T {
    validate_typecast::<T>(data);

    unsafe { &mut *(data.as_ptr() as *mut T) }
}

#[macro_export]
macro_rules! println_type_size {
    ($struct:ty) => {
        println!(
            "size_of::<{}>() = {}",
            core::any::type_name::<$struct>(),
            core::mem::size_of::<$struct>()
        )
    };
}
