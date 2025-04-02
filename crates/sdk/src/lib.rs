#![cfg_attr(not(feature = "std"), no_std)]
#![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;

use hashbrown as _;

#[cfg(not(feature = "std"))]
mod bindings;
#[macro_use]
pub mod entrypoint;
pub mod constructor;
pub mod leb128;
#[cfg(feature = "std")]
pub mod runtime;
#[cfg(not(feature = "std"))]
pub mod rwasm;
pub mod shared;
pub mod storage;
#[cfg(feature = "std")]
pub mod testing;

#[cfg(not(feature = "std"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
unsafe fn panic(info: &core::panic::PanicInfo) -> ! {
    use crate::{native_api::NativeAPI, rwasm::RwasmContext};
    let panic_message = alloc::format!("{}", info.message());
    let native_sdk = RwasmContext {};
    native_sdk.write(panic_message.as_bytes());
    native_sdk.exit(ExitCode::Panic.into_i32())
}

#[cfg(not(feature = "std"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: fluentbase_types::HeapBaseAllocator = fluentbase_types::HeapBaseAllocator {};

pub use fluentbase_codec as codec;
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;

#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{
        #[cfg(target_arch = "wasm32")]
        unsafe { $crate::rwasm::_debug_log($msg.as_ptr(), $msg.len() as u32) }
        #[cfg(feature = "std")]
        println!("{}", $msg);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        debug_log!(msg);
    }};
}
