#![cfg_attr(not(feature = "runtime"), no_std)]
#![feature(inherent_associated_types)]

extern crate alloc;
extern crate core;
extern crate lol_alloc;

#[cfg(feature = "runtime")]
mod runtime;
#[cfg(not(feature = "runtime"))]
mod rwasm;

#[cfg(feature = "runtime")]
pub use runtime::LowLevelSDK;
#[cfg(not(feature = "runtime"))]
pub use rwasm::LowLevelSDK;

#[cfg(feature = "evm")]
pub mod evm;

mod sdk;
pub use sdk::*;

mod types;
pub use types::*;

#[cfg(not(feature = "runtime"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
        LowLevelSDK::sys_write(panic_message.as_bytes());
    }
    LowLevelSDK::sys_halt(-71);
    loop {}
}

#[cfg(not(feature = "runtime"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: lol_alloc::AssumeSingleThreaded<lol_alloc::LeakingAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::LeakingAllocator::new()) };
