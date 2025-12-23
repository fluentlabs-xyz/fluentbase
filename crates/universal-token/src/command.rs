use crate::consts::{
    SIG_ERC20_ALLOWANCE, SIG_ERC20_APPROVE, SIG_ERC20_BALANCE_OF, SIG_ERC20_MINT,
    SIG_ERC20_TRANSFER, SIG_ERC20_TRANSFER_FROM,
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    byteorder::BE,
    bytes::BytesMut,
    codec::{Codec, Encoder, SolidityABI},
    Address, ExitCode, U256,
};

pub trait UniversalTokenCommand
where
    Self: Encoder<BE, 32, true, false>,
{
    const SIGNATURE: u32;

    fn encode_for_send(&self, buffer: &mut Vec<u8>) {
        let mut bytes = BytesMut::new();
        SolidityABI::<Self>::encode(self, &mut bytes, 0).unwrap();
        let bytes = bytes.freeze();
        let signature_be = Self::SIGNATURE.to_be_bytes();
        buffer.extend_from_slice(&signature_be);
        buffer.extend_from_slice(&bytes);
    }

    fn try_decode(buf: &[u8]) -> Result<Self, ExitCode> {
        SolidityABI::<Self>::decode(&buf, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
    }
}

#[derive(Default, Debug, Codec)]
pub struct TransferCommand {
    pub to: Address,
    pub amount: U256,
}
impl UniversalTokenCommand for TransferCommand {
    const SIGNATURE: u32 = SIG_ERC20_TRANSFER;
}

#[derive(Default, Debug, Codec)]
pub struct TransferFromCommand {
    pub from: Address,
    pub to: Address,
    pub amount: U256,
}
impl UniversalTokenCommand for TransferFromCommand {
    const SIGNATURE: u32 = SIG_ERC20_TRANSFER_FROM;
}

#[derive(Default, Debug, Codec)]
pub struct ApproveCommand {
    pub spender: Address,
    pub amount: U256,
}
impl UniversalTokenCommand for ApproveCommand {
    const SIGNATURE: u32 = SIG_ERC20_APPROVE;
}

#[derive(Default, Debug, Codec)]
pub struct AllowanceCommand {
    pub owner: Address,
    pub spender: Address,
}
impl UniversalTokenCommand for AllowanceCommand {
    const SIGNATURE: u32 = SIG_ERC20_ALLOWANCE;
}

#[derive(Default, Debug, Codec)]
pub struct BalanceOfCommand {
    pub owner: Address,
}
impl UniversalTokenCommand for BalanceOfCommand {
    const SIGNATURE: u32 = SIG_ERC20_BALANCE_OF;
}

#[derive(Default, Debug, Codec)]
pub struct MintCommand {
    pub to: Address,
    pub amount: U256,
}
impl UniversalTokenCommand for MintCommand {
    const SIGNATURE: u32 = SIG_ERC20_MINT;
}
