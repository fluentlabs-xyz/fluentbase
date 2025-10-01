mod context;
pub mod evm;
pub mod genesis;
pub mod hashes;
pub mod helpers;
mod preimage;
mod rwasm;
mod sdk;

pub use context::*;
use fluentbase_types::Address;
pub use genesis::*;
pub use preimage::*;
pub use rwasm::*;
pub use sdk::*;

pub fn is_delegated_runtime_address(address: &Address) -> bool {
    address == &PRECOMPILE_EVM_RUNTIME
        || address == &PRECOMPILE_SVM_RUNTIME
        || address == &PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME
        || address == &PRECOMPILE_WASM_RUNTIME
}

#[macro_export]
macro_rules! bn254_add_common_impl {
    ($p: ident, $q: ident, $action_p_eq_q: block, $action_rest: block) => {
        if *$p == [0u8; 64] {
            if *$q != [0u8; 64] {
                *$p = *$q;
            }
            return;
        } else if *$q == [0u8; 64] {
            return;
        } else if *$p == *$q {
            $action_p_eq_q
        } else {
            $action_rest
        }
    };
}
