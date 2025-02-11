use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_sol_types::{sol, SolValue};
use byteorder::BE;
use bytes::BytesMut;
use fluentbase_codec::{
    byteorder,
    encoder::{CompactABI, Encoder, SolidityABI, SolidityPackedABI},
    Codec,
};

mod structs;
mod tuples;
