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

pub(crate) fn sstore_gas<E>(
    gas: &mut Gas,
    gas_params: &GasParams,
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

    let gas_refund = gas_params.sstore_refund(true, &state_load.data);
    gas.record_refund(gas_refund);
    Ok(())
}
