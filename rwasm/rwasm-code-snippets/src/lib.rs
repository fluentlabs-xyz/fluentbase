#![no_std]

extern crate alloc;
extern crate core;
#[cfg(test)]
// #[macro_use]
// extern crate std;
extern crate wat;

mod arithmetic;
mod bitwise;
pub(crate) mod common;
pub(crate) mod consts;
mod host;
mod memory;
mod stack;
mod system;
#[cfg(test)]
pub(crate) mod test_helper;
#[cfg(test)]
mod test_utils;
mod tests;

// #[cfg(test)]
// #[ctor::ctor]
// fn log_init() {
//     let init_res =
//         env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
//             .try_init();
//     if let Err(_) = init_res {
//         println!("failed to init logger");
//     }
// }

#[cfg(any(
    feature = "arithmetic_add",
    feature = "arithmetic_mulmod",
    feature = "arithmetic_div",
    feature = "arithmetic_sdiv"
))]
#[panic_handler]
#[inline(always)]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
        // SDK::sys_write(panic_message.as_bytes());
        panic!("panic: {}", panic_message);
    }
    // SDK::sys_halt(-71);
    panic!("panic");
    loop {}
}

#[cfg(any(
    feature = "arithmetic_add",
    feature = "arithmetic_mulmod",
    feature = "arithmetic_div",
    feature = "arithmetic_sdiv"
))]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
