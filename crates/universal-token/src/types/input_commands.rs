use crate::common::{fixed_bytes_from_u256, sig_to_bytes, u256_from_slice_try};
use crate::consts::{
    ERR_INVALID_RECIPIENT, ERR_MALFORMED_INPUT, SIG_ALLOWANCE, SIG_APPROVE, SIG_BALANCE_OF,
    SIG_MINT, SIG_TRANSFER, SIG_TRANSFER_FROM,
};
use crate::storage::{ADDRESS_LEN_BYTES, U256_LEN_BYTES};
use alloc::vec::Vec;
use fluentbase_sdk::{Address, U256};

pub trait Encodable: Sized {
    const SIG: u32;
    const LEN: usize;
    fn encode(&self, dst: &mut Vec<u8>);
    fn encode_for_send(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(size_of::<u32>() + Self::LEN);
        dst.extend_from_slice(&sig_to_bytes(Self::SIG));
        self.encode(dst);
    }
    fn try_decode(data: &[u8]) -> Result<Self, u32>;
    fn validate_input_for_decode(input: &[u8]) -> Result<(), u32> {
        if input.len() != Self::LEN {
            return Err(ERR_MALFORMED_INPUT);
        }
        Ok(())
    }
}

pub struct TransferCommand {
    pub to: Address,
    pub amount: U256,
}
impl Encodable for TransferCommand {
    const SIG: u32 = SIG_TRANSFER;
    const LEN: usize = ADDRESS_LEN_BYTES + U256_LEN_BYTES;

    fn encode(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(Self::LEN);
        dst.extend_from_slice(self.to.as_slice());
        dst.extend_from_slice(&fixed_bytes_from_u256(&self.amount));
    }

    fn try_decode(input: &[u8]) -> Result<Self, u32> {
        const TO_OFFSET: usize = 0;
        const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
        Self::validate_input_for_decode(input)?;
        let to = Address::from_slice(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]);
        let amount =
            u256_from_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES]).unwrap();

        Ok(Self { to, amount })
    }
}

pub struct TransferFromCommand {
    pub from: Address,
    pub to: Address,
    pub amount: U256,
}
impl Encodable for TransferFromCommand {
    const SIG: u32 = SIG_TRANSFER_FROM;
    const LEN: usize = ADDRESS_LEN_BYTES + ADDRESS_LEN_BYTES + U256_LEN_BYTES;

    fn encode(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(Self::LEN);
        dst.extend_from_slice(self.from.as_slice());
        dst.extend_from_slice(self.to.as_slice());
        dst.extend_from_slice(&fixed_bytes_from_u256(&self.amount));
    }

    fn try_decode(input: &[u8]) -> Result<Self, u32> {
        const FROM_OFFSET: usize = 0;
        const TO_OFFSET: usize = FROM_OFFSET + ADDRESS_LEN_BYTES;
        const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
        Self::validate_input_for_decode(input)?;
        let from = Address::from_slice(&input[FROM_OFFSET..FROM_OFFSET + ADDRESS_LEN_BYTES]);
        let to = Address::from_slice(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]);
        let amount =
            u256_from_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES]).unwrap();

        Ok(Self { from, to, amount })
    }
}

pub struct ApproveCommand {
    pub owner: Address,
    pub spender: Address,
    pub amount: U256,
}
impl Encodable for ApproveCommand {
    const SIG: u32 = SIG_APPROVE;
    const LEN: usize = ADDRESS_LEN_BYTES + ADDRESS_LEN_BYTES + U256_LEN_BYTES;

    fn encode(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(Self::LEN);
        dst.extend_from_slice(self.owner.as_slice());
        dst.extend_from_slice(self.spender.as_slice());
        dst.extend_from_slice(&fixed_bytes_from_u256(&self.amount));
    }

    fn try_decode(input: &[u8]) -> Result<Self, u32> {
        const OWNER_OFFSET: usize = 0;
        const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
        const AMOUNT_OFFSET: usize = SPENDER_OFFSET + ADDRESS_LEN_BYTES;
        Self::validate_input_for_decode(input)?;
        let owner = Address::from_slice(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES]);
        let spender =
            Address::from_slice(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES]);
        let amount =
            u256_from_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + size_of::<U256>()]).unwrap();

        Ok(Self {
            owner,
            spender,
            amount,
        })
    }
}

pub struct AllowanceCommand {
    pub owner: Address,
    pub spender: Address,
}
impl Encodable for AllowanceCommand {
    const SIG: u32 = SIG_ALLOWANCE;
    const LEN: usize = ADDRESS_LEN_BYTES + ADDRESS_LEN_BYTES;

    fn encode(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(Self::LEN);
        dst.extend_from_slice(self.owner.as_slice());
        dst.extend_from_slice(self.spender.as_slice());
    }

    fn try_decode(input: &[u8]) -> Result<Self, u32> {
        const OWNER_OFFSET: usize = 0;
        const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
        Self::validate_input_for_decode(input)?;
        let owner = Address::from_slice(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES]);
        let spender =
            Address::from_slice(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES]);

        Ok(Self { owner, spender })
    }
}

pub struct BalanceOfCommand {
    pub owner: Address,
}
impl Encodable for BalanceOfCommand {
    const SIG: u32 = SIG_BALANCE_OF;
    const LEN: usize = ADDRESS_LEN_BYTES;

    fn encode(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(Self::LEN);
        dst.extend_from_slice(self.owner.as_slice());
    }

    fn try_decode(input: &[u8]) -> Result<Self, u32> {
        const OWNER_OFFSET: usize = 0;
        Self::validate_input_for_decode(input)?;
        let owner = Address::from_slice(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES]);

        Ok(Self { owner })
    }
}

pub struct MintCommand {
    pub to: Address,
    pub amount: U256,
}
impl Encodable for MintCommand {
    const SIG: u32 = SIG_MINT;
    const LEN: usize = ADDRESS_LEN_BYTES + U256_LEN_BYTES;

    fn encode(&self, dst: &mut Vec<u8>) {
        dst.reserve_exact(Self::LEN);
        dst.extend_from_slice(self.to.as_slice());
        dst.extend_from_slice(&fixed_bytes_from_u256(&self.amount));
    }

    fn try_decode(input: &[u8]) -> Result<Self, u32> {
        Self::validate_input_for_decode(input)?;
        let to = Address::from_slice(&input[..ADDRESS_LEN_BYTES]);
        let zero_address = Address::ZERO;
        if to == zero_address {
            return Err(ERR_INVALID_RECIPIENT);
        }
        let amount =
            u256_from_slice_try(&input[ADDRESS_LEN_BYTES..ADDRESS_LEN_BYTES + U256_LEN_BYTES])
                .unwrap();

        Ok(Self { to, amount })
    }
}
