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
mod evm;
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
    use crate::{evm::write_evm_panic_message, rwasm::RwasmContext};
    let native_sdk = RwasmContext {};
    // if cfg!(feature = "more-panic") {
    let panic_message = alloc::format!("{}", info).replace("\n", " ");
    write_evm_panic_message(&native_sdk, &panic_message);
    // } else {
    //     let panic_message = info
    //         .message()
    //         .as_str()
    //         .unwrap_or_else(|| &"can't resolve panic message");
    //     write_evm_panic_message(&native_sdk, &panic_message);
    // }
    native_sdk.exit(ExitCode::Panic.into_i32());
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
        unsafe { fluentbase_sdk::rwasm::_debug_log($msg.as_ptr(), $msg.len() as u32) }
        #[cfg(feature = "std")]
        println!("{}", $msg);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        debug_log!(msg);
    }};
}
