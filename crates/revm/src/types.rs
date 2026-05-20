use crate::RwasmFrame;
use alloy_primitives::{Address, B256};
use revm::{
    bytecode::Bytecode,
    context::{journaled_state::JournalLoadError, Cfg, ContextTr, JournalTr},
    precompile::{PrecompileSpecId, Precompiles},
    primitives::hardfork::SpecId,
    Database,
};

/// Returns `true` if `address` is part of the executor's system-precompile set.
///
/// P.S: We exclude Fluent system precompiles from this list since it may affect
///  future runtime upgrades and cause redundant forks, because EVM precompiles have
///  enforced empty account state.
pub(crate) fn is_evm_system_precompile(spec: SpecId, address: &Address) -> bool {
    let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec));
    precompiles.contains(address)
}

/// Loads accounts and its delegate account.
///
/// The assumption is that warm gas is already deducted.
///
/// Returns `(regular_gas_cost, state_gas_cost, bytecode, code_hash)`.
/// `state_gas_cost` is non-zero only when creating a new empty account (EIP-8037).
#[inline]
#[allow(clippy::type_complexity)]
pub(crate) fn load_account_delegated<CTX: ContextTr>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    address: Address,
    transfers_value: bool,
    create_empty_account: bool,
) -> Result<(u64, u64, Bytecode, B256, Address), JournalLoadError<<CTX::Db as Database>::Error>> {
    let spec_id: SpecId = ctx.cfg().spec().into();
    let mut cost = 0;
    let mut state_gas_cost = 0;
    let is_berlin = spec_id.is_enabled_in(SpecId::BERLIN);
    let is_spurious_dragon = spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON);
    let remaining_gas = frame.interpreter.gas.remaining();

    let additional_cold_cost = ctx.gas_params().cold_account_additional_cost();
    let warm_storage_read_cost = ctx.gas_params().warm_storage_read_cost();

    let skip_cold_load = is_berlin && remaining_gas < additional_cold_cost;
    let mut account =
        ctx.journal_mut()
            .load_account_info_skip_cold_load(address, true, skip_cold_load)?;
    if is_berlin && account.is_cold {
        cost += additional_cold_cost;
    }
    let mut bytecode = account.code.clone().unwrap_or_default();
    let mut code_hash = account.code_hash();
    let mut bytecode_address = address;

    // EVM precompiles are "preloaded" and typically empty/stately-less. However, a precompile can also
    // be explicitly included in genesis, which changes its account states and affects CALL gas
    // accounting. Using CALL to invoke a precompile is usually pointless (precompiles are. Effectively stateless),
    // but some test suites require this edge case. Marking system precompiles as empty improves
    // EVM compatibility, even though it may. Cause certain unit tests to fail. We accept that trade-off.
    if create_empty_account && is_evm_system_precompile(spec_id, &address) {
        account.is_empty = true;
    }

    // New account cost, as account is empty, there is no delegated account, and we can return early.
    if create_empty_account && account.is_empty {
        cost += ctx
            .gas_params()
            .new_account_cost(is_spurious_dragon, transfers_value);
        if ctx.is_amsterdam_eip8037_enabled() && transfers_value {
            state_gas_cost += ctx.gas_params().new_account_state_gas();
        }
        return Ok((cost, state_gas_cost, bytecode, code_hash, bytecode_address));
    }

    // load delegate code if account is EIP-7702
    if let Some(eip7702_address) = account.code.as_ref().and_then(Bytecode::eip7702_address) {
        // EIP-7702 is enabled after berlin hardfork.
        cost += warm_storage_read_cost;
        if cost > remaining_gas {
            return Err(JournalLoadError::ColdLoadSkipped);
        }

        // skip cold load if there is enough gas to cover the cost.
        let skip_cold_load = remaining_gas < cost + additional_cold_cost;
        let delegate_account = ctx.journal_mut().load_account_info_skip_cold_load(
            eip7702_address,
            true,
            skip_cold_load,
        )?;

        if delegate_account.is_cold {
            cost += additional_cold_cost;
        }
        bytecode = delegate_account.code.clone().unwrap_or_default();
        code_hash = delegate_account.code_hash();
        bytecode_address = eip7702_address;
    }

    Ok((cost, state_gas_cost, bytecode, code_hash, bytecode_address))
}
