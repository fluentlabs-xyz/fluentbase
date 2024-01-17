#![no_std]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

mod arithmetic;
mod bitwise;
pub(crate) mod common;
pub mod common_sp;
pub(crate) mod consts;
mod control;
mod host;
mod host_env;
mod memory;
mod stack;
mod system;
#[cfg(test)]
pub(crate) mod test_helper;
#[cfg(test)]
mod test_utils;
mod tests;
mod ts;
mod types;

// #[cfg(test)]
// #[ctor::ctor]
// fn log_init() {
//     let init_res =
//         env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
//             .try_init();
//     if let Err(_) = init_res {
//         // println!("failed to init logger");
//     }
// }
//
// #[panic_handler]
// #[inline(always)]
// fn panic(info: &core::panic::PanicInfo) -> ! {
//     if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
//         // SDK::sys_write(panic_message.as_bytes());
//         panic!("panic: {}", panic_message);
//     }
//     // SDK::sys_halt(-71);
//     panic!("panic");
//     loop {}
// }
//
// #[global_allocator]
// static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
