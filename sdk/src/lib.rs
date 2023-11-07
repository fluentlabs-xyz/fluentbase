// #![no_std]

#[cfg(feature = "runtime")]
mod runtime;
#[cfg(not(feature = "runtime"))]
mod rwasm;

#[cfg(feature = "runtime")]
pub use runtime::*;
#[cfg(not(feature = "runtime"))]
pub use rwasm::*;

// #[cfg(not(feature = "std"))]
// #[panic_handler]
// #[inline(always)]
// fn panic(info: &core::panic::PanicInfo) -> ! {
//     // if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
//     //     sys_write(panic_message.as_ptr() as u32, panic_message.len() as u32);
//     // }
//     // sys_panic();
//     loop {}
// }

// #[cfg(not(feature = "std"))]
// #[lang = "eh_personality"]
// extern "C" fn eh_personality() {}
