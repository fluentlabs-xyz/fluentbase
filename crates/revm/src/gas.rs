use revm::{
    context::context_interface::{
        cfg::GasParams, context::SStoreResult, journaled_state::StateLoad,
    },
    interpreter::Gas,
};

pub(crate) enum SstoreGasError<E> {
    OutOfFuel,
    Store(E),
}

/// Charges an SSTORE the way the EVM interpreter does, in the same order: the static cost, the
/// dynamic cost of the transition, the EIP-8037 state gas for a slot that leaves its original zero
/// (only once Amsterdam enables it), and finally the refund.
///
/// `eip8037_enabled` is the host's `is_amsterdam_eip8037_enabled()`; the CALL syscalls charge
/// their state gas the same way, so a fork that schedules EIP-8037 does not open a gap between
/// EVM bytecode and rWasm contracts writing the same slot.
pub(crate) fn sstore_gas<E>(
    gas: &mut Gas,
    gas_params: &GasParams,
    eip8037_enabled: bool,
    sstore: impl FnOnce(bool) -> Result<StateLoad<SStoreResult>, E>,
) -> Result<(), SstoreGasError<E>> {
    if gas.remaining() <= gas_params.call_stipend() {
        return Err(SstoreGasError::OutOfFuel);
    }
    if !gas.record_regular_cost(gas_params.sstore_static_gas()) {
        return Err(SstoreGasError::OutOfFuel);
    }

    let skip_cold = gas.remaining() < gas_params.cold_storage_cost();
    let state_load = sstore(skip_cold).map_err(SstoreGasError::Store)?;
    let gas_cost = gas_params.sstore_dynamic_gas(true, &state_load.data, state_load.is_cold);
    if !gas.record_regular_cost(gas_cost) {
        return Err(SstoreGasError::OutOfFuel);
    }

    // EIP-8037: state gas for new slot creation.
    if eip8037_enabled && !gas.record_state_cost(gas_params.sstore_state_gas(&state_load.data)) {
        return Err(SstoreGasError::OutOfFuel);
    }

    let gas_refund = gas_params.sstore_refund(true, &state_load.data);
    gas.record_refund(gas_refund);
    Ok(())
}
