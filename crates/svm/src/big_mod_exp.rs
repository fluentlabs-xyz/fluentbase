use crate::word_size::{
    common::MemoryMappingHelper,
    slice::{RetVal, SpecMethods},
};
use alloc::{vec, vec::Vec};

#[repr(C)]
#[derive(Copy, Clone)]
pub struct BigModExpParams {
    pub base: u64,
    pub base_len: u64,
    pub exponent: u64,
    pub exponent_len: u64,
    pub modulus: u64,
    pub modulus_len: u64,
}

impl<'a> SpecMethods<'a> for BigModExpParams {
    const ITEM_SIZE_BYTES: usize = size_of::<Self>();

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        _memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self>
    where
        Self: Sized,
    {
        const COMPONENT_BYTES_SIZE: usize = size_of::<u64>();
        const TOTAL_BYTES_SIZE: usize = COMPONENT_BYTES_SIZE * 6;
        assert_eq!(byte_repr.len(), TOTAL_BYTES_SIZE, "incorrect byte repr");
        #[inline(always)]
        fn get_at_idx(byte_repr: &[u8], idx: usize) -> u64 {
            u64::from_le_bytes(
                byte_repr[COMPONENT_BYTES_SIZE * idx..COMPONENT_BYTES_SIZE * (idx + 1)]
                    .try_into()
                    .unwrap(),
            )
        }
        RetVal::Instance(BigModExpParams {
            base: get_at_idx(byte_repr, 0),
            base_len: get_at_idx(byte_repr, 1),
            exponent: get_at_idx(byte_repr, 2),
            exponent_len: get_at_idx(byte_repr, 3),
            modulus: get_at_idx(byte_repr, 4),
            modulus_len: get_at_idx(byte_repr, 5),
        })
    }
}
