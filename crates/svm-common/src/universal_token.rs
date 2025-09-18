use crate::common::{
    lamports_ref_to_bytes_ref, lamports_ref_try_from_slice, lamports_to_bytes,
    lamports_try_from_slice, pubkey_ref_try_from_slice, pubkey_try_from_slice,
};
use alloc::vec::Vec;
use solana_pubkey::{Pubkey, PUBKEY_BYTES};

pub struct TransferParams<'a> {
    pub mint: &'a Pubkey,
    pub to: &'a Pubkey,
    pub authority: &'a Pubkey,
    pub amount: u64,
    pub decimals: u8,
}
impl<'a> TransferParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const MINT_OFFSET: usize = 0;
        const TO_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const AUTHORITY_OFFSET: usize = TO_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;
        const DECIMALS_OFFSET: usize = AMOUNT_OFFSET + size_of::<u64>();

        let Some(decimals) = input.get(DECIMALS_OFFSET).cloned() else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(to) = pubkey_ref_try_from_slice(&input[TO_OFFSET..]) else {
            return Err(());
        };
        let Ok(authority) = pubkey_ref_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
            return Err(());
        };
        let Ok(amount) = lamports_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };
        Ok(Self {
            mint,
            to,
            authority,
            amount,
            decimals,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.to.as_ref());
        out.extend_from_slice(self.authority.as_ref());
        out.extend_from_slice(&self.amount.to_be_bytes());
        out.push(self.decimals);
    }
}

pub struct TransferFromParams<'a> {
    pub from: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub to: &'a Pubkey,
    pub authority: &'a Pubkey,
    pub amount: &'a u64,
    pub decimals: u8,
}
impl<'a> TransferFromParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const FROM_OFFSET: usize = 0;
        const MINT_OFFSET: usize = FROM_OFFSET + PUBKEY_BYTES;
        const TO_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const AUTHORITY_OFFSET: usize = TO_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;
        const DECIMALS_OFFSET: usize = AMOUNT_OFFSET + size_of::<u64>();

        let Some(decimals) = input.get(DECIMALS_OFFSET).cloned() else {
            return Err(());
        };
        let Ok(from) = pubkey_ref_try_from_slice(&input[FROM_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(to) = pubkey_ref_try_from_slice(&input[TO_OFFSET..]) else {
            return Err(());
        };
        let Ok(authority) = pubkey_ref_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
            return Err(());
        };
        let Ok(amount) = lamports_ref_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };
        Ok(Self {
            from,
            mint,
            to,
            authority,
            amount,
            decimals,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.from.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.to.as_ref());
        out.extend_from_slice(self.authority.as_ref());
        out.extend_from_slice(lamports_ref_to_bytes_ref(self.amount));
        out.push(self.decimals);
    }
}

pub struct InitializeMintParams<'a> {
    pub mint: &'a Pubkey,
    pub mint_authority: &'a Pubkey,
    pub freeze_opt: Option<&'a Pubkey>,
    pub decimals: u8,
}
impl<'a> InitializeMintParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const MINT_OFFSET: usize = 0;
        const MINT_AUTHORITY_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const FREEZE_OFFSET: usize = MINT_AUTHORITY_OFFSET + PUBKEY_BYTES;
        const DECIMALS_OFFSET: usize = FREEZE_OFFSET + PUBKEY_BYTES;

        let Some(decimals) = input.get(DECIMALS_OFFSET).cloned() else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint_authority) = pubkey_ref_try_from_slice(&input[MINT_AUTHORITY_OFFSET..]) else {
            return Err(());
        };
        let Ok(freeze) = pubkey_ref_try_from_slice(&input[FREEZE_OFFSET..]) else {
            return Err(());
        };

        let freeze_opt = if freeze.as_ref() == &[0u8; PUBKEY_BYTES] {
            None
        } else {
            Some(freeze)
        };
        Ok(Self {
            mint,
            mint_authority,
            freeze_opt,
            decimals,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.mint_authority.as_ref());
        if let Some(freeze) = self.freeze_opt {
            out.extend_from_slice(freeze.as_ref());
        } else {
            out.extend_from_slice(&[0u8; PUBKEY_BYTES]);
        };
        out.push(self.decimals);
    }
}

pub struct InitializeAccountParams<'a> {
    pub account: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub owner: &'a Pubkey,
}

impl<'a> InitializeAccountParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const ACCOUNT_OFFSET: usize = 0;
        const MINT_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;

        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };
        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            owner,
            account,
            mint,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.owner.as_ref());
    }
}

pub struct MintToParams<'a> {
    pub mint: &'a Pubkey,
    pub account: &'a Pubkey,
    pub owner: &'a Pubkey,
    pub amount: &'a u64,
}

impl<'a> MintToParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const MINT_OFFSET: usize = 0;
        const ACCOUNT_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = OWNER_OFFSET + PUBKEY_BYTES;

        let Ok(amount) = lamports_ref_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            mint,
            account,
            owner,
            amount,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.owner.as_ref());
        out.extend_from_slice(lamports_ref_to_bytes_ref(self.amount));
    }
}
