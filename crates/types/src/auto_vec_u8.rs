use alloc::vec::Vec;
use alloy_primitives::Bytes;
#[cfg(not(feature = "std"))]
use revm_helpers::reusable_pool::global::VecU8;

#[cfg(feature = "std")]
pub type BytesOrVecU8 = Bytes;
#[cfg(not(feature = "std"))]
pub type BytesOrVecU8 = VecU8;

#[cfg(feature = "std")]
pub type Vecu8OrVecU8 = Vec<u8>;
#[cfg(not(feature = "std"))]
pub type Vecu8OrVecU8 = VecU8;

pub fn copy_from_slice(bytes: &[u8]) -> BytesOrVecU8 {
    #[cfg(feature = "std")]
    {
        Bytes::copy_from_slice(bytes)
    }
    #[cfg(not(feature = "std"))]
    {
        VecU8::try_from_slice(bytes).expect("enough cap")
    }
}

pub fn new() -> BytesOrVecU8 {
    #[cfg(feature = "std")]
    {
        Bytes::new()
    }
    #[cfg(not(feature = "std"))]
    {
        VecU8::default_for_reuse()
    }
}
