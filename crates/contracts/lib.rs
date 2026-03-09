//! Build-time embedded system contract artifacts generated into `BUILD_OUTPUTS`.
include!(concat!(env!("OUT_DIR"), "/build_output.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_create2_factory_output() {
        assert_eq!(
            FLUENTBASE_CONTRACTS_CREATE2_FACTORY.name,
            "fluentbase_contracts_create2_factory"
        );
        assert!(!FLUENTBASE_CONTRACTS_CREATE2_FACTORY.wasm_bytecode.is_empty());
    }
}

