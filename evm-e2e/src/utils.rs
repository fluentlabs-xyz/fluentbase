use crate::runner::execute_test_suite;
use k256::ecdsa::SigningKey;
use revm::primitives::Address;
use std::{
    path::Path,
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

#[cfg(feature = "wasmtime")]
mod precompiled {
    include!(concat!(env!("OUT_DIR"), "/precompiled_module.rs"));
}

#[cfg(feature = "wasmtime")]
#[ctor::ctor]
fn warmup_wasmtime_modules() {
    use fluentbase_runtime::RuntimeExecutor;
    use fluentbase_sdk::rwasm_core::{
        wasmtime::deserialize_wasmtime_module, CompilationConfig, RwasmModule,
    };
    for (name, wasmtime_module) in precompiled::PRECOMPILED_MODULES {
        let module = deserialize_wasmtime_module(CompilationConfig::default().with_consume_fuel(false), wasmtime_module)
            .expect("failed to parse wasmtime module");
        let contract = fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS
            .values()
            .find(|v| v.name.contains(name))
            .expect("missing genesis contract");
        let (rwasm_module, _) = RwasmModule::new(contract.rwasm_bytecode.as_ref());
        fluentbase_runtime::default_runtime_executor().warmup_wasmtime(
            rwasm_module,
            module,
            contract.rwasm_bytecode_hash,
        );
    }
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
