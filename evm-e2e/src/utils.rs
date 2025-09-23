use crate::runner::execute_test_suite;
use fluentbase_runtime::Runtime;
use fluentbase_sdk::{
    b256, keccak256,
    rwasm_core::{deserialize_wasmtime_module, CompilationConfig, Strategy},
};
use k256::ecdsa::SigningKey;
use revm::primitives::Address;
use std::{
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Recover the address from a private key (SigningKey).
pub fn recover_address(private_key: &[u8]) -> Option<Address> {
    let key = SigningKey::from_slice(private_key).ok()?;
    let public_key = key.verifying_key().to_encoded_point(false);
    Some(Address::from_raw_public_key(&public_key.as_bytes()[1..]))
}

pub(crate) fn run_e2e_test(test_path: &'static str) {
    let path = format!("./{}", test_path);
    let elapsed = Arc::new(Mutex::new(Duration::new(0, 0)));
    execute_test_suite(Path::new(path.as_str()), &elapsed, false, false).unwrap();
}

mod precompiled {
    include!(concat!(env!("OUT_DIR"), "/precompiled_module.rs"));
}

#[ctor::ctor]
fn warmup_wasmtime_modules() {
    let wasmtime_module = deserialize_wasmtime_module(
        CompilationConfig::default(),
        &precompiled::PRECOMPILED_RUNTIME_EVM_CWASM_MODULE,
    )
    .expect("failed to parse wasmtime module");
    let code_hash = keccak256(&precompiled::PRECOMPILED_RUNTIME_EVM_RWASM_MODULE);
    println!("precompiled evm runtime code hash: {:?}", code_hash);
    // 0xc62b88c3b842aea2c89fb1a69212d8e24925936873c4f52c38c4496eb6c491b2
    Runtime::warmup_strategy_raw(
        b256!("0xc62b88c3b842aea2c89fb1a69212d8e24925936873c4f52c38c4496eb6c491b2"),
        Strategy::Wasmtime {
            module: Rc::new(wasmtime_module),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::primitives::{address, hex};

    #[test]
    fn sanity_test() {
        assert_eq!(
            Some(address!("a94f5374fce5edbc8e2a8697c15331677e6ebf0b")),
            recover_address(&hex!(
                "45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8"
            ))
        )
    }
}
