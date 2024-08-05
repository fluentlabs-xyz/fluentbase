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
use alloc::{borrow::Cow, format, vec, vec::Vec};
use core::{hash::Hash, ops::Deref};
use fluentbase_sdk::{
    AccountManager,
    Address,
    Bytes,
    ContextReader,
    GuestAccountManager,
    GuestContextReader,
    U256,
};
use fluentbase_types::{Bytes32, Bytes34, Bytes64};
// use fuel_core::state::generic_database::GenericDatabase;
use fuel_core_executor::ports::RelayerPort;
use fuel_core_storage::{
    self,
    codec::Encoder,
    column::Column,
    kv_store::KeyValueInspect,
    kv_store::KeyValueMutate,
    kv_store::Value,
    kv_store::WriteOperation,
    transactional::{Changes, Modifiable},
    Mappable,
    Result as StorageResult,
    // StorageInspect,
    // StorageMutate
};
use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    entities::coins::coin::{CompressedCoin, CompressedCoinV1},
    fuel_tx::{AssetId, ContractId},
    fuel_types::canonical::Serialize,
    services::relayer::Event,
};
use revm_primitives::{bitvec::macros::internal::funty::Fundamental, hex};

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

#[derive(Clone)]
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
    pub(crate) fn contracts_raw_code(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let preimage = self.am.preimage(
            &ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key)).value_preimage_key(),
        );
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_raw_code_update(&self, raw_key: &Bytes32, data: &[u8]) {
        let key =
            ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key)).value_preimage_key();
        self.am.update_preimage(&key, 0, data);
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

    pub(crate) fn contracts_state_data_update(&self, raw_key: &Bytes64, data: &[u8]) {
        let key = ContractsStateHelper::new(raw_key).value_preimage_key();
        self.am.update_preimage(&key, 0, data);
    }

    pub(crate) fn contracts_state_data(&self, raw_key: &Bytes64) -> Option<Bytes> {
        let key = ContractsStateHelper::new(raw_key).value_preimage_key();
        let preimage = self.am.preimage(&key);
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_state_merkle_data_update(&self, raw_key: &Bytes32, data: &[u8]) {
        let key = ContractsStateHelper::new_transformed(raw_key).merkle_data_preimage_key();
        self.am.update_preimage(&key, 0, data);
    }

    pub(crate) fn contracts_state_merkle_data(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let key = ContractsStateHelper::new_transformed(raw_key).merkle_data_preimage_key();
        let preimage = self.am.preimage(&key);
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_state_merkle_metadata_update(&self, raw_key: &Bytes32, data: &[u8]) {
        let key = ContractsStateHelper::new_transformed(raw_key).merkle_metadata_preimage_key();
        self.am.update_preimage(&key, 0, data);
    }

    pub(crate) fn contracts_state_merkle_metadata(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let key = ContractsStateHelper::new_transformed(raw_key).merkle_metadata_preimage_key();
        let preimage = self.am.preimage(&key);
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_assets_value_update(&self, raw_key: &Bytes64, value: &[u8]) {
        let slot = ContractsAssetsHelper::new(raw_key).value_storage_slot();
        let value =
            ContractsAssetsHelper::value_to_u256(value.try_into().expect("encoded value is valid"));
        self.am
            .write_storage(CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS, slot, value);
    }

    pub(crate) fn contracts_assets_value(&self, raw_key: &Bytes64) -> Option<Bytes> {
        let slot = ContractsAssetsHelper::new(raw_key).value_storage_slot();
        let (val, _is_cold) =
            self.am
                .storage(CONTRACTS_ASSETS_KEY_TO_VALUE_STORAGE_ADDRESS, slot, false);
        if val == U256::ZERO {
            return None;
        }
        Some(Bytes::copy_from_slice(
            ContractsAssetsHelper::u256_to_value(&val).as_slice(),
        ))
    }

    pub(crate) fn contracts_assets_merkle_data_update(&self, raw_key: &Bytes32, value: &[u8]) {
        let key = ContractsAssetsHelper::from_transformed(raw_key).merkle_data_preimage_key();
        self.am.update_preimage(&key, 0, value);
    }

    pub(crate) fn contracts_assets_merkle_data(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let key = ContractsAssetsHelper::from_transformed(raw_key).merkle_data_preimage_key();
        let preimage = self.am.preimage(&key);
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn contracts_assets_merkle_metadata_update(&self, raw_key: &Bytes32, value: &[u8]) {
        let key = ContractsAssetsHelper::from_transformed(raw_key).merkle_metadata_preimage_key();
        self.am.update_preimage(&key, 0, value);
    }

    pub(crate) fn contracts_assets_merkle_metadata(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let key = ContractsAssetsHelper::from_transformed(raw_key).merkle_metadata_preimage_key();
        let preimage = self.am.preimage(&key);
        if preimage.len() > 0 {
            return Some(preimage);
        }
        None
    }

    pub(crate) fn coins_owner_with_balance(
        &self,
        raw_key: &Bytes34,
    ) -> Option<CoinsOwnerWithBalanceHelper> {
        let (owner, _is_cold) = self.am.storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            CoinsHelper::new(raw_key).owner_storage_slot(),
            false,
        );
        if owner == U256::ZERO {
            return None;
        }
        let (balance, _is_cold) = self.am.storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            CoinsHelper::new(raw_key).balance_storage_slot(),
            false,
        );
        Some(CoinsOwnerWithBalanceHelper::from_u256_address_balance(&(
            owner, balance,
        )))
    }

    pub(crate) fn coins_owner_with_balance_update(
        &self,
        raw_key: &Bytes34,
        v: &CoinsOwnerWithBalanceHelper,
    ) {
        let (address, balance) = v.to_u256_address_balance();
        self.am.write_storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            CoinsHelper::new(raw_key).owner_storage_slot(),
            address,
        );
        self.am.write_storage(
            UTXO_UNIQ_ID_TO_OWNER_WITH_BALANCE_STORAGE_ADDRESS,
            CoinsHelper::new(raw_key).balance_storage_slot(),
            balance,
        );
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> KeyValueInspect for WasmStorage<'a, CR, AM> {
    type Column = Column;

    fn size_of_value(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<usize>> {
        self.get(key, column).map(|v1| v1.map(|v2| v2.len()))
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

                return Ok(value_data.map(|v| v.to_vec()));
            }
            Column::Coins => {
                // key -> UtxoId
                // value -> CompressedCoin

                let utxo_id_key: Bytes34 = key.try_into().expect("34 bytes key");
                let Some(owner_with_balance) = self.coins_owner_with_balance(&utxo_id_key) else {
                    return Ok(None);
                };
                let mut fuel_address = FuelAddress::new(*owner_with_balance.address());
                let (account, _is_cold) = self.am.account(fuel_address.fluent_address());
                let amount = account.balance / U256::from(1_000_000_000);
                let compressed_coin = CompressedCoin::V1(CompressedCoinV1 {
                    owner: fuel_address.get(),
                    amount: amount.as_limbs()[0], // gwei ?
                    asset_id: AssetId::BASE,
                    tx_pointer: Default::default(),
                });

                let r =
                    postcard::to_allocvec(&compressed_coin).expect("compressed coin serialized");
                return Ok(Some(r));
            }

            Column::ContractsStateMerkleData => {
                // key - 32 bytes
                // value - 66 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_state_merkle_data(&key);

                return Ok(data.map(|v| v.to_vec()));
            }
            Column::ContractsStateMerkleMetadata => {
                // key - 32 bytes
                // value - 33 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_state_merkle_metadata(&key);

                return Ok(data.map(|v| v.to_vec()));
            }

            Column::ContractsAssetsMerkleData => {
                // key - 32 bytes
                // value - 66 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_assets_merkle_data(&key);

                return Ok(data.map(|v| v.to_vec()));
            }
            Column::ContractsAssetsMerkleMetadata => {
                // key - 32 bytes
                // value - 33 bytes
                let key: Bytes32 = key.try_into().expect("32 bytes key");
                let data = self.contracts_assets_merkle_metadata(&key);

                return Ok(data.map(|v| v.to_vec()));
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
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
        Ok(None)
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

                if buf.len() <= 0 {
                    // get current if exists
                    let old_value = KeyValueInspect::get(&self, key, column)?;
                    if let Some(old_value) = old_value {
                        let compressed_coin: CompressedCoin =
                            postcard::from_bytes(old_value.as_slice())
                                .expect("compressed coin recovered");
                        // fetch old acc
                        let mut fuel_address = FuelAddress::new(*compressed_coin.owner());
                        let (mut account, _) = self.am.account(fuel_address.fluent_address());
                        // subtract balance
                        account.balance -= U256::from(1_000_000_000)
                            * U256::from(compressed_coin.amount().as_u64());
                        // write updated acc
                        self.am.write_account(&account);
                    }
                    // delete current mapping
                    let coins_owner_with_balance = CoinsOwnerWithBalanceHelper::default();
                    self.coins_owner_with_balance_update(&utxo_id_key, &coins_owner_with_balance);

                    return Ok(0);
                }

                let compressed_coin: CompressedCoin =
                    postcard::from_bytes(buf).expect("compressed coin");
                // assert_eq!(compressed_coin.asset_id(), &AssetId::BASE);
                let coins_owner_with_balance = CoinsOwnerWithBalanceHelper::from_owner(
                    compressed_coin.owner(),
                    *compressed_coin.amount(),
                );
                self.coins_owner_with_balance_update(&utxo_id_key, &coins_owner_with_balance);

                let mut fuel_address = FuelAddress::new(*coins_owner_with_balance.address());
                let (mut account, _) = self.am.account(fuel_address.fluent_address());
                let coin_amount =
                    U256::from(1_000_000_000) * U256::from(coins_owner_with_balance.balance());
                account.balance += coin_amount;
                self.am.write_account(&account);
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
                self.contracts_state_merkle_data_update(&key, buf);
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
                self.contracts_state_merkle_metadata_update(&key, buf);
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
                self.contracts_assets_merkle_data_update(&key, buf);
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
                self.contracts_assets_merkle_metadata_update(&key, buf);
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
            | Column::Messages
            | Column::ProcessedTransactions
            | Column::FuelBlockConsensus
            | Column::ConsensusParametersVersions
            | Column::StateTransitionBytecodeVersions
            | Column::UploadedBytecodes
            | Column::GenesisMetadata => {
                // panic!(
                //     "unsupported column referenced '{:?}' while writing data",
                //     &column
                // )
                return Ok(0);
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
            | Column::Coins
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata => {
                self.write(key, column, &[])?;
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
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

impl<'a, CR: ContextReader, AM: AccountManager> Modifiable for WasmStorage<'a, CR, AM> {
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

// impl<'a, CR: ContextReader, AM: AccountManager, Type: Mappable> StorageInspect<Type>
//     for WasmStorage<'a, CR, AM>
// {
//     type Error = ();
//
//     fn get(&self, key: &Type::Key) -> Result<Option<Cow<Type::OwnedValue>>, Self::Error> {
//         // TODO
//         Ok(None)
//     }
//
//     fn contains_key(&self, key: &Type::Key) -> Result<bool, Self::Error> {
//         // TODO
//         Ok(false)
//     }
// }
//
// impl<'a, CR: ContextReader, AM: AccountManager, Type: Mappable> StorageMutate<Type>
//     for WasmStorage<'a, CR, AM>
// {
//     fn replace(
//         &mut self,
//         key: &Type::Key,
//         value: &Type::Value,
//     ) -> Result<Option<Type::OwnedValue>, Self::Error> {
//         // TODO
//         Ok(None)
//     }
//
//     fn take(&mut self, key: &Type::Key) -> Result<Option<Type::OwnedValue>, Self::Error> {
//         // TODO
//         Ok(None)
//     }
// }
