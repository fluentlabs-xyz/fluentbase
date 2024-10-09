#![cfg_attr(not(feature = "std"), no_std)]
#![feature(panic_info_message)]
#![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;

#[cfg(not(feature = "std"))]
mod bindings;
#[macro_use]
pub mod macros;
pub mod journal;
#[cfg(feature = "std")]
pub mod runtime;
#[cfg(not(feature = "std"))]
pub mod rwasm;
pub mod shared;
pub mod storage;
pub mod types;

#[cfg(not(feature = "std"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
unsafe fn panic(info: &core::panic::PanicInfo) -> ! {
    use self::bindings::{_exit, _write};
    if cfg!(feature = "more-panic") {
        let panic_message = alloc::format!("{}", info).replace("\n", " ");
        _write(panic_message.as_ptr(), panic_message.len() as u32);
    } else {
        let panic_message = info
            .message()
            .as_str()
            .unwrap_or_else(|| &"can't resolve panic message");
        _write(panic_message.as_ptr(), panic_message.len() as u32);
    }
    _exit(ExitCode::Panic.into_i32());
}

#[cfg(not(feature = "std"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: fluentbase_types::HeapBaseAllocator = fluentbase_types::HeapBaseAllocator {};

pub use fluentbase_codec as codec;
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;
