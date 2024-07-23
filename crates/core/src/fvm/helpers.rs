use alloc::vec;
use core::ops::Deref;
use fluentbase_sdk::{Address, LowLevelSDK, U256};
use fluentbase_types::{Bytes32, SharedAPI};
use fuel_core_storage::{
    codec::{
        primitive::{utxo_id_to_bytes, Primitive},
        Decode,
    },
    ContractsAssetKey,
};
use fuel_core_types::{
    fuel_tx::{TxId, UtxoId},
    fuel_types,
    fuel_vm::ContractsStateKey,
};

pub(crate) struct IndexedHash(Bytes32);

impl Deref for IndexedHash {
    type Target = Bytes32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IndexedHash {
    pub(crate) fn new(hash: &Bytes32) -> IndexedHash {
        IndexedHash(hash.clone())
    }

    pub(crate) fn update_with_index(mut self, index: u32) -> IndexedHash {
        let mut preimage = vec![0u8; 32 + 4];
        preimage[..4].copy_from_slice(&index.to_le_bytes());
        preimage[4..].copy_from_slice(self.0.as_slice());
        LowLevelSDK::keccak256(
            preimage.as_ptr(),
            preimage.len() as u32,
            self.0.as_mut_ptr(),
        );
        self
    }

    pub(crate) fn compute_by_index(&self, index: u32) -> IndexedHash {
        let mut res = IndexedHash::new(&self.0);
        res.update_with_index(index)
    }
}

pub(crate) struct UtxoIdWrapper(UtxoId);

impl UtxoIdWrapper {
    pub const ENCODED_LEN: usize = TxId::LEN + core::mem::size_of::<u16>();

    pub(crate) fn new(utxo_id: UtxoId) -> Self {
        Self(utxo_id)
    }

    pub(crate) fn get(&self) -> UtxoId {
        self.0
    }

    pub(crate) fn hash(&self) -> Bytes32 {
        let mut utxo_encoded = utxo_id_to_bytes(&self.0);
        let mut hash = Bytes32::default();
        LowLevelSDK::keccak256(
            utxo_encoded.as_ptr(),
            utxo_encoded.len() as u32,
            hash.as_mut_ptr(),
        );
        hash
    }

    pub(crate) fn encode(&self) -> [u8; Self::ENCODED_LEN] {
        utxo_id_to_bytes(&self.0)
    }

    pub(crate) fn decode(v: &[u8]) -> Self {
        let decoded = Primitive::decode(v).expect("failed to decode utxo id");
        UtxoIdWrapper(decoded)
    }
}

pub(crate) struct OwnerBalanceWrapper {
    owner: Address,
    balance: u64,
}

impl OwnerBalanceWrapper {
    pub const ENCODED_LEN: usize = core::mem::size_of::<Address>() + core::mem::size_of::<u64>();

    pub(crate) fn new(owner: Address, balance: u64) -> Self {
        Self { owner, balance }
    }

    pub(crate) fn owner(&self) -> &Address {
        return &self.owner;
    }

    pub(crate) fn balance(&self) -> u64 {
        return self.balance;
    }

    pub(crate) fn encode_as_u256(&self) -> U256 {
        let mut res = Bytes32::default();
        res.copy_from_slice(self.owner.as_slice());
        res[core::mem::size_of::<Address>()..].copy_from_slice(&self.balance.to_be_bytes());
        U256::from_be_slice(&res)
    }

    pub(crate) fn decode_from_u256(v: &U256) -> Self {
        let v = v.to_be_bytes::<32>();
        let mut address = Address::from_slice(&v[..core::mem::size_of::<Address>()]);
        let mut balance_arr = [0u8; core::mem::size_of::<u64>()];
        balance_arr.copy_from_slice(&v[core::mem::size_of::<Address>()..]);
        OwnerBalanceWrapper {
            owner: address,
            balance: u64::from_be_bytes(balance_arr),
        }
    }
}

pub(crate) struct ContractsStateKeyWrapper {
    csk: ContractsStateKey,
}

impl ContractsStateKeyWrapper {
    pub(crate) fn new(csk: ContractsStateKey) -> Self {
        return Self { csk };
    }

    pub(crate) fn new_from_slice(v: &[u8]) -> Self {
        return Self {
            csk: ContractsStateKey::from_slice(v)
                .expect("failed to create contract state key from slice"),
        };
    }

    pub(crate) fn get(&self) -> ContractsStateKey {
        self.csk
    }
}

impl AsRef<[u8]> for ContractsStateKeyWrapper {
    fn as_ref(&self) -> &[u8] {
        self.csk.as_ref()
    }
}

impl Hashable for ContractsStateKeyWrapper {}

pub trait Hashable: AsRef<[u8]> {
    fn hash(&self) -> Bytes32 {
        let mut hash = Bytes32::default();
        LowLevelSDK::keccak256(
            self.as_ref().as_ptr(),
            self.as_ref().len() as u32,
            hash.as_mut_ptr(),
        );
        hash
    }
}

pub(crate) struct ContractsAssetKeyWrapper {
    cak: ContractsAssetKey,
}

impl AsRef<[u8]> for ContractsAssetKeyWrapper {
    fn as_ref(&self) -> &[u8] {
        self.cak.as_ref()
    }
}

impl Hashable for ContractsAssetKeyWrapper {}

impl ContractsAssetKeyWrapper {
    pub(crate) fn new(cak: ContractsAssetKey) -> Self {
        return Self { cak };
    }

    pub(crate) fn new_from_slice(v: &[u8]) -> Self {
        return Self {
            cak: ContractsAssetKey::from_slice(v)
                .expect("failed to create contract asset key from slice"),
        };
    }

    pub(crate) fn from_u256(v: &U256) -> Self {
        return Self {
            cak: ContractsAssetKey::from_slice(&v.to_be_bytes::<64>())
                .expect("failed to create contract asset key from slice"),
        };
    }

    pub(crate) fn get(&self) -> ContractsAssetKey {
        self.cak
    }
}

pub(crate) struct FuelAddressWrapper {
    address: fuel_types::Address,
}

impl FuelAddressWrapper {
    pub(crate) fn new(address: fuel_types::Address) -> Self {
        Self { address }
    }

    pub(crate) fn get(&self) -> fuel_types::Address {
        self.address
    }
}

impl From<&Address> for FuelAddressWrapper {
    fn from(value: &Address) -> Self {
        let mut address = fuel_types::Address::default();
        address[12..].copy_from_slice(&value.0 .0);
        Self { address }
    }
}

impl AsRef<fuel_types::Address> for FuelAddressWrapper {
    fn as_ref(&self) -> &fuel_core_types::fuel_tx::Address {
        return &self.address;
    }
}
