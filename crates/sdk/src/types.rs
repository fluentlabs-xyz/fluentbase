mod context;
pub mod evm;
pub mod hashes;
pub mod helpers;
mod preimage;
mod rwasm;
mod sdk;

pub use context::*;
pub use preimage::*;
pub use rwasm::*;
pub use sdk::*;

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
