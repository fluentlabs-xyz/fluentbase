use crate::{as_usize_saturated, gas, pop_top, push, push_b256, try_push, EVM};
use fluentbase_sdk::{ContextReader, SharedAPI, U256};

/// EIP-1344: ChainID opcode
pub fn chainid<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, U256::from(evm.sdk.context().block_chain_id()));
}

pub fn coinbase<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push_b256!(evm, evm.sdk.context().block_coinbase().into_word());
}

pub fn timestamp<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    try_push!(evm, evm.sdk.context().block_timestamp());
}

pub fn block_number<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    try_push!(evm, evm.sdk.context().block_number());
}

pub fn difficulty<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push_b256!(evm, evm.sdk.context().block_prev_randao());
}

pub fn gaslimit<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    try_push!(evm, evm.sdk.context().block_gas_limit());
}

pub fn gasprice<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    try_push!(evm, evm.sdk.context().tx_gas_price());
}

/// EIP-3198: BASEFEE opcode
pub fn basefee<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    try_push!(evm, evm.sdk.context().block_base_fee());
}

pub fn origin<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push_b256!(evm, evm.sdk.context().tx_origin().into_word());
}

// EIP-4844: Shard Blob Transactions
pub fn blob_hash<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, index);
    let _i = as_usize_saturated!(index);
    // TODO(dmitry123): "we don't support blob hashes"
    *index = U256::ZERO;
}

/// EIP-7516: BLOBBASEFEE opcode
pub fn blob_basefee<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, evm.sdk.context().block_base_fee());
}
