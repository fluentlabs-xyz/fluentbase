#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{func_entrypoint, ExitCode, SharedAPI};
use solana_ee_core::fluentbase_helpers::{exec_svm_message, process_svm_result};

func_entrypoint!(main);

pub fn main(mut sdk: impl SharedAPI) {
    let input = sdk.input();

    let result = exec_svm_message(&mut sdk, input);
    let (output, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error '{}' output '{:?}'",
            exit_code,
            output.as_ref()
        );
    }

    let out = output.as_ref();
    sdk.write(out);
}

#[cfg(test)]
mod tests {
    // find tests in 'solana-ee' project
}
