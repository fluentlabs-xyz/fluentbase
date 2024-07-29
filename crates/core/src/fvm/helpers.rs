use alloc::vec;
use core::{mem::size_of, ops::Deref};
use fluentbase_sdk::{Address, LowLevelSDK, U256};
use fluentbase_types::{Bytes32, Bytes34, Bytes64, SharedAPI};
use fuel_core_storage::{column::Column, ContractsAssetKey};
use fuel_core_types::{
    fuel_tx::ContractId,
    fuel_types,
    fuel_types::canonical::Deserialize,
    fuel_vm::ContractsStateKey,
};

fn keccak256(data: &[u8], target: &mut Bytes32) {
    LowLevelSDK::keccak256(data.as_ptr(), data.len() as u32, target.as_mut_ptr());
}

pub trait PreimageKey {
    const COLUMN: Column;

    fn preimage_key(hash: &Bytes32) -> IndexedHash {
        IndexedHash::from_hash(hash).update_with_column(Self::COLUMN.as_u32())
    }

    fn preimage_key_raw(raw_key: &[u8]) -> IndexedHash {
        let mut hash = Bytes32::default();
        keccak256(raw_key, &mut hash);
        Self::preimage_key(&hash)
    }
}

pub trait StorageSlot {
    const COLUMN: Column;

    fn storage_slot(hash: &Bytes32, slot: u32) -> IndexedHash {
        IndexedHash::from_hash(hash).update_with_column_index(Self::COLUMN.as_u32(), slot)
    }

    fn storage_slot_raw(raw_key: &[u8], slot: u32) -> IndexedHash {
        let mut hash = Bytes32::default();
        keccak256(raw_key, &mut hash);
        Self::storage_slot(&hash, slot)
    }
}

pub(crate) struct IndexedHash(Bytes32);

impl Deref for IndexedHash {
    type Target = Bytes32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IndexedHash {
    pub(crate) fn from_hash(hash: &Bytes32) -> IndexedHash {
        IndexedHash(hash.clone())
    }
    pub(crate) fn from_data_slice(data: &[u8]) -> IndexedHash {
        let mut hash = Bytes32::default();
        keccak256(data, &mut hash);
        IndexedHash(hash)
    }

    pub(crate) fn update_with_column(mut self, column: u32) -> IndexedHash {
        let mut preimage = vec![0u8; 32 + 4];
        preimage[..4].copy_from_slice(&column.to_le_bytes());
        preimage[4..].copy_from_slice(self.0.as_slice());
        keccak256(&preimage, &mut self.0);
        self
    }

    pub(crate) fn compute_by_column(&self, column: u32) -> IndexedHash {
        let mut res = IndexedHash::from_hash(&self.0);
        res.update_with_column(column)
    }

    pub(crate) fn update_with_column_index(mut self, column: u32, index: u32) -> IndexedHash {
        let mut preimage = vec![0u8; 32 + 4 + 4];
        preimage[..4].copy_from_slice(&column.to_le_bytes());
        preimage[4..8].copy_from_slice(&index.to_le_bytes());
        preimage[8..].copy_from_slice(self.0.as_slice());
        keccak256(&preimage, &mut self.0);
        self
    }

    pub(crate) fn compute_by_column_index(&self, column: u32, index: u32) -> IndexedHash {
        let mut res = IndexedHash::from_hash(&self.0);
        res.update_with_column_index(column, index)
    }

    pub(crate) fn inner(&self) -> &Bytes32 {
        &self.0
    }
}
pub struct MetadataHelper<'a> {
    original_key: &'a [u8],
}

impl<'a> PreimageKey for MetadataHelper<'a> {
    const COLUMN: Column = Column::Metadata;
}

impl<'a> MetadataHelper<'a> {
    pub fn new(key: &'a [u8]) -> Self {
        Self { original_key: key }
    }

    pub fn value_preimage_key(&self) -> IndexedHash {
        Self::preimage_key_raw(self.original_key)
    }
}

pub struct ContractsRawCodeHelper {
    original_key: ContractId,
}

impl PreimageKey for ContractsRawCodeHelper {
    const COLUMN: Column = Column::ContractsRawCode;
}

impl ContractsRawCodeHelper {
    pub fn new(contract_id: &ContractId) -> Self {
        Self {
            original_key: *contract_id,
        }
    }

    pub fn value_preimage_key(&self) -> IndexedHash {
        Self::preimage_key(&self.original_key)
    }
}

pub struct ContractsLatestUtxoHelper {
    original_key: ContractId,
}

impl PreimageKey for ContractsLatestUtxoHelper {
    const COLUMN: Column = Column::ContractsLatestUtxo;
}

impl ContractsLatestUtxoHelper {
    pub fn new(contract_id: &ContractId) -> Self {
        Self {
            original_key: *contract_id,
        }
    }
    pub fn value_preimage_key(&self) -> IndexedHash {
        Self::preimage_key(&self.original_key)
    }
}

pub(crate) struct ContractsStateHelper {
    original_key: ContractsStateKey,
}

impl PreimageKey for ContractsStateHelper {
    const COLUMN: Column = Column::ContractsState;
}

impl ContractsStateHelper {
    pub(crate) fn new(key: &Bytes64) -> Self {
        return Self {
            original_key: ContractsStateKey::from_array(*key),
        };
    }

    pub(crate) fn from_slice(v: &[u8]) -> Self {
        return Self {
            original_key: ContractsStateKey::from_slice(v).expect("valid contract state key"),
        };
    }

    pub(crate) fn get(&self) -> ContractsStateKey {
        self.original_key
    }

    pub(crate) fn value_preimage_key(&self) -> IndexedHash {
        Self::preimage_key_raw(self.original_key.as_ref())
    }
}

pub(crate) struct ContractsAssetsHelper {
    original_key: ContractsAssetKey,
}

impl StorageSlot for ContractsAssetsHelper {
    const COLUMN: Column = Column::ContractsAssets;
}

impl ContractsAssetsHelper {
    const VALUE_STORAGE_SLOT: u32 = 0;
    pub(crate) fn new(key: &Bytes64) -> Self {
        return Self {
            original_key: ContractsAssetKey::from_array(*key),
        };
    }

    pub(crate) fn from_slice(v: &[u8]) -> Self {
        return Self {
            original_key: ContractsAssetKey::from_slice(v).expect("valid contracts assets key"),
        };
    }

    pub(crate) fn get(&self) -> ContractsAssetKey {
        self.original_key
    }

    pub(crate) fn value_storage_slot(&self) -> U256 {
        U256::from_be_bytes(
            Self::storage_slot_raw(self.original_key.as_ref(), Self::VALUE_STORAGE_SLOT)
                .inner()
                .clone(),
        )
    }

    pub(crate) fn value_to_u256(v: &[u8; 4]) -> U256 {
        U256::from_be_slice(v)
    }

    pub(crate) fn u256_to_value(v: &U256) -> [u8; 4] {
        let mut res = [0u8; 4];
        res.copy_from_slice(&v.to_be_bytes::<32>()[24..]);
        res
    }
}

pub(crate) struct CoinsHelper {
    original_key: Bytes34, // UtxoId as a key
}

impl StorageSlot for CoinsHelper {
    const COLUMN: Column = Column::Coins;
}

impl CoinsHelper {
    const OWNER_WITH_BALANCE_SLOT: u32 = 0;
    pub(crate) fn new(key: &Bytes34) -> Self {
        return Self { original_key: *key };
    }

    pub(crate) fn from_slice(v: &[u8]) -> Self {
        let mut original_key = Bytes34::from_bytes(v).expect("valid utxo id key");
        return Self { original_key };
    }

    pub(crate) fn get(&self) -> Bytes34 {
        self.original_key
    }

    pub(crate) fn owner_with_balance_storage_slot(&self) -> U256 {
        U256::from_be_bytes(
            Self::storage_slot_raw(self.original_key.as_ref(), Self::OWNER_WITH_BALANCE_SLOT)
                .inner()
                .clone(),
        )
    }
}

pub(crate) struct FuelAddress {
    address: fuel_types::Address,
}

impl FuelAddress {
    pub(crate) fn new(address: fuel_types::Address) -> Self {
        Self { address }
    }

    pub(crate) fn address(&self) -> fuel_types::Address {
        self.address
    }
}

impl From<Address> for FuelAddress {
    fn from(value: Address) -> Self {
        let mut address = fuel_types::Address::default();
        address[12..].copy_from_slice(&value.0 .0);
        Self { address }
    }
}

impl AsRef<fuel_types::Address> for FuelAddress {
    fn as_ref(&self) -> &fuel_core_types::fuel_tx::Address {
        return &self.address;
    }
}

pub(crate) struct CoinsOwnerWithBalanceHelper {
    address: Address,
    balance: u64,
}

impl CoinsOwnerWithBalanceHelper {
    pub const ENCODED_LEN: usize = size_of::<Address>() + size_of::<u64>();

    pub(crate) fn new(address: Address, balance: u64) -> Self {
        Self { address, balance }
    }

    pub(crate) fn from_owner(owner: &fuel_types::Address, balance: u64) -> Self {
        Self {
            address: Address::from_slice(&owner[12..]),
            balance,
        }
    }

    pub(crate) fn address(&self) -> &Address {
        return &self.address;
    }

    pub(crate) fn balance(&self) -> u64 {
        return self.balance;
    }

    pub(crate) fn to_u256(&self) -> U256 {
        let mut res = Bytes32::default();
        res.copy_from_slice(self.address.as_slice());
        res[size_of::<Address>()..].copy_from_slice(&self.balance.to_be_bytes());
        U256::from_be_slice(&res)
    }

    pub(crate) fn from_u256(v: &U256) -> Self {
        let v = v.to_be_bytes::<32>();
        let mut address = Address::from_slice(&v[..size_of::<Address>()]);
        let mut balance_arr = [0u8; size_of::<u64>()];
        balance_arr
            .copy_from_slice(&v[size_of::<Address>()..size_of::<Address>() + size_of::<u64>()]);
        CoinsOwnerWithBalanceHelper {
            address,
            balance: u64::from_be_bytes(balance_arr),
        }
    }
}
