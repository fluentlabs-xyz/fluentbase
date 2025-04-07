#[cfg(target_arch = "wasm32")]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    use crate::{native_api::NativeAPI, rwasm::RwasmContext, ExitCode};
    let panic_message = alloc::format!("{}", info.message());
    let native_sdk = RwasmContext {};
    native_sdk.write(panic_message.as_bytes());
    native_sdk.exit(ExitCode::Panic.into_i32())
}
