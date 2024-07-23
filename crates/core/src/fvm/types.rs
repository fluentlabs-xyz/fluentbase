use crate::fvm::helpers::{
    ContractsAssetKeyWrapper,
    ContractsStateKeyWrapper,
    FuelAddressWrapper,
    Hashable,
    IndexedHash,
    OwnerBalanceWrapper,
    UtxoIdWrapper,
};
use alloc::{vec, vec::Vec};
use core::hash::Hash;
use fluentbase_sdk::{AccountManager, Address, ContextReader, U256};
use fluentbase_types::Bytes32;
use fuel_core_executor::ports::RelayerPort;
use fuel_core_storage::{
    self,
    codec::Encoder,
    column::Column,
    kv_store::{KeyValueInspect, Value},
    ContractsStateData,
    Result as StorageResult,
};
use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    entities::coins::coin::{CompressedCoin, CompressedCoinV1},
    fuel_tx::AssetId,
    fuel_types::canonical::{Deserialize, Serialize},
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
pub const CONTRACT_ASSET_KEY_TO_VALUE: Address =
    Address::new(hex!("e3d4160aa0d55eae58508cc89d6cbcab1354bdbc"));

pub struct WasmStorage<'a, CR: ContextReader, AM: AccountManager> {
    pub cr: &'a CR,
    pub am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> WasmStorage<'a, CR, AM> {
    pub(crate) fn owner_balance_wrapper(
        &self,
        utxo_id_wrapper: &UtxoIdWrapper,
    ) -> OwnerBalanceWrapper {
        let (storage_value, _is_cold) = self.am.storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            U256::from_be_bytes(utxo_id_wrapper.hash()),
            false,
        );
        OwnerBalanceWrapper::decode_from_u256(&storage_value)
    }

    pub(crate) fn utxo_id_wrapper(&self, key: &Bytes32) -> UtxoIdWrapper {
        let preimage = self.am.preimage(key);
        UtxoIdWrapper::decode(&preimage)
    }

    pub(crate) fn contract_state_key_wrapper(&self, key: &Bytes32) -> ContractsStateKeyWrapper {
        let preimage = self.am.preimage(key);
        ContractsStateKeyWrapper::new_from_slice(&preimage)
    }

    pub(crate) fn contracts_latest_utxo(&self, key: &Bytes32) -> fluentbase_types::Bytes {
        self.preimage_by_hash_column(&key, Column::ContractsLatestUtxo)
    }

    pub(crate) fn contracts_raw_code(&self, key: &Bytes32) -> fluentbase_types::Bytes {
        self.preimage_by_hash_column(&key, Column::ContractsRawCode)
    }

    pub(crate) fn contract_state_data(
        &self,
        cskw: &ContractsStateKeyWrapper,
    ) -> ContractsStateData {
        let preimage = self.preimage_by_hash_column(&cskw.hash(), Column::ContractsState);
        ContractsStateData::from(preimage.to_vec())
    }

    pub(crate) fn contract_asset_key_wrapper(&self, key: &Bytes32) -> ContractsAssetKeyWrapper {
        let preimage = self.preimage_by_hash_column(key, Column::ContractsAssets);
        ContractsAssetKeyWrapper::new_from_slice(&preimage)
    }

    pub(crate) fn contract_asset_value(&self, cakw: &ContractsAssetKeyWrapper) -> u64 {
        let (val, _is_cold) = self.am.storage(
            CONTRACT_ASSET_KEY_TO_VALUE,
            U256::from_be_bytes(cakw.hash()),
            false,
        );
        val.as_limbs()[0]
    }

    pub(crate) fn preimage_by_hash_column(
        &self,
        hash: &Bytes32,
        column: Column,
    ) -> fluentbase_types::Bytes {
        let indexed_hash = IndexedHash::new(hash).update_with_index(column.as_u32());
        self.am.preimage(&indexed_hash)
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> KeyValueInspect for WasmStorage<'a, CR, AM> {
    type Column = Column;

    fn size_of_value(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<usize>> {
        self.get(key, column).map(|v1| v1.map(|v2| v2.size()))
    }

    fn get(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<Value>> {
        let size = self.size_of_value(key, column)?;
        let key: Bytes32 = key.try_into().expect("32 bytes key");

        if let Some(size) = size {
            let mut value = vec![0u8; size];
            match column {
                Column::Metadata => {
                    // TODO hardcode it or compute?
                }
                Column::ContractsRawCode => {
                    let contract_id = key;
                    let raw_code = self.contracts_raw_code(&contract_id);
                    return Ok(Some(raw_code.to_vec()));
                }
                Column::ContractsState => {
                    let contract_state_key_hash = key;
                    let cskw = self.contract_state_key_wrapper(&contract_state_key_hash);
                    let data = self.contract_state_data(&cskw);

                    return Ok(Some(data.0));
                }
                Column::ContractsLatestUtxo => {
                    let contract_id = key;
                    let preimage = self.contracts_latest_utxo(&contract_id);

                    return Ok(Some(preimage.to_vec()));
                }
                Column::ContractsAssets => {
                    let cakw = self.contract_asset_key_wrapper(&key);
                    let cav = self.contract_asset_value(&cakw);

                    return Ok(Some(cav.to_bytes()));
                }
                Column::Coins => {
                    let utxo_id = self.utxo_id_wrapper(&key);
                    let owner_with_balance = self.owner_balance_wrapper(&utxo_id);
                    let (account, _is_cold) = self.am.account(owner_with_balance.owner().clone());
                    let mut fuel_address_wrapper: FuelAddressWrapper = (&account.address).into();
                    let mut compressed_coin = CompressedCoin::V1(CompressedCoinV1 {
                        owner: fuel_address_wrapper.get(),
                        amount: account.balance.as_limbs()[0],
                        asset_id: AssetId::BASE,
                        tx_pointer: Default::default(),
                    });

                    // let res = CompressedCoin::serialize(&compressed_coin);
                    return Ok(None);
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
                    panic!("not supported column: {:?}", &column)
                }
            }
            Ok(None)
        } else {
            Ok(None)
        }
    }
}
