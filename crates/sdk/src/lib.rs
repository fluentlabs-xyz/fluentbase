#![cfg_attr(not(feature = "std"), no_std)]
#![warn(unused_crate_dependencies)]
#![allow(unused_imports)]
extern crate alloc;
extern crate core;

pub struct LowLevelSDK;

mod evm;

pub use evm::*;

mod account;
pub use account::*;
#[cfg(not(feature = "std"))]
mod bindings;
mod guest;
pub use guest::*;
#[macro_use]
pub mod macros;
mod allocator;
pub use allocator::{alloc_ptr, alloc_slice};
pub mod contracts;
#[cfg(feature = "std")]
mod runtime;
#[cfg(not(feature = "std"))]
mod rwasm;
pub mod types;
pub mod utils;

#[cfg(not(feature = "std"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn panic(info: &core::panic::PanicInfo) -> ! {
    #[cfg(nightly)]
    if let Some(panic_message) = info.message().and_then(|v| v.as_str()) {
        LowLevelSDK::write(panic_message.as_ptr(), panic_message.len() as u32);
    }
    #[cfg(not(nightly))]
    {
        let panic_message = alloc::format!("{}", info).replace("\n", " ");
        LowLevelSDK::write(panic_message.as_ptr(), panic_message.len() as u32);
    }
    LowLevelSDK::exit(ExitCode::Panic.into_i32());
}

#[cfg(not(feature = "std"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: allocator::HeapBaseAllocator = allocator::HeapBaseAllocator {};

pub use byteorder;
pub use fluentbase_codec as codec;
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;
