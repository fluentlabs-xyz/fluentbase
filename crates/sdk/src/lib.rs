#![cfg_attr(not(feature = "std"), no_std)]
#![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;
extern crate lol_alloc;

pub struct LowLevelSDK;

pub mod evm;
mod sdk;

pub use sdk::LowLevelAPI;

#[cfg(not(feature = "std"))]
mod bindings;
#[cfg(feature = "std")]
mod runtime;
#[cfg(not(feature = "std"))]
mod rwasm;
mod types;

pub use fluentbase_types::{Bytes20, Bytes32};

#[cfg(not(feature = "std"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let panic_message = alloc::format!("{}", info);
    LowLevelSDK::sys_write(panic_message.as_bytes());
    LowLevelSDK::sys_halt(-71);
    loop {}
}

#[cfg(not(feature = "std"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: lol_alloc::AssumeSingleThreaded<lol_alloc::LeakingAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::LeakingAllocator::new()) };
