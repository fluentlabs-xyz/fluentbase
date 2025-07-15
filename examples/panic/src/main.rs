#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(unused)]
extern crate alloc;
// extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, ExitCode, SharedAPI};

#[inline(always)]
pub unsafe fn handle_panic_info(info: &core::panic::PanicInfo) -> ! {
    use fluentbase_sdk::{native_api::NativeAPI, rwasm::RwasmContext, ExitCode};
    let panic_message = alloc::format!("{}", info.message());
    let native_sdk = RwasmContext {};
    native_sdk.write(panic_message.as_bytes());
    native_sdk.exit(ExitCode::Panic)
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    panic!("it's panic time")
    // use fluentbase_sdk::{native_api::NativeAPI, rwasm::RwasmContext, ExitCode};
    // let panic_message = alloc::format!("{}", core::hint::black_box("it's panic time"));
    // let native_sdk = RwasmContext {};
    // native_sdk.write(panic_message.as_bytes());
    // native_sdk.exit(ExitCode::Panic)
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::HostTestingContext;

    #[should_panic(expected = "it's panic time")]
    #[test]
    fn tets_contract_works() {
        let sdk = HostTestingContext::default();
        main_entry(sdk);
    }
}
