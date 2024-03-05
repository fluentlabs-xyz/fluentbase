use alloc::vec::Vec;
use fluentbase_codec::{define_codec_struct, BufferDecoder, Encoder};

define_codec_struct! {
    pub struct CoreInput {
        method_id: u32,
        method_data: Vec<u8>,
    }
}

impl CoreInput {
    pub fn new(method_id: u32, method_data: Vec<u8>) -> Self {
        CoreInput {
            method_id,
            method_data,
        }
    }
}

pub const CREATE_METHOD_ID: u32 = 1;
define_codec_struct! {
    pub struct CreateMethodInput {
        value32: [u8; 32],
        code: Vec<u8>,
        gas_limit: u32,
    }
}

impl CreateMethodInput {
    pub fn new(value32: [u8; 32], code: Vec<u8>, gas_limit: u32) -> Self {
        CreateMethodInput {
            value32,
            code,
            gas_limit,
        }
    }
}
//
// define_codec_struct! {
//     pub struct Create2MethodData {}
// }
//
// define_codec_struct! {
//     pub struct CallMethodData {}
// }
