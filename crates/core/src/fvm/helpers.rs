use alloc::vec;
use core::{mem::size_of, ops::Deref, str::FromStr};
use fluentbase_sdk::{Address, Bytes32, Bytes34, Bytes64, SovereignAPI, B256, U256};
use fuel_core_storage::{column::Column, ContractsAssetKey};
use fuel_core_types::{
    fuel_tx::{
        consensus_parameters::{
            ConsensusParametersV1,
            ContractParametersV1,
            FeeParametersV1,
            PredicateParametersV1,
            ScriptParametersV1,
            TxParametersV1,
        },
        AssetId,
        ConsensusParameters,
        ContractId,
        ContractParameters,
        FeeParameters,
        GasCosts,
        PredicateParameters,
        ScriptParameters,
        TxParameters,
    },
    fuel_types,
    fuel_types::{canonical::Deserialize, ChainId},
    fuel_vm::ContractsStateKey,
};

pub const TESTNET_BASE_ASSET_ID: &str =
    "f8f8b6283d7fa5b672b530cbb84fcccb4ff8dc40f8176ef4544ddb1f1952ad07";
pub const TESTNET_PRIVILEGED_ADDRESS: &str =
    "9f0e19d6c2a6283a3222426ab2630d35516b1799b503f37b02105bebe1b8a3e9";

pub fn fuel_testnet_consensus_params_from(
    max_gas_per_tx: Option<u64>,
    max_gas_per_predicate: Option<u64>,
    block_gas_limit: Option<u64>,
    chain_id: ChainId,
    gas_costs: Option<GasCosts>,
) -> ConsensusParameters {
    ConsensusParameters::V1(ConsensusParametersV1 {
        tx_params: TxParameters::V1(TxParametersV1 {
            max_inputs: 8,
            max_outputs: 8,
            max_witnesses: 8,
            max_gas_per_tx: max_gas_per_tx.unwrap_or(30000000),
            max_size: 110 * 1024,
            max_bytecode_subsections: 255,
        }),
        predicate_params: PredicateParameters::V1(PredicateParametersV1 {
            max_predicate_length: 1024 * 1024,
            max_predicate_data_length: 1024 * 1024,
            max_message_data_length: 1024 * 1024,
            max_gas_per_predicate: max_gas_per_predicate.unwrap_or(30000000),
        }),
        script_params: ScriptParameters::V1(ScriptParametersV1 {
            max_script_length: 1024 * 1024,
            max_script_data_length: 1024 * 1024,
        }),
        contract_params: ContractParameters::V1(ContractParametersV1 {
            contract_max_size: 100 * 1024,
            max_storage_slots: 1760,
        }),
        fee_params: FeeParameters::V1(FeeParametersV1 {
            gas_price_factor: 92,
            gas_per_byte: 62,
        }),
        chain_id,
        gas_costs: gas_costs.unwrap_or_default(),
        base_asset_id: AssetId::from_str(TESTNET_BASE_ASSET_ID).expect("valid asset id format"),
        block_gas_limit: block_gas_limit.unwrap_or(30000000),
        privileged_address: fuel_types::Address::from_str(TESTNET_PRIVILEGED_ADDRESS)
            .expect("valid privileged address format"),
    })
}

pub fn fuel_testnet_consensus_params_from_cr<SDK: SovereignAPI>(sdk: &SDK) -> ConsensusParameters {
    fuel_testnet_consensus_params_from(
        Some(sdk.tx_context().gas_limit),
        Some(sdk.tx_context().gas_limit),
        Some(sdk.block_context().gas_limit),
        ChainId::new(sdk.block_context().chain_id),
        None,
    )
}

fn keccak256(data: &[u8], target: &mut Bytes32) {
    use keccak_hash::keccak;
    // TODO: "replace with SDK version"
    *target = keccak(data).0;
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

impl Into<B256> for IndexedHash {
    fn into(self) -> B256 {
        B256::from(self.0)
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
        let res = IndexedHash::from_hash(&self.0);
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
        let res = IndexedHash::from_hash(&self.0);
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

pub(crate) enum ContractsStateKeyWrapper {
    Original(ContractsStateKey),
    Transformed(Bytes32),
}

pub(crate) struct ContractsStateHelper {
    key: ContractsStateKeyWrapper,
}

impl PreimageKey for ContractsStateHelper {
    const COLUMN: Column = Column::ContractsState;
}

impl StorageSlot for ContractsStateHelper {
    const COLUMN: Column = Column::ContractsState;
}

impl ContractsStateHelper {
    const MERKLE_DATA_STORAGE_SLOT: u32 = 0;
    const MERKLE_METADATA_STORAGE_SLOT: u32 = 1;

    pub(crate) fn new(key: &Bytes64) -> Self {
        return Self {
            key: ContractsStateKeyWrapper::Original(ContractsStateKey::from_array(*key)),
        };
    }

    pub(crate) fn new_transformed(key: &Bytes32) -> Self {
        return Self {
            key: ContractsStateKeyWrapper::Transformed(*key),
        };
    }

    // pub(crate) fn from_slice(v: &[u8]) -> Self {
    //     return Self {
    //         key: ContractsStateKey::from_slice(v)
    //             .expect("valid contract state key 64 bytes"),
    //     };
    // }

    pub(crate) fn get(&self) -> &ContractsStateKeyWrapper {
        &self.key
    }

    pub(crate) fn value_preimage_key(&self) -> IndexedHash {
        if let ContractsStateKeyWrapper::Original(key) = self.key {
            return Self::preimage_key_raw(key.as_ref());
        }
        panic!("original key expected")
    }

    pub(crate) fn merkle_data_preimage_key(&self) -> IndexedHash {
        if let ContractsStateKeyWrapper::Transformed(key) = self.key {
            return Self::storage_slot_raw(key.as_ref(), Self::MERKLE_DATA_STORAGE_SLOT);
        }
        panic!("transformed key expected")
    }

    pub(crate) fn merkle_metadata_preimage_key(&self) -> IndexedHash {
        if let ContractsStateKeyWrapper::Transformed(key) = self.key {
            return Self::storage_slot_raw(key.as_ref(), Self::MERKLE_METADATA_STORAGE_SLOT);
        }
        panic!("transformed key expected")
    }
}

pub(crate) enum ContractsAssetKeyWrapper {
    Original(ContractsAssetKey),
    Transformed(Bytes32),
}

pub(crate) struct ContractsAssetsHelper {
    key: ContractsAssetKeyWrapper,
}

impl StorageSlot for ContractsAssetsHelper {
    const COLUMN: Column = Column::ContractsAssets;
}

impl ContractsAssetsHelper {
    const VALUE_STORAGE_SLOT: u32 = 0;
    const MERKLE_DATA_STORAGE_SLOT: u32 = 1;
    const MERKLE_METADATA_STORAGE_SLOT: u32 = 2;
    pub(crate) fn new(original_key: &Bytes64) -> Self {
        return Self {
            key: ContractsAssetKeyWrapper::Original(ContractsAssetKey::from_array(*original_key)),
        };
    }
    pub(crate) fn from_transformed(key: &Bytes32) -> Self {
        return Self {
            key: ContractsAssetKeyWrapper::Transformed(*key),
        };
    }

    // pub(crate) fn from_slice(v: &[u8]) -> Self {
    //     return Self {
    //         original_key: ContractsAssetKey::from_slice(v).expect("contracts assets key 64
    // bytes"),     };
    // }

    pub(crate) fn get(&self) -> &ContractsAssetKeyWrapper {
        &self.key
    }

    pub(crate) fn value_storage_slot(&self) -> U256 {
        if let ContractsAssetKeyWrapper::Original(key) = self.key {
            return U256::from_be_bytes(
                Self::storage_slot_raw(key.as_ref(), Self::VALUE_STORAGE_SLOT)
                    .inner()
                    .clone(),
            );
        }
        panic!("original key expected")
    }

    pub(crate) fn value_to_u256(v: &[u8; 8]) -> U256 {
        U256::from_be_slice(v)
    }

    pub(crate) fn u256_to_value(v: &U256) -> [u8; 8] {
        let mut res = [0u8; 8];
        res.copy_from_slice(&v.to_be_bytes::<32>()[24..]);
        res
    }

    pub(crate) fn merkle_data_preimage_key(&self) -> IndexedHash {
        if let ContractsAssetKeyWrapper::Transformed(key) = self.key {
            return Self::storage_slot_raw(key.as_ref(), Self::MERKLE_DATA_STORAGE_SLOT);
        }
        panic!("transformed key expected")
    }

    pub(crate) fn merkle_metadata_preimage_key(&self) -> IndexedHash {
        if let ContractsAssetKeyWrapper::Transformed(key) = self.key {
            return Self::storage_slot_raw(key.as_ref(), Self::MERKLE_METADATA_STORAGE_SLOT);
        }
        panic!("transformed key expected")
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
    const OWNER_SLOT: u32 = 1;
    const BALANCE_SLOT: u32 = 2;
    pub(crate) fn new(key: &Bytes34) -> Self {
        return Self { original_key: *key };
    }

    pub(crate) fn from_slice(v: &[u8]) -> Self {
        let original_key = Bytes34::from_bytes(v).expect("valid utxo id key");
        Self { original_key }
    }

    pub(crate) fn get(&self) -> Bytes34 {
        self.original_key
    }

    // pub(crate) fn owner_with_balance_storage_slot(&self) -> U256 {
    //     U256::from_be_bytes(
    //         Self::storage_slot_raw(self.original_key.as_ref(), Self::OWNER_WITH_BALANCE_SLOT)
    //             .inner()
    //             .clone(),
    //     )
    // }

    pub(crate) fn owner_storage_slot(&self) -> U256 {
        U256::from_be_bytes(
            Self::storage_slot_raw(self.original_key.as_ref(), Self::OWNER_SLOT)
                .inner()
                .clone(),
        )
    }

    pub(crate) fn balance_storage_slot(&self) -> U256 {
        U256::from_be_bytes(
            Self::storage_slot_raw(self.original_key.as_ref(), Self::BALANCE_SLOT)
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

    pub(crate) fn get(&self) -> fuel_types::Address {
        self.address
    }
    pub(crate) fn fluent_address(&self) -> Address {
        Address::from_slice(&self.address[12..])
    }
}

impl From<&Address> for FuelAddress {
    fn from(value: &Address) -> Self {
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

#[derive(Default)]
pub(crate) struct CoinsOwnerWithBalanceHelper {
    address: fuel_types::Address,
    balance: u64,
}

impl CoinsOwnerWithBalanceHelper {
    pub const ENCODED_LEN: usize = size_of::<Address>() + size_of::<u64>();

    pub(crate) fn new(address: fuel_types::Address, balance: u64) -> Self {
        Self { address, balance }
    }

    pub(crate) fn from_owner(owner: &fuel_types::Address, balance: u64) -> Self {
        Self {
            address: owner.clone(),
            balance,
        }
    }

    pub(crate) fn address(&self) -> &fuel_types::Address {
        return &self.address;
    }

    pub(crate) fn balance(&self) -> u64 {
        return self.balance;
    }

    pub(crate) fn to_u256_address_balance(&self) -> (U256, U256) {
        (
            U256::from_be_slice(self.address.as_slice()),
            U256::from_limbs_slice(&[self.balance]),
        )
    }

    pub(crate) fn from_u256_address_balance(v: &(U256, U256)) -> Self {
        let address = &v.0.to_be_bytes::<32>();
        let balance = &v.1.as_limbs()[0];
        let address = fuel_types::Address::from_bytes_ref(address);
        CoinsOwnerWithBalanceHelper {
            address: *address,
            balance: *balance,
        }
    }
}
