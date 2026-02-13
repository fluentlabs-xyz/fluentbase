use fluentbase_evm::gas::{
    CALLVALUE, COLD_ACCOUNT_ACCESS_COST, NEWACCOUNT, WARM_STORAGE_READ_COST,
};
use revm::{
    context::journaled_state::AccountLoad, interpreter::StateLoad, primitives::hardfork::SpecId,
};

/// Berlin warm and cold storage access cost for account access.
#[inline]
pub(crate) const fn account_warm_cold_cost(is_cold: bool) -> u64 {
    if is_cold {
        COLD_ACCOUNT_ACCESS_COST
    } else {
        WARM_STORAGE_READ_COST
    }
}

/// Berlin warm and cold storage access cost for account access.
///
/// If delegation is Some, add additional cost for delegation account load.
#[inline]
pub(crate) const fn warm_cold_cost_with_delegation(load: StateLoad<AccountLoad>) -> u64 {
    let mut gas = account_warm_cold_cost(load.is_cold);
    if let Some(is_cold) = load.data.is_delegate_account_cold {
        gas += account_warm_cold_cost(is_cold);
    }
    gas
}

/// Calculate call gas cost for the call instruction.
///
/// There is three types of gas.
/// * Account access gas. after berlin it can be cold or warm.
/// * Transfer value gas. If value is transferred and balance of target account is updated.
/// * If account is not existing and needs to be created. After Spurious dragon
///   this is only accounted if value is transferred.
///
/// account_load.is_empty will be accounted only if hardfork is SPURIOUS_DRAGON and
/// there is transfer value. [`bytecode::opcode::CALL`] use this field.
///
/// While [`bytecode::opcode::STATICCALL`], [`bytecode::opcode::DELEGATECALL`],
/// [`bytecode::opcode::CALLCODE`] need to have this field hardcoded to false
/// as they were present before SPURIOUS_DRAGON hardfork.
#[inline]
pub(crate) const fn call_cost(
    spec_id: SpecId,
    transfers_value: bool,
    account_load: StateLoad<AccountLoad>,
) -> u64 {
    let is_empty = account_load.data.is_empty;
    // Account access.
    let mut gas = if spec_id.is_enabled_in(SpecId::BERLIN) {
        warm_cold_cost_with_delegation(account_load)
    } else if spec_id.is_enabled_in(SpecId::TANGERINE) {
        // EIP-150: Gas cost changes for IO-heavy operations
        700
    } else {
        40
    };

    // Transfer value cost
    if transfers_value {
        gas += CALLVALUE;
    }

    // New account cost
    if is_empty {
        // EIP-161: State trie clearing (invariant-preserving alternative)
        if spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON) {
            // Account only if there is value transferred.
            if transfers_value {
                gas += NEWACCOUNT;
            }
        } else {
            gas += NEWACCOUNT;
        }
    }

    gas
}
