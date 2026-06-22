#[cfg(target_arch = "wasm32")]
#[cfg(not(feature = "fast-panic"))]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    use crate::evm::write_evm_panic_message;
    use fluentbase_types::{ExitCode, NativeAPI, RwasmContext};

    let panic_message = alloc::format!("{}", info.message());
    let native_sdk = RwasmContext {};
    write_evm_panic_message(&panic_message, |slice| native_sdk.write(slice));
    native_sdk.exit(ExitCode::Panic)
}

#[cfg(target_arch = "wasm32")]
#[cfg(feature = "fast-panic")]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    use crate::evm::write_evm_panic_message;
    use fluentbase_types::{ExitCode, NativeAPI, RwasmContext};

    let Some(message) = info.message().as_str() else {
        // TODO(dmitry123): "how to support multiline panic messages?"
        unreachable!("multiline or panic with args is not supported with fast-panic feature")
    };
    let native_sdk = RwasmContext {};
    write_evm_panic_message(message, |slice| native_sdk.write(slice));
    native_sdk.exit(ExitCode::Panic)
}
