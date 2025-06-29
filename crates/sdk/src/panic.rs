#[cfg(target_arch = "wasm32")]
#[cfg(not(feature = "fast-panic"))]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    use crate::{native_api::NativeAPI, rwasm::RwasmContext, ExitCode};
    let panic_message = alloc::format!("{}", info.message());
    let native_sdk = RwasmContext {};
    native_sdk.write(panic_message.as_bytes());
    native_sdk.exit(ExitCode::Panic)
}

#[cfg(target_arch = "wasm32")]
#[cfg(feature = "fast-panic")]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    use crate::{native_api::NativeAPI, rwasm::RwasmContext, ExitCode};
    let Some(message) = info.message().as_str() else {
        // TODO(dmitry123): "how to support multiline panic messages?"
        core::hint::unreachable_unchecked();
    };
    let native_sdk = RwasmContext {};
    native_sdk.write(message.as_bytes());
    native_sdk.exit(ExitCode::Panic)
}
