use fluentbase_sdk::U256;

pub const U256_LEN_BYTES: usize = size_of::<U256>();
pub const U256_LEN_BITS: usize = U256_LEN_BYTES * u8::BITS as usize;
