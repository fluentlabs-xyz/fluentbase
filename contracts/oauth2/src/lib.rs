#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

use fluentbase_sdk::{system_entrypoint, ExitCode, SystemAPI};

pub fn main_entry<SDK: SystemAPI>(_sdk: &mut SDK) -> Result<(), ExitCode> {
    Err(ExitCode::UnreachableCodeReached)
}

system_entrypoint!(main_entry);
