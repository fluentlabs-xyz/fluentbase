#![feature(lang_items)]
// #![no_std]

#[cfg(feature = "std")]
extern crate wee_alloc;

#[cfg(feature = "std")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[cfg(feature = "std")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
        sys_write(panic_message.as_ptr(), panic_message.len() as u32);
    }
    sys_panic();
    loop {}
}

pub mod binding;
pub use binding::*;

const HALT_CODE_EXIT: u32 = 0;
const HALT_CODE_PANIC: u32 = 1;

#[cfg(feature = "std")]
#[lang = "eh_personality"]
extern "C" fn eh_personality() {}