#![feature(prelude_import)]
#[prelude_import]
use std::prelude::rust_2021::*;
#[macro_use]
extern crate std;
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use bytes::{Buf, BytesMut};
use codec2::{encoder::SolidityABI, error::CodecError};
use core::ops::Deref;
use fluentbase_sdk::{
    basic_entrypoint, derive::{router, signature, Contract},
    Bytes, SharedAPI,
};
struct ROUTER<SDK> {
    sdk: SDK,
}
impl<SDK> ROUTER<SDK> {
    pub fn new(sdk: SDK) -> Self {
        ROUTER { sdk }
    }
}
pub trait RouterAPI {
    fn greeting(&self, message: Bytes) -> Bytes;
}
impl<SDK: SharedAPI> ROUTER<SDK> {
    fn deploy(&self) {}
}
