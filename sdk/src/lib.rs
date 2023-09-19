#![feature(lang_items)]
// #![no_std]

// #[cfg(not(feature = "std"))]
// extern crate wee_alloc;

// #[cfg(not(feature = "std"))]
// #[global_allocator]
// static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

// #[cfg(not(feature = "std"))]
// #[panic_handler]
// #[inline(always)]
// fn panic(info: &core::panic::PanicInfo) -> ! {
//     if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
//         evm_return_raw(panic_message.as_ptr(), panic_message.len() as u32);
//     }
//     sys_panic();
//     loop {}
// }
mod binding;
pub use binding::*;

const HALT_CODE_EXIT: u32 = 0;
const HALT_CODE_PANIC: u32 = 1;

// #[cfg(not(feature = "std"))]
// #[lang = "eh_personality"]
// extern "C" fn eh_personality() {}
