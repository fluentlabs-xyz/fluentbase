use alloc::{vec, vec::Vec};
use core::{ops::Deref, str::FromStr};
use fluentbase_sdk::{Address, Bytes32, Bytes34, Bytes64, SharedAPI, B256, U256};
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
use phantom_type::PhantomType;
use revm_primitives::hex;

pub const FUEL_TESTNET_BASE_ASSET_ID: &str =
    "f8f8b6283d7fa5b672b530cbb84fcccb4ff8dc40f8176ef4544ddb1f1952ad07";
pub const FUEL_TESTNET_PRIVILEGED_ADDRESS: &str =
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
        base_asset_id: AssetId::from_str(FUEL_TESTNET_BASE_ASSET_ID)
            .expect("valid asset id format"),
        block_gas_limit: block_gas_limit.unwrap_or(30000000),
        privileged_address: fuel_types::Address::from_str(FUEL_TESTNET_PRIVILEGED_ADDRESS)
            .expect("valid privileged address format"),
    })
}

pub fn fuel_testnet_consensus_params_from_cr<SDK: SharedAPI>(sdk: &SDK) -> ConsensusParameters {
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

pub trait StorageSlotPure {
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

pub trait StorageSlotCalc {
    // const COLUMN: Column;

    fn storage_slot(&self, slot: u32) -> U256;
}

pub(crate) struct StorageChunksWriter<'a, SDK, SC> {
    pub(crate) address: &'a Address,
    pub(crate) slot_calc: &'a SC,
    pub(crate) _phantom: PhantomType<SDK>,
}
impl<'a, SDK: SharedAPI, SC: StorageSlotCalc> FixedChunksWriter<'a, SDK, SC>
    for StorageChunksWriter<'a, SDK, SC>
{
    fn address(&self) -> &'a Address {
        self.address
    }

    fn slot_calc(&self) -> &'a SC {
        &self.slot_calc
    }
}
impl<'a, SDK: SharedAPI, SC: StorageSlotCalc> VariableLengthDataWriter<'a, SDK, SC>
    for StorageChunksWriter<'a, SDK, SC>
{
    fn address(&self) -> &'a Address {
        self.address
    }

    fn slot_calc(&self) -> &'a SC {
        &self.slot_calc
    }
}
pub trait FixedChunksWriter<'a, SDK: SharedAPI, SC: StorageSlotCalc + 'a> {
    fn address(&self) -> &'a Address;
    fn slot_calc(&self) -> &'a SC;

    fn write_data_chunk_padded(
        &self,
        sdk: &mut SDK,
        data: &[u8],
        chunk_index: u32,
        force_write: bool,
    ) -> usize {
        let start_index = chunk_index * U256::BYTES as u32;
        let end_index = start_index + U256::BYTES as u32;
        let data_tail_index = data.len() as u32;
        if start_index >= data_tail_index {
            if force_write {
                let _ = sdk.write_storage(self.slot_calc().storage_slot(chunk_index), U256::ZERO);
            }
            return 0;
        }
        let chunk =
            &data[start_index as usize..core::cmp::min(end_index, data_tail_index) as usize];
        let value = U256::from_le_slice(chunk);
        let _ = sdk.write_storage(self.slot_calc().storage_slot(chunk_index), value);
        chunk.len()
    }

    fn write_data_in_padded_chunks(
        &self,
        sdk: &mut SDK,
        data: &[u8],
        tail_chunk_index: u32,
        force_write: bool,
    ) -> usize {
        let mut len_written = 0;
        for chunk_index in 0..=tail_chunk_index {
            len_written += self.write_data_chunk_padded(sdk, data, chunk_index, force_write)
        }
        len_written
    }

    fn read_data_chunk_padded(&self, sdk: &'a SDK, chunk_index: u32, buf: &mut Vec<u8>) {
        let slot = self.slot_calc().storage_slot(chunk_index);
        let value = sdk.storage(&slot);
        buf.extend_from_slice(value.as_le_slice());
    }

    fn read_data_in_padded_chunks(&self, sdk: &'a SDK, tail_chunk_index: u32, buf: &mut Vec<u8>) {
        for chunk_index in 0..=tail_chunk_index {
            self.read_data_chunk_padded(sdk, chunk_index, buf)
        }
    }
}
pub trait VariableLengthDataWriter<'a, SDK: SharedAPI, SC: StorageSlotCalc + 'a> {
    fn address(&self) -> &'a Address;

    fn slot_calc(&self) -> &'a SC;

    fn write_data(&self, sdk: &mut SDK, data: &[u8]) -> usize {
        let data_len = data.len();
        if data_len <= 0 {
            return 0;
        }
        let slot = self.slot_calc().storage_slot(0);
        let _ = sdk.write_storage(slot, U256::from(data_len));
        let chunks_count = (data_len - 1) / U256::BYTES + 1;
        for chunk_index in 0..chunks_count {
            let chunk_start_index = chunk_index * U256::BYTES;
            let chunk_end_index = core::cmp::min(data_len, chunk_start_index + U256::BYTES);
            let chunk = &data[chunk_start_index..chunk_end_index];
            let value = U256::from_le_slice(chunk);
            let slot = self.slot_calc().storage_slot(chunk_index as u32 + 1);
            let _ = sdk.write_storage(slot, value);
        }
        data_len
    }

    fn read_data(&self, sdk: &SDK, buf: &mut Vec<u8>) -> anyhow::Result<()> {
        let slot = self.slot_calc().storage_slot(0);
        let data_len = sdk.storage(&slot);
        let data_len: usize = data_len
            .try_into()
            .map_err(|e| anyhow::Error::msg("failed to decode result data len"))?;
        if data_len <= 0 {
            return Ok(());
        }
        let chunks_count = (data_len - 1) / U256::BYTES + 1;
        for chunk_index in 0..chunks_count - 1 {
            let slot = self.slot_calc().storage_slot(chunk_index as u32 + 1);
            let value = sdk.storage(&slot);
            let value = value.as_le_slice();
            buf.extend_from_slice(value);
        }
        let chunk_index = chunks_count - 1;
        let last_chunk_len = data_len - U256::BYTES * chunk_index;
        let slot = self.slot_calc().storage_slot(chunk_index as u32 + 1);
        let value = sdk.storage(&slot);
        let value = &value.as_le_slice()[0..last_chunk_len];
        buf.extend_from_slice(value);

        Ok(())
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

impl Into<U256> for IndexedHash {
    fn into(self) -> U256 {
        U256::from_le_bytes(self.0)
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

impl StorageSlotPure for ContractsRawCodeHelper {
    const COLUMN: Column = Column::ContractsRawCode;
}

impl StorageSlotCalc for ContractsRawCodeHelper {
    // const COLUMN: Column = Column::ContractsRawCode;

    fn storage_slot(&self, slot: u32) -> U256 {
        <Self as StorageSlotPure>::storage_slot(&self.original_key, slot).into()
    }
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

impl StorageSlotPure for ContractsLatestUtxoHelper {
    const COLUMN: Column = Column::ContractsLatestUtxo;
}

impl StorageSlotCalc for ContractsLatestUtxoHelper {
    // const COLUMN: Column = Column::ContractsLatestUtxo;

    fn storage_slot(&self, slot: u32) -> U256 {
        <Self as StorageSlotPure>::storage_slot(&self.original_key, slot).into()
    }
}

impl ContractsLatestUtxoHelper {
    pub fn new(contract_id: &ContractId) -> Self {
        Self {
            original_key: *contract_id,
        }
    }
}

pub struct DepositWithdrawalIndexHelper<'a, SDK> {
    sdk: &'a mut SDK,
}

impl<'a, SDK: SharedAPI> DepositWithdrawalIndexHelper<'a, SDK> {
    pub const BASE_SLOT: U256 = U256::from_be_bytes(hex!(
        "3b7beb7da1bf6fe0385840aec5f2bb7a20a36c96e298b5f66760841e0e77a209"
    ));
    pub const BASE_INDEX: U256 = U256::from_be_bytes(hex!(
        "0000000000000000000000000000000000000000000000000012300000000000"
    ));
    pub fn new(sdk: &'a mut SDK) -> Self {
        Self { sdk }
    }

    const fn slot(&self) -> U256 {
        Self::BASE_SLOT
    }

    pub fn next_index(&mut self) -> U256 {
        let current_index = self.sdk.storage(&self.slot());
        let next_index = current_index + U256::from(1);
        self.sdk.write_storage(self.slot(), next_index);

        Self::BASE_INDEX + current_index
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

impl StorageSlotPure for ContractsStateHelper {
    const COLUMN: Column = Column::ContractsState;
}

impl StorageSlotCalc for ContractsStateHelper {
    // const COLUMN: Column = Column::ContractsState;

    fn storage_slot(&self, slot: u32) -> U256 {
        match self.key {
            ContractsStateKeyWrapper::Original(v) => {
                <Self as StorageSlotPure>::storage_slot_raw(v.as_ref(), slot).into()
            }
            ContractsStateKeyWrapper::Transformed(v) => {
                <Self as StorageSlotPure>::storage_slot(&v, slot).into()
            }
        }
    }
}

impl ContractsStateHelper {
    const VALUE_STORAGE_SLOT: u32 = 0;

    pub(crate) fn new(key: &Bytes64) -> Self {
        Self {
            key: ContractsStateKeyWrapper::Original(ContractsStateKey::from_array(*key)),
        }
    }

    pub(crate) fn new_transformed(key: &Bytes32) -> Self {
        Self {
            key: ContractsStateKeyWrapper::Transformed(*key),
        }
    }

    pub(crate) fn get(&self) -> &ContractsStateKeyWrapper {
        &self.key
    }

    pub(crate) fn value_storage_slot(&self) -> IndexedHash {
        if let ContractsStateKeyWrapper::Original(key) = self.key {
            return Self::storage_slot_raw(key.as_ref(), Self::VALUE_STORAGE_SLOT);
        }
        panic!("original key expected")
    }
}

pub(crate) enum ContractsAssetKeyWrapper {
    Original(ContractsAssetKey),
    Transformed(Bytes32),
}

pub(crate) struct ContractsAssetsHelper {
    key: ContractsAssetKeyWrapper,
}

impl StorageSlotPure for ContractsAssetsHelper {
    const COLUMN: Column = Column::ContractsAssets;
}

impl StorageSlotCalc for ContractsAssetsHelper {
    // const COLUMN: Column = Column::ContractsAssets;

    fn storage_slot(&self, slot: u32) -> U256 {
        match self.key {
            ContractsAssetKeyWrapper::Original(v) => {
                <Self as StorageSlotPure>::storage_slot_raw(v.as_ref(), slot).into()
            }
            ContractsAssetKeyWrapper::Transformed(v) => {
                <Self as StorageSlotPure>::storage_slot(&v, slot).into()
            }
        }
    }
}

impl ContractsAssetsHelper {
    const VALUE_STORAGE_SLOT: u32 = 0;
    pub(crate) fn new(original_key: &Bytes64) -> Self {
        Self {
            key: ContractsAssetKeyWrapper::Original(ContractsAssetKey::from_array(*original_key)),
        }
    }
    pub(crate) fn new_transformed(key: &Bytes32) -> Self {
        Self {
            key: ContractsAssetKeyWrapper::Transformed(*key),
        }
    }

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

    // pub(crate) fn merkle_data_preimage_key(&self) -> IndexedHash {
    //     if let ContractsAssetKeyWrapper::Transformed(key) = self.key {
    //         return Self::storage_slot_raw(key.as_ref(), Self::MERKLE_DATA_STORAGE_SLOT);
    //     }
    //     panic!("transformed key expected")
    // }
    //
    // pub(crate) fn merkle_metadata_preimage_key(&self) -> IndexedHash {
    //     if let ContractsAssetKeyWrapper::Transformed(key) = self.key {
    //         return Self::storage_slot_raw(key.as_ref(), Self::MERKLE_METADATA_STORAGE_SLOT);
    //     }
    //     panic!("transformed key expected")
    // }
}

pub(crate) struct CoinsHelper {
    original_key: Bytes34, // UtxoId as a key
}

impl StorageSlotPure for CoinsHelper {
    const COLUMN: Column = Column::Coins;
}

impl StorageSlotCalc for CoinsHelper {
    // const COLUMN: Column = Column::Coins;

    fn storage_slot(&self, slot: u32) -> U256 {
        <Self as StorageSlotPure>::storage_slot_raw(&self.original_key, slot).into()
    }
}

impl CoinsHelper {
    const OWNER_WITH_BALANCE_SLOT: u32 = 0;
    const OWNER_SLOT: u32 = 1;
    const ASSET_ID_SLOT: u32 = 2;
    const BALANCE_SLOT: u32 = 3;
    pub(crate) fn new(key: &Bytes34) -> Self {
        Self { original_key: *key }
    }

    pub(crate) fn from_slice(v: &[u8]) -> Self {
        let original_key = Bytes34::from_bytes(v).expect("valid utxo id key");
        Self { original_key }
    }

    pub(crate) fn get(&self) -> Bytes34 {
        self.original_key
    }

    pub(crate) fn owner_storage_slot(&self) -> U256 {
        U256::from_be_bytes(
            Self::storage_slot_raw(self.original_key.as_ref(), Self::OWNER_SLOT)
                .inner()
                .clone(),
        )
    }

    pub(crate) fn asset_id_storage_slot(&self) -> U256 {
        U256::from_be_bytes(
            Self::storage_slot_raw(self.original_key.as_ref(), Self::ASSET_ID_SLOT)
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

pub struct FuelAddress {
    address: fuel_types::Address,
}

impl FuelAddress {
    pub fn new(address: fuel_types::Address) -> Self {
        Self { address }
    }
    pub fn from_evm_address(evm_address: &Address) -> Self {
        let mut address = fuel_types::Address::zeroed();
        address.0.as_mut_slice()[12..].copy_from_slice(evm_address.as_slice());
        Self { address }
    }
    pub fn new_zero() -> Self {
        const ADDRESS: fuel_types::Address = fuel_types::Address::zeroed();
        Self { address: ADDRESS }
    }
    pub fn new_max() -> Self {
        const ADDRESS: fuel_types::Address = fuel_types::Address::new([0xff; 32]);
        Self { address: ADDRESS }
    }

    pub fn get(&self) -> fuel_types::Address {
        self.address
    }
    pub fn fluent_address(&self) -> Address {
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
        &self.address
    }
}

// #[derive(Default)]
// pub(crate) struct CoinsHolderHelper {
//     address: fuel_types::Address,
//     asset_id: AssetId,
//     balance: u64,
// }
//
// impl CoinsHolderHelper {
//     pub const ENCODED_LEN: usize = size_of::<Address>() + size_of::<u64>();
//
//     pub(crate) fn new(address: fuel_types::Address, asset_id: AssetId, balance: u64) -> Self {
//         Self {
//             address,
//             asset_id,
//             balance,
//         }
//     }
//
//     pub(crate) fn from(owner: &fuel_types::Address, asset_id: AssetId, balance: u64) -> Self {
//         Self {
//             address: owner.clone(),
//             asset_id,
//             balance,
//         }
//     }
//
//     pub(crate) fn address(&self) -> &fuel_types::Address {
//         &self.address
//     }
//
//     pub(crate) fn asset_id(&self) -> &AssetId {
//         &self.asset_id
//     }
//
//     pub(crate) fn balance(&self) -> u64 {
//         self.balance
//     }
//
//     pub(crate) fn to_u256_tuple(&self) -> (U256, U256, U256) {
//         (
//             U256::from_le_slice(self.address.as_slice()),
//             U256::from_le_slice(self.asset_id.as_slice()),
//             U256::from_limbs_slice(&[self.balance]),
//         )
//     }
//
//     pub(crate) fn from_u256_tuple(address: &U256, asset_id: &U256, balance: &U256) -> Self {
//         let address = fuel_types::Address::from_bytes(&address.to_le_bytes::<32>()).unwrap();
//         let asset_id = AssetId::from_bytes(&asset_id.to_le_bytes::<32>()).unwrap();
//         let balance = balance.as_limbs()[0];
//         CoinsHolderHelper {
//             address,
//             asset_id,
//             balance,
//         }
//     }
// }
