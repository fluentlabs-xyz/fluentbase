#![cfg_attr(not(feature = "std"), no_std)]
#![warn(unused_crate_dependencies)]
#![allow(unused_imports)]
extern crate alloc;
extern crate core;

mod evm;

pub use evm::*;

#[cfg(not(feature = "std"))]
mod bindings;
#[macro_use]
pub mod macros;
pub mod contracts;
mod journal;
#[cfg(feature = "std")]
pub mod runtime;
#[cfg(not(feature = "std"))]
pub mod rwasm;
pub mod types;

#[cfg(not(feature = "std"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
unsafe fn panic(info: &core::panic::PanicInfo) -> ! {
    use self::bindings::{_exit, _write};
    #[cfg(nightly)]
    if let Some(panic_message) = info.message().and_then(|v| v.as_str()) {
        _write(panic_message.as_ptr(), panic_message.len() as u32);
    }
    #[cfg(not(nightly))]
    {
        let panic_message = alloc::format!("{}", info).replace("\n", " ");
        _write(panic_message.as_ptr(), panic_message.len() as u32);
    }
    _exit(ExitCode::Panic.into_i32());
}

#[cfg(not(feature = "std"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: fluentbase_types::HeapBaseAllocator = fluentbase_types::HeapBaseAllocator {};

pub use byteorder;
pub use fluentbase_codec as codec;
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;
