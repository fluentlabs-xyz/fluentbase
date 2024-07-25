use crate::fvm::helpers::{
    CoinsHelper,
    CoinsOwnerWithBalanceHelper,
    ContractsAssetsHelper,
    ContractsLatestUtxoHelper,
    ContractsRawCodeHelper,
    ContractsStateHelper,
    FuelAddress,
    MetadataHelper,
};
use alloc::{vec, vec::Vec};
use core::hash::Hash;
use fluentbase_sdk::{AccountManager, Address, ContextReader, U256};
use fluentbase_types::{Bytes32, Bytes34, Bytes64};
use fuel_core_executor::ports::RelayerPort;
use fuel_core_storage::{
    self,
    codec::Encoder,
    column::Column,
    kv_store::{KeyValueInspect, KeyValueMutate, Value},
    Result as StorageResult,
};
use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    entities::coins::coin::{CompressedCoin, CompressedCoinV1},
    fuel_tx::{AssetId, ContractId},
    fuel_types::canonical::Serialize,
    services::relayer::Event,
};
use revm_primitives::hex;

pub struct WasmRelayer;

impl RelayerPort for WasmRelayer {
    fn enabled(&self) -> bool {
        false
    }

    fn get_events(&self, _: &DaBlockHeight) -> anyhow::Result<Vec<Event>> {
        Ok(vec![])
    }
}

pub const UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS: Address =
    Address::new(hex!("c5c497b0814b0eebc27864ea5ff9af596b715ee3"));
pub const CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS: Address =
    Address::new(hex!("e3d4160aa0d55eae58508cc89d6cbcab1354bdbc"));

pub struct WasmStorage<'a, CR: ContextReader, AM: AccountManager> {
    pub cr: &'a CR,
    pub am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> WasmStorage<'a, CR, AM> {
    pub(crate) fn metadata(&self, raw_key: &[u8]) -> Option<fluentbase_types::Bytes> {
        let preimage = self
            .am
            .preimage(&MetadataHelper::new(raw_key).value_preimage_key());
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn metadata_update(&self, raw_key: &[u8], data: &[u8]) {
        self.am
            .update_preimage(&MetadataHelper::new(raw_key).value_preimage_key(), 0, data);
    }
    pub(crate) fn contracts_raw_code(&self, raw_key: &Bytes32) -> Option<fluentbase_types::Bytes> {
        let preimage = self.am.preimage(
            &ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key)).value_preimage_key(),
        );
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_raw_code_update(&self, raw_key: &Bytes32, data: &[u8]) {
        self.am.update_preimage(
            &ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key)).value_preimage_key(),
            0,
            data,
        );
    }

    pub(crate) fn contracts_latest_utxo(
        &self,
        raw_key: &Bytes32,
    ) -> Option<fluentbase_types::Bytes> {
        let preimage = self.am.preimage(
            &ContractsLatestUtxoHelper::new(&ContractId::new(*raw_key)).value_preimage_key(),
        );
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_latest_utxo_update(&self, raw_key: &Bytes32, data: &[u8]) {
        self.am.update_preimage(
            &ContractsLatestUtxoHelper::new(&ContractId::new(*raw_key)).value_preimage_key(),
            0,
            data,
        );
    }

    pub(crate) fn contracts_state_data(
        &self,
        raw_key: &Bytes64,
    ) -> Option<fluentbase_types::Bytes> {
        let preimage = self
            .am
            .preimage(&ContractsStateHelper::new(raw_key).value_preimage_key());
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_state_data_update(&self, raw_key: &Bytes64, data: &[u8]) {
        self.am.update_preimage(
            &ContractsStateHelper::new(raw_key).value_preimage_key(),
            0,
            data,
        );
    }

    pub(crate) fn contracts_assets_value(&self, raw_key: &Bytes64) -> fluentbase_types::Bytes {
        let (val, _is_cold) = self.am.storage(
            CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS,
            ContractsAssetsHelper::new(raw_key).value_storage_slot(),
            false,
        );
        fluentbase_types::Bytes::copy_from_slice(
            ContractsAssetsHelper::u256_to_value(&val).as_slice(),
        )
    }

    pub(crate) fn contracts_assets_value_update(&self, raw_key: &Bytes64, value: &[u8]) {
        self.am.write_storage(
            CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS,
            ContractsAssetsHelper::new(raw_key).value_storage_slot(),
            ContractsAssetsHelper::value_to_u256(value.try_into().expect("encoded value is valid")),
        );
    }

    pub(crate) fn coins_owner_with_balance(
        &self,
        raw_key: &Bytes34,
    ) -> CoinsOwnerWithBalanceHelper {
        let (val, _is_cold) = self.am.storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            CoinsHelper::new(raw_key).owner_with_balance_storage_slot(),
            false,
        );
        CoinsOwnerWithBalanceHelper::from_u256(&val)
    }

    pub(crate) fn coins_owner_with_balance_update(
        &self,
        raw_key: &Bytes34,
        v: &CoinsOwnerWithBalanceHelper,
    ) {
        self.am.write_storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            CoinsHelper::new(raw_key).owner_with_balance_storage_slot(),
            v.to_u256(),
        );
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> KeyValueInspect for WasmStorage<'a, CR, AM> {
    type Column = Column;

    fn size_of_value(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<usize>> {
        self.get(key, column).map(|v1| v1.map(|v2| v2.size()))
    }

    fn get(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<Value>> {
        assert!(key.len() > 0, "key len greater 0");

        match column {
            Column::Metadata => {
                // key -> [u8]
                // value -> [u8]

                let raw_metadata = self.metadata(key);

                return Ok(raw_metadata.map(|v| v.to_vec()));
            }
            Column::ContractsRawCode => {
                // key -> ContractId
                // value -> [u8]

                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let raw_code = self.contracts_raw_code(&key);

                return Ok(raw_code.map(|v| v.to_vec()));
            }
            Column::ContractsState => {
                // key -> ContractsStateKey
                // value -> [u8]

                let contract_state_key: Bytes64 = key.try_into().expect("64 bytes key");
                let contracts_state_data = self.contracts_state_data(&contract_state_key);

                return Ok(contracts_state_data.map(|v| v.to_vec()));
            }
            Column::ContractsLatestUtxo => {
                // key -> ContractId
                // value -> ContractUtxoInfo

                let contract_id: Bytes32 = key.try_into().expect("32 bytes key");
                let contracts_latest_utxo_data = self.contracts_latest_utxo(&contract_id);

                return Ok(contracts_latest_utxo_data.map(|v| v.to_vec()));
            }
            Column::ContractsAssets => {
                // key -> ContractsAssetKey
                // value -> u64

                let contracts_assets_key: Bytes64 = key.try_into().expect("64 bytes key");
                let value_data = self.contracts_assets_value(&contracts_assets_key);

                return Ok(Some(value_data.to_vec()));
            }
            Column::Coins => {
                // key -> UtxoId
                // value -> CompressedCoin

                let utxo_id_key: Bytes34 = key.try_into().expect("34 bytes key");
                let owner_with_balance = self.coins_owner_with_balance(&utxo_id_key);
                let (account, _is_cold) = self.am.account(owner_with_balance.address().clone());
                let mut fuel_address_helper: FuelAddress = account.address.into();
                let amount = account.balance / U256::from(1_000_000_000);
                let compressed_coin = CompressedCoin::V1(CompressedCoinV1 {
                    owner: fuel_address_helper.address(),
                    amount: amount.as_limbs()[0], // gwei ?
                    asset_id: AssetId::BASE,
                    tx_pointer: Default::default(),
                });

                let r =
                    postcard::to_allocvec(&compressed_coin).expect("compressed coin serialized");
                return Ok(Some(r));
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata
            | Column::Messages
            | Column::ProcessedTransactions
            | Column::FuelBlockConsensus
            | Column::ConsensusParametersVersions
            | Column::StateTransitionBytecodeVersions
            | Column::UploadedBytecodes
            | Column::GenesisMetadata => {
                panic!(
                    "unsupported column referenced '{:?}' while getting data from storage",
                    &column
                )
            }
        }
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> KeyValueMutate for WasmStorage<'a, CR, AM> {
    fn write(&mut self, key: &[u8], column: Self::Column, buf: &[u8]) -> StorageResult<usize> {
        match column {
            Column::Metadata => {
                // key -> [u8]
                // value -> [u8]

                self.metadata_update(&key, buf);
            }
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
                self.contracts_state_data_update(&key, buf);
            }
            Column::ContractsLatestUtxo => {
                // key -> ContractId
                // value -> ContractUtxoInfo

                let key: Bytes32 = key.try_into().expect("32 bytes key");
                self.contracts_latest_utxo_update(&key, buf);
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
                let compressed_coin: CompressedCoin =
                    postcard::from_bytes(buf).expect("compressed coin");
                assert_eq!(compressed_coin.asset_id(), &AssetId::BASE);
                let coins_owner_with_balance = CoinsOwnerWithBalanceHelper::from_owner(
                    compressed_coin.owner(),
                    *compressed_coin.amount(),
                );
                self.coins_owner_with_balance_update(&utxo_id_key, &coins_owner_with_balance);

                let (mut account, _) = self.am.account(*coins_owner_with_balance.address());
                let mut coin_amount = U256::from(1_000_000_000);
                coin_amount = coin_amount * U256::from(coins_owner_with_balance.balance());
                account.balance += U256::from(coins_owner_with_balance.balance());
                self.am.write_account(&account);
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata
            | Column::Messages
            | Column::ProcessedTransactions
            | Column::FuelBlockConsensus
            | Column::ConsensusParametersVersions
            | Column::StateTransitionBytecodeVersions
            | Column::UploadedBytecodes
            | Column::GenesisMetadata => {
                panic!(
                    "unsupported column referenced '{:?}' while writing data",
                    &column
                )
            }
        }
        Ok(buf.len())
    }

    fn delete(&mut self, key: &[u8], column: Self::Column) -> StorageResult<()> {
        match column {
            Column::Metadata
            | Column::ContractsRawCode
            | Column::ContractsState
            | Column::ContractsLatestUtxo
            | Column::ContractsAssets
            | Column::Coins => {
                self.write(key, column, &[])?;
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata
            | Column::Messages
            | Column::ProcessedTransactions
            | Column::FuelBlockConsensus
            | Column::ConsensusParametersVersions
            | Column::StateTransitionBytecodeVersions
            | Column::UploadedBytecodes
            | Column::GenesisMetadata => {
                panic!(
                    "unsupported column referenced '{:?}' while deleting data",
                    &column
                )
            }
        }
        Ok(())
    }
}
