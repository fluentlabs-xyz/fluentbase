use revm_interpreter::primitives::SpecId;

pub(crate) const BASE_SPEC: SpecId = SpecId::PRAGUE;

pub(crate) unsafe fn read_i16(ptr: *const u8) -> i16 {
    i16::from_be_bytes(core::slice::from_raw_parts(ptr, 2).try_into().unwrap())
}

pub(crate) unsafe fn read_u16(ptr: *const u8) -> u16 {
    u16::from_be_bytes(core::slice::from_raw_parts(ptr, 2).try_into().unwrap())
}
