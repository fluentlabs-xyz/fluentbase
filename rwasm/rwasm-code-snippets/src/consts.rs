pub const U64_MSBIT_IS_1: u64 = 0x8000000000000000;
pub const U64_ALL_BITS_ARE_1: u64 = 0xffffffffffffffff;
pub const U64_MAX_VAL: u64 = 0xffffffffffffffff;
pub const U64_ALL_BITS_ARE_1_EXCEPT_MSB: u64 = 0xffffffffffffffff - U64_MSBIT_IS_1;
pub const BYTE_MAX_VAL: u64 = 255;
pub const U64_LSBYTE_MASK: u64 = 255;
pub const BITS_IN_BYTE: u64 = 8;
pub const BYTES_IN_WASM_I64: u64 = 8;
