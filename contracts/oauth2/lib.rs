#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

use fluentbase_sdk::{system_entrypoint2, ExitCode, SharedAPI};

pub fn main_entry<SDK: SharedAPI>(_sdk: &mut SDK) -> Result<(), ExitCode> {
    Err(ExitCode::UnreachableCodeReached)
}

system_entrypoint2!(main_entry);
