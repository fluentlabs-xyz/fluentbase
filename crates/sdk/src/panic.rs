use crate::evm::write_evm_panic_message;
use fluentbase_types::{ExitCode, NativeAPI, RwasmContext};

#[cfg(not(feature = "fast-panic"))]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    let panic_message = alloc::format!("{}", info.message());
    let native_sdk = RwasmContext {};
    write_evm_panic_message(panic_message.as_str(), |message_bytes| {
        native_sdk.write(message_bytes)
    });
    native_sdk.exit(ExitCode::Panic)
}

#[cfg(feature = "fast-panic")]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    let Some(panic_message) = info.message().as_str() else {
        // TODO(dmitry123): How to support multiline panic messages? Not supported API by Rust yet w/o alloc.
        unreachable!("multiline or panic with args is not supported with fast-panic feature")
    };
    let native_sdk = RwasmContext {};
    write_evm_panic_message(panic_message.as_str(), |message_bytes| {
        native_sdk.write(message_bytes)
    });
    native_sdk.exit(ExitCode::Panic)
}
