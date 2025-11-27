use alloc::vec;
use fluentbase_sdk::crypto::crypto_keccak256;
use fluentbase_sdk::{B256, U256};

pub type SlotType = u32;

const SLOT_BYTES: usize = size_of::<SlotType>();

pub trait IKeyDeriver: Sized {
    const SLOT: SlotType;

    #[inline]
    fn b256(&self, v: &B256) -> U256 {
        let mut data = [0u8; SLOT_BYTES + size_of::<B256>()];
        data[..SLOT_BYTES].copy_from_slice(Self::SLOT.to_be_bytes().as_ref());
        data[SLOT_BYTES..].copy_from_slice(&v.0);
        crypto_keccak256(&data).into()
    }

    #[inline]
    fn u256(&self, v: &U256) -> U256 {
        let mut data = [0u8; SLOT_BYTES + size_of::<B256>()];
        data[..SLOT_BYTES].copy_from_slice(Self::SLOT.to_be_bytes().as_ref());
        data[SLOT_BYTES..].copy_from_slice(&v.as_le_bytes());
        crypto_keccak256(&data).into()
    }

    #[inline]
    fn slice(&self, v: &[u8]) -> U256 {
        let mut data = vec![0u8; SLOT_BYTES + v.len()];
        data[..SLOT_BYTES].copy_from_slice(Self::SLOT.to_be_bytes().as_ref());
        data[SLOT_BYTES..].copy_from_slice(v);
        crypto_keccak256(&data).into()
    }
}

#[macro_export]
macro_rules! impl_key_deriver {
    ($num:expr) => {
        paste::paste! {
            struct [<KeyDeriver $num>] {}
            impl IKeyDeriver for [<KeyDeriver $num>] {
                const SLOT: SlotType = $num;
            }
        }
    };
}
