use fluentbase_sdk::{BlockContextReader, SharedAPI, TxContextReader};
use revm_interpreter::{gas, primitives::U256, push, push_b256, try_push, Interpreter};

/// EIP-1344: ChainID opcode
pub fn chainid<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    push!(interpreter, U256::from(sdk.context().block_chain_id()));
}

pub fn coinbase<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    push_b256!(interpreter, sdk.context().block_coinbase().into_word());
}

pub fn timestamp<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    try_push!(interpreter, sdk.context().block_timestamp());
}

pub fn block_number<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    try_push!(interpreter, sdk.context().block_number());
}

pub fn difficulty<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    push_b256!(interpreter, sdk.context().block_prev_randao());
}

pub fn gaslimit<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    try_push!(interpreter, sdk.context().block_gas_limit());
}

pub fn gasprice<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    try_push!(interpreter, sdk.context().tx_gas_price());
}

/// EIP-3198: BASEFEE opcode
pub fn basefee<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    try_push!(interpreter, sdk.context().block_base_fee());
}

pub fn origin<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    push_b256!(interpreter, sdk.context().tx_origin().into_word());
}

// EIP-4844: Shard Blob Transactions
pub fn blob_hash<SDK: SharedAPI>(_interpreter: &mut Interpreter, _sdk: &mut SDK) {
    todo!("not supported")
}

/// EIP-7516: BLOBBASEFEE opcode
pub fn blob_basefee<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    push!(interpreter, sdk.context().block_base_fee());
}
