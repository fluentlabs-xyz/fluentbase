use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{
    is_delegated_runtime_address, Address, Bytes, EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES,
};
use fluentbase_testing::EvmTestingContext;

/// Instantiates non-delegated system-runtime addresses, including every crypto precompile.
///
/// The existing e2e suite supplies meaningful vectors for EVM, UST, Wasm, and several
/// precompiles. These minimal calls make the scheduled coverage job detect build/export drift for
/// every remaining address; behavior continues to be asserted by contract unit tests and Ethereum
/// fixtures.
#[test]
fn exercise_remaining_system_runtime_artifacts() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let caller = Address::repeat_byte(0x42);

    for address in EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES
        .iter()
        .copied()
        .filter(|address| !is_delegated_runtime_address(address))
    {
        let _ = ctx.call_evm_tx(caller, address, Bytes::new(), None, None);
    }
}
