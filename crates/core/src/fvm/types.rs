use crate::fvm::helpers::{
    CoinsHelper,
    CoinsHolderHelper,
    ContractsAssetsHelper,
    ContractsLatestUtxoHelper,
    ContractsRawCodeHelper,
    ContractsStateHelper,
    FixedChunksWriter,
    FuelAddress,
    StorageChunksWriter,
    VariableLengthDataWriter,
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    AccountStatus,
    Address,
    Bytes,
    Bytes32,
    Bytes34,
    Bytes64,
    SovereignAPI,
    U256,
};
use fuel_core_executor::ports::RelayerPort;
use fuel_core_storage::{
    self,
    column::Column,
    kv_store::{KeyValueInspect, KeyValueMutate, Value, WriteOperation},
    transactional::{Changes, Modifiable},
    Result as StorageResult,
};
use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    entities::coins::coin::{CompressedCoin, CompressedCoinV1},
    fuel_tx::ContractId,
    services::relayer::Event,
};
use revm_primitives::{
    alloy_primitives::private::serde::de::IntoDeserializer,
    bitvec::macros::internal::funty::Fundamental,
    hex,
};

pub struct WasmRelayer;

impl RelayerPort for WasmRelayer {
    fn enabled(&self) -> bool {
        false
    }

    fn get_events(&self, _: &DaBlockHeight) -> anyhow::Result<Vec<Event>> {
        Ok(vec![])
    }
}

pub const CONTRACTS_RAW_CODE_STORAGE_ADDRESS: Address =
    Address::new(hex!("ba8ab429ff0aaa5f1bb8f19f1f9974ffc82ff161"));
pub const UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS: Address =
    Address::new(hex!("c5c497b0814b0eebc27864ea5ff9af596b715ee3"));
pub const CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS: Address =
    Address::new(hex!("e3d4160aa0d55eae58508cc89d6cbcab1354bdbc"));
pub const CONTRACTS_LATEST_UTXO_STORAGE_ADDRESS: Address =
    Address::new(hex!("eb4cc317c536bff071ef700e2f3d2f2701e4e9e5"));
pub const CONTRACTS_STATE_DATA_STORAGE_ADDRESS: Address =
    Address::new(hex!("4ac7fb43ea3ae6330ffdb14ec65c17ec8eace55d"));
pub const CONTRACTS_STATE_MERKLE_DATA_STORAGE_ADDRESS: Address =
    Address::new(hex!("1a456cdbe1c54e7a774dd89d659c128d56dba51d"));
pub const CONTRACTS_STATE_MERKLE_METADATA_STORAGE_ADDRESS: Address =
    Address::new(hex!("727d22651ab98fcf20fa7bdd646e71102c6ac47b"));
pub const CONTRACTS_ASSETS_MERKLE_DATA_STORAGE_ADDRESS: Address =
    Address::new(hex!("037e25b327c1a5acc4a98e8e2e8d16066119eeed"));
pub const CONTRACTS_ASSETS_MERKLE_METADATA_STORAGE_ADDRESS: Address =
    Address::new(hex!("f96178848125f6d39487bd426a42adf7129ba924"));

pub const STORAGE_ADDRESSES: [Address; 9] = [
    CONTRACTS_RAW_CODE_STORAGE_ADDRESS,
    UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
    CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS,
    CONTRACTS_LATEST_UTXO_STORAGE_ADDRESS,
    CONTRACTS_STATE_DATA_STORAGE_ADDRESS,
    CONTRACTS_STATE_MERKLE_DATA_STORAGE_ADDRESS,
    CONTRACTS_STATE_MERKLE_METADATA_STORAGE_ADDRESS,
    CONTRACTS_ASSETS_MERKLE_DATA_STORAGE_ADDRESS,
    CONTRACTS_ASSETS_MERKLE_METADATA_STORAGE_ADDRESS,
];

const CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN: usize = 44;
const CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN: usize = 66;
const CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN: usize = 33;
const CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN: usize = 66;
const CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN: usize = 33;

pub struct WasmStorage<'a, SDK: SovereignAPI> {
    pub sdk: &'a mut SDK,
}

impl<'a, SDK: SovereignAPI> WasmStorage<'a, SDK> {
    // pub(crate) fn metadata_update(&mut self, raw_key: &[u8], data: &[u8]) {
    //     let key: B256 = MetadataHelper::new(raw_key).value_preimage_key().into();
    //     self.sdk
    //         .write_preimage(Address::ZERO, key, Bytes::copy_from_slice(data));
    // }
    //
    // pub(crate) fn metadata(&self, raw_key: &[u8]) -> Option<Bytes> {
    //     let key: B256 = MetadataHelper::new(raw_key).value_preimage_key().into();
    //     self.sdk.preimage(&key).filter(|v| !v.is_empty())
    // }

    pub(crate) fn contracts_raw_code_update(&mut self, raw_key: &Bytes32, data: &[u8]) {
        let helper = ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_RAW_CODE_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let _ = storage_chunks.write_data(self.sdk, &helper, data);
    }

    pub(crate) fn contracts_raw_code(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_RAW_CODE_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let mut buf = Vec::new();
        storage_chunks
            .read_data(self.sdk, &helper, &mut buf)
            .expect("raw code extracted successfully");
        if buf.len() <= 0 {
            return None;
        }
        Some(buf.into())
    }

    pub(crate) fn contracts_latest_utxo_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN,
            anyhow::Error::msg("ContractsLatestUtxo encoded len must be <= 44")
        );
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_LATEST_UTXO_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsLatestUtxoHelper::new(ContractId::from_bytes_ref(raw_key));
        storage_chunks.write_data_in_padded_chunks(
            self.sdk,
            &helper,
            data,
            (CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN / 32) as u32,
            true,
        );
        Ok(())
    }

    pub(crate) fn contracts_latest_utxo(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsLatestUtxoHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_LATEST_UTXO_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        const CAPACITY: usize = ((CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, &helper, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub(crate) fn contracts_state_data_update(&mut self, raw_key: &Bytes64, value: Bytes32) {
        let slot: U256 = ContractsStateHelper::new(raw_key)
            .value_storage_slot()
            .into();
        self.sdk.write_storage(
            CONTRACTS_STATE_DATA_STORAGE_ADDRESS,
            slot,
            U256::from_be_bytes(value),
        );
    }

    pub(crate) fn contracts_state_data(&self, raw_key: &Bytes64) -> Option<Bytes> {
        let slot: U256 = ContractsStateHelper::new(raw_key)
            .value_storage_slot()
            .into();
        let (v, _) = self
            .sdk
            .storage(&CONTRACTS_STATE_DATA_STORAGE_ADDRESS, &slot);
        if v == U256::ZERO {
            return None;
        }
        Some(v.to_be_bytes_vec().into())
    }

    pub(crate) fn contracts_assets_value_update(&mut self, raw_key: &Bytes64, value: &[u8]) {
        let slot = ContractsAssetsHelper::new(raw_key).value_storage_slot();
        let value =
            ContractsAssetsHelper::value_to_u256(value.try_into().expect("encoded value is valid"));
        self.sdk
            .write_storage(CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS, slot, value);
    }

    pub(crate) fn contracts_assets_value(&self, raw_key: &Bytes64) -> Option<Bytes> {
        let slot = ContractsAssetsHelper::new(raw_key).value_storage_slot();
        let (val, _is_cold) = self
            .sdk
            .storage(&CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS, &slot);
        if val == U256::ZERO {
            return None;
        }
        Some(Bytes::copy_from_slice(
            ContractsAssetsHelper::u256_to_value(&val).as_slice(),
        ))
    }

    pub(crate) fn contracts_state_merkle_data_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("merkle_data encoded len must be <= 66")
        );
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_STATE_MERKLE_DATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsStateHelper::new_transformed(raw_key);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, &helper, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub(crate) fn contracts_state_merkle_data(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_STATE_MERKLE_DATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsStateHelper::new_transformed(raw_key);
        const CAPACITY: usize = ((CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, &helper, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub(crate) fn contracts_state_merkle_metadata_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("merkle_metadata encoded len must be <= 33")
        );
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_STATE_MERKLE_METADATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsStateHelper::new_transformed(raw_key);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, &helper, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub(crate) fn contracts_state_merkle_metadata(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_STATE_MERKLE_METADATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsStateHelper::new_transformed(raw_key);
        const CAPACITY: usize =
            ((CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, &helper, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub(crate) fn contracts_assets_merkle_data_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("merkle_data encoded len must be <= 66")
        );
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_ASSETS_MERKLE_DATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, &helper, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub(crate) fn contracts_assets_merkle_data(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_ASSETS_MERKLE_DATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        const CAPACITY: usize = ((CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, &helper, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub(crate) fn contracts_assets_merkle_metadata_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("merkle_metadata encoded len must be <= 33")
        );
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_ASSETS_MERKLE_METADATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, &helper, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub(crate) fn contracts_assets_merkle_metadata(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let mut storage_chunks = StorageChunksWriter {
            address: &CONTRACTS_ASSETS_MERKLE_METADATA_STORAGE_ADDRESS,
            _phantom: Default::default(),
        };
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        const CAPACITY: usize =
            ((CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, &helper, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub(crate) fn coins_update(&mut self, raw_key: &Bytes34, v: &CoinsHolderHelper) {
        let (address, asset_id, balance) = v.to_u256_tuple();
        let ch = CoinsHelper::new(raw_key);
        self.sdk.write_storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            ch.owner_storage_slot(),
            address,
        );
        self.sdk.write_storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            ch.asset_id_slot(),
            asset_id,
        );
        self.sdk.write_storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            ch.balance_storage_slot(),
            balance,
        );
    }

    pub(crate) fn coins(&self, raw_key: &Bytes34) -> Option<CoinsHolderHelper> {
        let ch = CoinsHelper::new(raw_key);
        let (owner, _is_cold) = self.sdk.storage(
            &UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            &ch.owner_storage_slot(),
        );
        if owner == U256::ZERO {
            return None;
        }
        let (asset_id, _is_cold) = self.sdk.storage(
            &UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            &ch.asset_id_slot(),
        );
        let (balance, _is_cold) = self.sdk.storage(
            &UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            &ch.balance_storage_slot(),
        );
        Some(CoinsHolderHelper::from_u256_tuple(
            &owner, &asset_id, &balance,
        ))
    }
}

impl<'a, SDK: SovereignAPI> KeyValueInspect for WasmStorage<'a, SDK> {
    type Column = Column;

    fn size_of_value(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<usize>> {
        self.get(key, column).map(|v1| v1.map(|v2| v2.len()))
    }

    fn get(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<Value>> {
        assert!(key.len() > 0, "key len greater 0");

        match column {
            // Column::Metadata => {
            //     // key -> [u8]
            //     // value -> [u8]
            //
            //     let raw_metadata = self.metadata(key);
            //
            //     Ok(raw_metadata.map(|v| v.to_vec()))
            // }
            Column::ContractsRawCode => {
                // key -> ContractId
                // value -> [u8]

                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let raw_code = self.contracts_raw_code(&key);

                Ok(raw_code.map(|v| v.to_vec()))
            }
            Column::ContractsState => {
                // key -> ContractsStateKey
                // value -> [u8]

                let contract_state_key: Bytes64 = key.try_into().expect("64 bytes key");
                let contracts_state_data = self.contracts_state_data(&contract_state_key);

                Ok(contracts_state_data.map(|v| v.into()))
            }
            Column::ContractsLatestUtxo => {
                // key -> ContractId
                // value -> ContractUtxoInfo

                let contract_id: Bytes32 = key.try_into().expect("32 bytes key");
                let contracts_latest_utxo_data = self.contracts_latest_utxo(&contract_id);

                Ok(contracts_latest_utxo_data.map(|v| v.to_vec()))
            }
            Column::ContractsAssets => {
                // key -> ContractsAssetKey
                // value -> u64

                let contracts_assets_key: Bytes64 = key.try_into().expect("64 bytes key");
                let value_data = self.contracts_assets_value(&contracts_assets_key);

                Ok(value_data.map(|v| v.to_vec()))
            }
            Column::Coins => {
                // key -> UtxoId
                // value -> CompressedCoin

                let utxo_id_key: Bytes34 = key.try_into().expect("34 bytes key");
                let Some(coins_holder_helper) = self.coins(&utxo_id_key) else {
                    return Ok(None);
                };
                let fuel_address = FuelAddress::new(*coins_holder_helper.address());
                let (account, _is_cold) = self.sdk.account(&fuel_address.fluent_address());
                let amount = account.balance / U256::from(1_000_000_000);
                let compressed_coin = CompressedCoin::V1(CompressedCoinV1 {
                    owner: fuel_address.get(),
                    amount: amount.as_limbs()[0],
                    asset_id: *coins_holder_helper.asset_id(),
                    tx_pointer: Default::default(),
                });

                let r =
                    postcard::to_allocvec(&compressed_coin).expect("compressed coin serialized");
                Ok(Some(r))
            }

            Column::ContractsStateMerkleData => {
                // key - 32 bytes
                // value - 66 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_state_merkle_data(&key);

                Ok(data.map(|v| v.to_vec()))
            }
            Column::ContractsStateMerkleMetadata => {
                // key - 32 bytes
                // value - 33 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_state_merkle_metadata(&key);

                Ok(data.map(|v| v.to_vec()))
            }

            Column::ContractsAssetsMerkleData => {
                // key - 32 bytes
                // value - 66 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_assets_merkle_data(&key);

                Ok(data.map(|v| v.to_vec()))
            }
            Column::ContractsAssetsMerkleMetadata => {
                // key - 32 bytes
                // value - 33 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_assets_merkle_metadata(&key);

                Ok(data.map(|v| v.to_vec()))
            }

            _ => {
                panic!(
                    "unsupported column referenced '{:?}' while getting data from storage",
                    &column
                )
            }
        }
    }
}

impl<'a, SDK: SovereignAPI> KeyValueMutate for WasmStorage<'a, SDK> {
    fn write(&mut self, key: &[u8], column: Self::Column, buf: &[u8]) -> StorageResult<usize> {
        match column {
            // Column::Metadata => {
            //     // key -> [u8]
            //     // value -> [u8]
            //
            //     self.metadata_update(&key, buf);
            // }
            Column::ContractsRawCode => {
                // key -> ContractId
                // value -> [u8]

                let key: Bytes32 = key.try_into().expect("32 bytes key");
                self.contracts_raw_code_update(&key, buf);
            }
            Column::ContractsState => {
                // key -> ContractsStateKey
                // value -> [u8]

                let key: Bytes64 = key.try_into().expect("64 bytes key");
                let value: Bytes32 = buf.try_into().expect("32 bytes value");
                self.contracts_state_data_update(&key, value);
            }
            Column::ContractsLatestUtxo => {
                // key -> ContractId
                // value -> ContractUtxoInfo

                let key: Bytes32 = key.try_into().expect("32 bytes key");
                assert!(
                    self.contracts_latest_utxo_update(&key, buf).is_ok(),
                    "contracts_latest_utxo update must succeed"
                );
            }
            Column::ContractsAssets => {
                // key -> ContractsAssetKey
                // value -> u64

                let key: Bytes64 = key.try_into().expect("64 bytes key");
                self.contracts_assets_value_update(&key, buf);
            }
            Column::Coins => {
                // key -> UtxoId
                // value -> CompressedCoin

                let utxo_id_key: Bytes34 = key.try_into().expect("34 bytes key");

                if buf.len() <= 0 {
                    // deletion process
                    let old_value = KeyValueInspect::get(&self, key, column)?;
                    if let Some(old_value) = old_value {
                        let compressed_coin: CompressedCoin =
                            postcard::from_bytes(old_value.as_slice())
                                .expect("compressed coin recovered");
                        let fuel_address = FuelAddress::new(*compressed_coin.owner());
                        let (mut account, _) = self.sdk.account(&fuel_address.fluent_address());
                        account.balance -= U256::from(1_000_000_000)
                            * U256::from(compressed_coin.amount().as_u64());
                        self.sdk.write_account(account, AccountStatus::Modified);
                    }
                    // delete existing mapping
                    let coins_owner_with_balance = CoinsHolderHelper::default();
                    self.coins_update(&utxo_id_key, &coins_owner_with_balance);

                    return Ok(0);
                }

                let compressed_coin: CompressedCoin =
                    postcard::from_bytes(buf).expect("compressed coin");
                let coins = CoinsHolderHelper::from(
                    compressed_coin.owner(),
                    *compressed_coin.asset_id(),
                    *compressed_coin.amount(),
                );
                self.coins_update(&utxo_id_key, &coins);

                let fuel_address = FuelAddress::new(*coins.address());
                let (mut account, _) = self.sdk.account(&fuel_address.fluent_address());
                let coin_amount = U256::from(1_000_000_000) * U256::from(coins.balance());
                account.balance += coin_amount;
                self.sdk.write_account(account, AccountStatus::Modified);
            }

            Column::ContractsStateMerkleData => {
                // key - 32 bytes
                // value - 66 bytes
                assert!(
                    buf.len() == 66 || buf.len() == 0,
                    "buf len invalid: {}",
                    buf.len()
                );
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                self.contracts_state_merkle_data_update(&key, buf)
                    .map_err(|_| {
                        fuel_core_storage::Error::Other(anyhow::Error::msg(
                            "failed to write key-value for ContractsStateMerkleData",
                        ))
                    })?;
            }
            Column::ContractsStateMerkleMetadata => {
                // key - 32 bytes
                // value - 33 bytes
                assert!(
                    buf.len() == 33 || buf.len() == 0,
                    "buf len invalid: {}",
                    buf.len()
                );
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                self.contracts_state_merkle_metadata_update(&key, buf)
                    .map_err(|_| {
                        fuel_core_storage::Error::Other(anyhow::Error::msg(
                            "failed to write key-value for ContractsStateMerkleMetadata",
                        ))
                    })?;
            }

            Column::ContractsAssetsMerkleData => {
                // key - 32 bytes
                // value - 66 bytes
                assert!(
                    buf.len() == 66 || buf.len() == 0,
                    "buf len invalid: {}",
                    buf.len()
                );
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                self.contracts_assets_merkle_data_update(&key, buf)
                    .map_err(|_| {
                        fuel_core_storage::Error::Other(anyhow::Error::msg(
                            "failed to write key-value for ContractsAssetsMerkleData",
                        ))
                    })?;
            }
            Column::ContractsAssetsMerkleMetadata => {
                // key - 32 bytes
                // value - 33 bytes
                assert!(
                    buf.len() == 33 || buf.len() == 0,
                    "buf len invalid: {}",
                    buf.len()
                );
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                self.contracts_assets_merkle_metadata_update(&key, buf)
                    .map_err(|_| {
                        fuel_core_storage::Error::Other(anyhow::Error::msg(
                            "failed to write key-value for ContractsAssetsMerkleMetadata",
                        ))
                    })?;
            }

            _ => {
                return Ok(0);
            }
        }
        Ok(buf.len())
    }

    fn delete(&mut self, key: &[u8], column: Self::Column) -> StorageResult<()> {
        match column {
            Column::ContractsRawCode
            | Column::ContractsState
            | Column::ContractsLatestUtxo
            | Column::ContractsAssets
            | Column::Coins
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata => {
                self.write(key, column, &[])?;
            }

            _ => {
                panic!(
                    "unsupported column referenced '{:?}' while deleting data",
                    &column
                )
            }
        }
        Ok(())
    }
}

impl<'a, SDK: SovereignAPI> Modifiable for WasmStorage<'a, SDK> {
    fn commit_changes(&mut self, changes: Changes) -> StorageResult<()> {
        for (column_u32, ops) in &changes {
            let column = Column::try_from(*column_u32).expect("valid column number");
            for (key, op) in ops {
                match op {
                    WriteOperation::Insert(v) => {
                        let _count = self.write(key, column, v.as_slice());
                    }
                    WriteOperation::Remove => {
                        let _count = self.delete(key, column);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use fuel_core::txpool::types::TxId;
    use fuel_core_types::{
        entities::contract::{ContractUtxoInfo, ContractUtxoInfoV1},
        fuel_tx::{TxPointer, UtxoId},
        fuel_types::BlockHeight,
    };

    #[test]
    fn max_sizes_encoded() {
        let v = ContractUtxoInfo::V1(ContractUtxoInfoV1 {
            utxo_id: UtxoId::new(TxId::new([0xffu8; 32]), u16::MAX),
            tx_pointer: TxPointer::new(BlockHeight::new(u32::MAX), u16::MAX),
        });
        let res = postcard::to_allocvec(&v).unwrap();
        assert_eq!(44, res.len());
    }
}
