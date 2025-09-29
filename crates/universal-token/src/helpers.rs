use alloc::vec::Vec;
use bincode::config::{Configuration, Fixint, LittleEndian};

pub static BINCODE_CONFIG_DEFAULT: Configuration<LittleEndian, Fixint> = bincode::config::legacy();

pub fn serialize<T: bincode::enc::Encode>(
    entity: &T,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::encode_to_vec(entity, BINCODE_CONFIG_DEFAULT.clone())
}

pub fn deserialize<T: bincode::de::Decode<()>>(
    src: &[u8],
) -> Result<(T, usize), bincode::error::DecodeError> {
    bincode::decode_from_slice(src, BINCODE_CONFIG_DEFAULT.clone())
}
#[macro_export]
macro_rules! evm_exit {
    ($sdk:ident, $action:expr) => {
        if let Err(e) = $action {
            $sdk.evm_exit(e);
        };
    };
    ($sdk:ident, $action:expr, $err_num:ident) => {
        if $action == false {
            $sdk.evm_exit($err_num);
        };
    };
}
#[macro_export]
macro_rules! return_error_if_false {
    ($predicate:expr, $err:ident) => {
        if !$predicate {
            return Err($err);
        };
    };
}
#[macro_export]
macro_rules! return_error_if_true {
    ($predicate:expr, $err:ident) => {
        if $predicate {
            return Err($err);
        };
    };
}
