use alloc::vec::Vec;
use fluentbase_codec::{define_codec_struct, BufferDecoder, Encoder};
use paste;

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
