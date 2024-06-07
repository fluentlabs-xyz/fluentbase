#![cfg_attr(not(feature = "std"), no_std)]
#![warn(unused_crate_dependencies)]
#![allow(unused_imports)]
extern crate alloc;
extern crate core;
extern crate lol_alloc;

pub struct LowLevelSDK;

mod evm;

pub use evm::*;
mod sdk;

pub use sdk::{SharedAPI, SovereignAPI};

mod account;
pub use account::*;
#[cfg(not(feature = "std"))]
mod bindings;
mod jzkt;
pub use jzkt::*;
#[macro_use]
pub mod macros;
#[cfg(feature = "std")]
mod runtime;
#[cfg(not(feature = "std"))]
mod rwasm;
mod types;
mod utils;

pub use types::*;
pub use utils::*;

#[cfg(not(feature = "std"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let panic_message = alloc::format!("{}", info).replace("\n", " ");
    LowLevelSDK::write(panic_message.as_bytes());
    LowLevelSDK::exit(fluentbase_types::ExitCode::Panic.into_i32());
}

#[cfg(not(feature = "std"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: lol_alloc::AssumeSingleThreaded<lol_alloc::LeakingAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::LeakingAllocator::new()) };

pub use fluentbase_sdk_derive::{derive_keccak256_id, derive_solidity_router, sol_signature};

pub mod codec {
    pub use fluentbase_codec::*;
}

pub fn alloc_ptr(len: usize) -> *mut u8 {
    use alloc::alloc::{alloc, Layout};
    unsafe { alloc(Layout::from_size_align_unchecked(len, 8)) }
}

pub fn alloc_slice<'a>(len: usize) -> &'a mut [u8] {
    use core::ptr;
    unsafe { &mut *ptr::slice_from_raw_parts_mut(alloc_ptr(len), len) }
}

