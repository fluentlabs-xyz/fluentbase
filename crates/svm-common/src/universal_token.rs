use crate::common::{
    lamports_ref_to_bytes_ref, lamports_ref_try_from_slice, lamports_try_from_slice,
    pubkey_ref_try_from_slice,
};
use crate::pubkey::PUBKEY_ZERO;
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

pub struct ApproveParams<'a> {
    pub from: &'a Pubkey,
    pub delegate: &'a Pubkey,
    pub owner: &'a Pubkey,
    pub amount: &'a u64,
}

impl<'a> ApproveParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const FROM_OFFSET: usize = 0;
        const DELEGATE_OFFSET: usize = FROM_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = DELEGATE_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = OWNER_OFFSET + PUBKEY_BYTES;

        let Ok(amount) = lamports_ref_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(from) = pubkey_ref_try_from_slice(&input[FROM_OFFSET..]) else {
            return Err(());
        };
        let Ok(delegate) = pubkey_ref_try_from_slice(&input[DELEGATE_OFFSET..]) else {
            return Err(());
        };
        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            from,
            delegate,
            owner,
            amount,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.from.as_ref());
        out.extend_from_slice(self.delegate.as_ref());
        out.extend_from_slice(self.owner.as_ref());
        out.extend_from_slice(lamports_ref_to_bytes_ref(self.amount));
    }
}

pub struct ApproveCheckedParams<'a> {
    pub source: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub delegate: &'a Pubkey,
    pub owner: &'a Pubkey,
    pub amount: &'a u64,
    pub decimals: u8,
}

impl<'a> ApproveCheckedParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const SOURCE_OFFSET: usize = 0;
        const MINT_OFFSET: usize = SOURCE_OFFSET + PUBKEY_BYTES;
        const DELEGATE_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = DELEGATE_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = OWNER_OFFSET + PUBKEY_BYTES;
        const DECIMALS_OFFSET: usize = AMOUNT_OFFSET + size_of::<u64>();

        let Some(decimals) = input.get(DECIMALS_OFFSET).cloned() else {
            return Err(());
        };
        let Ok(source) = pubkey_ref_try_from_slice(&input[SOURCE_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(delegate) = pubkey_ref_try_from_slice(&input[DELEGATE_OFFSET..]) else {
            return Err(());
        };
        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };
        let Ok(amount) = lamports_ref_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            source,
            mint,
            delegate,
            owner,
            amount,
            decimals,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.source.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.delegate.as_ref());
        out.extend_from_slice(self.owner.as_ref());
        out.extend_from_slice(lamports_ref_to_bytes_ref(self.amount));
        out.push(self.decimals);
    }
}

pub struct RevokeParams<'a> {
    pub source: &'a Pubkey,
    pub owner: &'a Pubkey,
}

impl<'a> RevokeParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const SOURCE_OFFSET: usize = 0;
        const OWNER_OFFSET: usize = SOURCE_OFFSET + PUBKEY_BYTES;

        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };
        let Ok(source) = pubkey_ref_try_from_slice(&input[SOURCE_OFFSET..]) else {
            return Err(());
        };

        Ok(Self { source, owner })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.source.as_ref());
        out.extend_from_slice(self.owner.as_ref());
    }
}

pub struct SetAuthorityParams<'a> {
    pub owned: &'a Pubkey,
    pub new_authority: Option<&'a Pubkey>,
    pub authority_type: u8,
    pub owner: &'a Pubkey,
}

impl<'a> SetAuthorityParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const OWNED_OFFSET: usize = 0;
        const NEW_AUTHORITY_OFFSET: usize = OWNED_OFFSET + PUBKEY_BYTES;
        const AUTHORITY_TYPE_OFFSET: usize = NEW_AUTHORITY_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = AUTHORITY_TYPE_OFFSET + 1;

        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };
        let Ok(owned) = pubkey_ref_try_from_slice(&input[OWNED_OFFSET..]) else {
            return Err(());
        };
        let Ok(new_authority) = pubkey_ref_try_from_slice(&input[NEW_AUTHORITY_OFFSET..]) else {
            return Err(());
        };
        let Some(authority_type) = input.get(AUTHORITY_TYPE_OFFSET).cloned() else {
            return Err(());
        };

        let new_authority_opt = if new_authority == &PUBKEY_ZERO {
            None
        } else {
            Some(new_authority)
        };

        Ok(Self {
            owned,
            new_authority: new_authority_opt,
            authority_type,
            owner,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.owned.as_ref());
        if let Some(new_authority) = self.new_authority {
            out.extend_from_slice(new_authority.as_ref());
        } else {
            out.extend_from_slice(&[0u8; PUBKEY_BYTES]);
        };
        out.push(self.authority_type);
        out.extend_from_slice(self.owner.as_ref());
    }
}

pub struct BurnParams<'a> {
    pub account: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub authority: &'a Pubkey,
    pub amount: &'a u64,
}
impl<'a> BurnParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const ACCOUNT_OFFSET: usize = 0;
        const MINT_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const AUTHORITY_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;

        let Ok(amount) = lamports_ref_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(authority) = pubkey_ref_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            account,
            mint,
            authority,
            amount,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.authority.as_ref());
        out.extend_from_slice(lamports_ref_to_bytes_ref(self.amount));
    }
}

pub struct BurnCheckedParams<'a> {
    pub account: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub authority: &'a Pubkey,
    pub amount: &'a u64,
    pub decimals: u8,
}
impl<'a> BurnCheckedParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const ACCOUNT_OFFSET: usize = 0;
        const MINT_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const AUTHORITY_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
        const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;
        const DECIMALS_OFFSET: usize = AMOUNT_OFFSET + size_of::<u64>();

        let Some(decimals) = input.get(DECIMALS_OFFSET).cloned() else {
            return Err(());
        };
        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(authority) = pubkey_ref_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
            return Err(());
        };
        let Ok(amount) = lamports_ref_try_from_slice(&input[AMOUNT_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            account,
            mint,
            authority,
            amount,
            decimals,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.authority.as_ref());
        out.extend_from_slice(lamports_ref_to_bytes_ref(self.amount));
        out.push(self.decimals);
    }
}
pub struct CloseAccountParams<'a> {
    pub account: &'a Pubkey,
    pub destination: &'a Pubkey,
    pub owner: &'a Pubkey,
}
impl<'a> CloseAccountParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const ACCOUNT_OFFSET: usize = 0;
        const DESTINATION_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = DESTINATION_OFFSET + PUBKEY_BYTES;

        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(destination) = pubkey_ref_try_from_slice(&input[DESTINATION_OFFSET..]) else {
            return Err(());
        };
        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            account,
            destination,
            owner,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.destination.as_ref());
        out.extend_from_slice(self.owner.as_ref());
    }
}
pub struct FreezeAccountParams<'a> {
    pub account: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub owner: &'a Pubkey,
}
impl<'a> FreezeAccountParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const ACCOUNT_OFFSET: usize = 0;
        const MINT_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;

        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            account,
            mint,
            owner,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.owner.as_ref());
    }
}

pub struct ThawAccountParams<'a> {
    pub account: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub owner: &'a Pubkey,
}
impl<'a> ThawAccountParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const ACCOUNT_OFFSET: usize = 0;
        const MINT_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
        const OWNER_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;

        let Ok(account) = pubkey_ref_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
            return Err(());
        };
        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Ok(owner) = pubkey_ref_try_from_slice(&input[OWNER_OFFSET..]) else {
            return Err(());
        };

        Ok(Self {
            account,
            mint,
            owner,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.account.as_ref());
        out.extend_from_slice(self.mint.as_ref());
        out.extend_from_slice(self.owner.as_ref());
    }
}

pub struct GetAccountDataSizeParams<'a> {
    // TODO mint_pubkey: &Pubkey, extension_types: &[ExtensionType]
    pub mint: &'a Pubkey,
    pub extension_types: Vec<u16>,
}
impl<'a> GetAccountDataSizeParams<'a> {
    pub fn try_parse(input: &'a [u8]) -> Result<Self, ()> {
        const MINT_OFFSET: usize = 0;
        const EXTENSION_TYPES_COUNT_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES; // 1 byte
        const EXTENSION_TYPES_OFFSET: usize = EXTENSION_TYPES_COUNT_OFFSET + 1;

        let Ok(mint) = pubkey_ref_try_from_slice(&input[MINT_OFFSET..]) else {
            return Err(());
        };
        let Some(extension_types_count) = input.get(EXTENSION_TYPES_COUNT_OFFSET).cloned() else {
            return Err(());
        };
        let mut extension_types = Vec::<u16>::with_capacity(extension_types_count as usize);
        for i in 0..extension_types_count {
            let base_offset = EXTENSION_TYPES_OFFSET * i as usize;
            let extension_type_bytes: Result<[u8; size_of::<u16>()], _> =
                input[base_offset..base_offset + size_of::<u16>()].try_into();
            if let Ok(extension_type_bytes) = extension_type_bytes {
                extension_types.push(u16::from_be_bytes(extension_type_bytes));
            } else {
                return Err(());
            }
        }

        Ok(Self {
            mint,
            extension_types,
        })
    }

    pub fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.mint.as_ref());
        out.push(self.extension_types.len() as u8);
        for extension_type in &self.extension_types {
            out.extend_from_slice(extension_type.to_be_bytes().as_ref());
        }
    }
}
