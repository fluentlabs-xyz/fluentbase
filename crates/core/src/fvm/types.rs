use crate::fvm::helpers::{
    CoinsHelper,
    ContractsAssetsHelper,
    ContractsLatestUtxoHelper,
    ContractsRawCodeHelper,
    ContractsStateHelper,
    DepositWithdrawalIndexHelper,
    FixedChunksWriter,
    StorageChunksWriter,
    VariableLengthDataWriter,
};
use alloc::{vec, vec::Vec};
use alloy_sol_types::{sol, SolValue};
use fluentbase_sdk::{
    derive::derive_keccak256_id,
    Address,
    Bytes,
    Bytes32,
    Bytes34,
    Bytes64,
    SharedAPI,
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
    fuel_tx::ContractId,
    fuel_types::canonical::Deserialize,
    services::relayer::Event,
};
use revm_primitives::{
    address,
    alloy_primitives::private::serde::de::IntoDeserializer,
    bitvec::macros::internal::funty::Fundamental,
};

pub const FVM_DEPOSIT_SIG: u32 = derive_keccak256_id!("_fvm_deposit(uint64)");
pub const FVM_WITHDRAW_SIG: u32 = derive_keccak256_id!("_fvm_withdraw(uint64)");

pub const FVM_DEPOSIT_SIG_BYTES: [u8; 4] = FVM_DEPOSIT_SIG.to_be_bytes();
pub const FVM_WITHDRAW_SIG_BYTES: [u8; 4] = FVM_WITHDRAW_SIG.to_be_bytes();

sol! {
    #[derive(PartialEq, Eq, Debug)]
    struct FvmDepositInput {
        bytes32 address;
    }
}
sol! {
    #[derive(PartialEq, Eq, Debug)]
    struct UtxoIdSol {
        bytes32 tx_id;
        uint256 output_index;
    }
}
sol! {
    #[derive(PartialEq, Eq, Debug)]
    struct FvmWithdrawInput {
        UtxoIdSol[] utxos;
        uint256 withdraw_amount;
    }
}
#[test]
fn utx_ids_sol_encode_decode() {
    let utxo_id_1 = UtxoIdSol {
        tx_id: [1u8; 32].into(),
        output_index: U256::from(1),
    };
    let utxo_id_2 = UtxoIdSol {
        tx_id: [2u8; 32].into(),
        output_index: U256::from(2),
    };
    let utxo_id_3 = UtxoIdSol {
        tx_id: [3u8; 32].into(),
        output_index: U256::from(3),
    };
    let utxo_id_encoded = utxo_id_1.abi_encode();
    let utxo_id_decoded = UtxoIdSol::abi_decode(&utxo_id_encoded, true).unwrap();
    assert_eq!(utxo_id_1, utxo_id_decoded);

    let utxo_ids = FvmWithdrawInput {
        utxos: vec![utxo_id_1, utxo_id_2, utxo_id_3],
        withdraw_amount: U256::from(10),
    };
    let utxo_ids_encoded = utxo_ids.abi_encode();
    let utxo_ids_decoded = FvmWithdrawInput::abi_decode(&utxo_ids_encoded, true).unwrap();
    assert_eq!(utxo_ids, utxo_ids_decoded);
}

pub struct WasmRelayer;

impl RelayerPort for WasmRelayer {
    fn enabled(&self) -> bool {
        false
    }

    fn get_events(&self, _: &DaBlockHeight) -> anyhow::Result<Vec<Event>> {
        Ok(vec![])
    }
}

pub const FUEL_BASE_STORAGE_ADDRESS: Address = address!("ba8ab429ff0aaa5f1bb8f19f1f9974ffc82ff161");
pub const FUEL_UTXO_OWNER_STORAGE_ADDRESS: Address =
    address!("be0550e039dcbe443c8a918ac4ae5f632ea43127");

pub const STORAGE_ADDRESSES: [Address; 2] =
    [FUEL_BASE_STORAGE_ADDRESS, FUEL_UTXO_OWNER_STORAGE_ADDRESS];

const CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN: usize = 44;
const COINS_MAX_ENCODED_LEN: usize = 83;
const CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN: usize = 66;
const CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN: usize = 33;
const CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN: usize = 66;
const CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN: usize = 33;

pub struct WasmStorage<'a, SDK: SharedAPI> {
    pub sdk: &'a mut SDK,
}

impl<'a, SDK: SharedAPI> WasmStorage<'a, SDK> {
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

    pub fn contracts_raw_code_update(&mut self, raw_key: &Bytes32, data: &[u8]) {
        let helper = ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        let _ = storage_chunks.write_data(self.sdk, data);
    }

    pub fn contracts_raw_code(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsRawCodeHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        let mut buf = Vec::new();
        storage_chunks
            .read_data(self.sdk, &mut buf)
            .expect("raw code extracted");
        if buf.len() <= 0 {
            return None;
        }
        Some(buf.into())
    }

    pub fn contracts_latest_utxo_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN,
            anyhow::Error::msg("contracts latest utxo encoded len must fit max len")
        );
        let helper = ContractsLatestUtxoHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        storage_chunks.write_data_in_padded_chunks(
            self.sdk,
            data,
            (CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN / 32) as u32,
            true,
        );
        Ok(())
    }

    pub fn contracts_latest_utxo(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsLatestUtxoHelper::new(ContractId::from_bytes_ref(raw_key));
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const CAPACITY: usize = ((CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_LATEST_UTXO_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub fn contracts_state_data_update(&mut self, raw_key: &Bytes64, value: Bytes32) {
        let slot: U256 = ContractsStateHelper::new(raw_key)
            .value_storage_slot()
            .into();
        self.sdk.write_storage(slot, U256::from_be_bytes(value));
    }

    pub fn contracts_state_data(&self, raw_key: &Bytes64) -> Option<Bytes> {
        let slot: U256 = ContractsStateHelper::new(raw_key)
            .value_storage_slot()
            .into();
        let v = self.sdk.storage(&slot);
        if v == U256::ZERO {
            return None;
        }
        Some(v.to_be_bytes_vec().into())
    }

    pub fn contracts_assets_value_update(&mut self, raw_key: &Bytes64, data: &[u8]) {
        let slot = ContractsAssetsHelper::new(raw_key).value_storage_slot();
        let value =
            ContractsAssetsHelper::value_to_u256(data.try_into().expect("valid encoded value"));
        self.sdk.write_storage(slot, value);
    }

    pub fn contracts_assets_value(&self, raw_key: &Bytes64) -> Option<Bytes> {
        let slot = ContractsAssetsHelper::new(raw_key).value_storage_slot();
        let val = self.sdk.storage(&slot);
        if val == U256::ZERO {
            return None;
        }
        Some(Bytes::copy_from_slice(
            ContractsAssetsHelper::u256_to_value(&val).as_slice(),
        ))
    }

    pub fn contracts_state_merkle_data_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("contract state merkle data encoded len must fit max len")
        );
        let helper = ContractsStateHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub fn contracts_state_merkle_data(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsStateHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const CAPACITY: usize = ((CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub fn contracts_state_merkle_metadata_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("contracts state merkle metadata encoded len must fit max len")
        );
        let helper = ContractsStateHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub fn contracts_state_merkle_metadata(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsStateHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const CAPACITY: usize =
            ((CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_STATE_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub fn contracts_assets_merkle_data_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("contracts assets merkle data encoded len must fit max len")
        );
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub fn contracts_assets_merkle_data(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const CAPACITY: usize = ((CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_DATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub fn contracts_assets_merkle_metadata_update(
        &mut self,
        raw_key: &Bytes32,
        data: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN,
            anyhow::Error::msg("contracts assets merkle metadata encoded len must fit max len")
        );
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.write_data_in_padded_chunks(self.sdk, data, MAX_CHUNK_INDEX, true);
        Ok(())
    }

    pub fn contracts_assets_merkle_metadata(&self, raw_key: &Bytes32) -> Option<Bytes> {
        let helper = ContractsAssetsHelper::new_transformed(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const CAPACITY: usize =
            ((CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (CONTRACTS_ASSETS_MERKLE_METADATA_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    // pub fn coins_update(&mut self, raw_key: &Bytes34, v: &CoinsHolderHelper) {
    //     let (address, asset_id, balance) = v.to_u256_tuple();
    //     let ch = CoinsHelper::new(raw_key);
    //     self.sdk.write_storage(ch.owner_storage_slot(), address);
    //     self.sdk.write_storage(ch.asset_id_storage_slot(), asset_id);
    //     self.sdk.write_storage(ch.balance_storage_slot(), balance);
    // }
    //
    // pub fn coins(&self, raw_key: &Bytes34) -> Option<CoinsHolderHelper> {
    //     let ch = CoinsHelper::new(raw_key);
    //     let owner = self.sdk.storage(&ch.owner_storage_slot());
    //     if owner == U256::ZERO {
    //         return None;
    //     }
    //     let asset_id = self.sdk.storage(&ch.asset_id_storage_slot());
    //     let balance = self.sdk.storage(&ch.balance_storage_slot());
    //     Some(CoinsHolderHelper::from_u256_tuple(
    //         &owner, &asset_id, &balance,
    //     ))
    // }

    pub fn coins_update(&mut self, raw_key: &Bytes34, data: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= COINS_MAX_ENCODED_LEN,
            anyhow::Error::msg("coins encoded len must fit max len")
        );
        let helper = CoinsHelper::new(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        storage_chunks.write_data_in_padded_chunks(
            self.sdk,
            data,
            (COINS_MAX_ENCODED_LEN / 32) as u32,
            true,
        );
        Ok(())
    }

    pub fn coins(&self, raw_key: &Bytes34) -> Option<Bytes> {
        let helper = CoinsHelper::new(raw_key);
        let mut storage_chunks = StorageChunksWriter {
            address: &FUEL_BASE_STORAGE_ADDRESS,
            slot_calc: &helper,
            _phantom: Default::default(),
        };
        const CAPACITY: usize = ((COINS_MAX_ENCODED_LEN - 1) / 32 + 1) * 32;
        let mut res = Vec::with_capacity(CAPACITY);
        const MAX_CHUNK_INDEX: u32 = (COINS_MAX_ENCODED_LEN / 32) as u32;
        storage_chunks.read_data_in_padded_chunks(self.sdk, MAX_CHUNK_INDEX, &mut res);
        if res.iter().all(|&v| v == 0) {
            return None;
        }
        Some(res.into())
    }

    pub fn utxo_owner_update(&mut self, raw_key: &Bytes34, data: Address) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() <= COINS_MAX_ENCODED_LEN,
            anyhow::Error::msg("coins encoded len must fit max len")
        );
        let helper = CoinsHelper::new(raw_key);
        let slot = helper.owner_storage_slot();
        let address_u256 = U256::from_be_slice(&data.0.as_slice());
        self.sdk.write_storage(slot, address_u256);
        Ok(())
    }

    pub fn utxo_owner(&self, raw_key: &Bytes34) -> Address {
        let helper = CoinsHelper::new(raw_key);
        let slot = helper.owner_storage_slot();
        let address_u256 = self.sdk.storage(&slot);
        Address::from_slice(&address_u256.to_be_bytes::<32>()[12..])
    }

    pub fn deposit_withdraw_tx_next_index(&mut self) -> U256 {
        DepositWithdrawalIndexHelper::new(self.sdk).next_index()
    }
}

impl<'a, SDK: SharedAPI> KeyValueInspect for WasmStorage<'a, SDK> {
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

                let contracts_assets_key: Bytes34 = key.try_into().expect("34 bytes key");
                let value_data = self.coins(&contracts_assets_key);

                Ok(value_data.map(|v| v.to_vec()))

                // let utxo_id_key: Bytes34 = key.try_into().expect("34 bytes key");
                // let Some(coins_holder_helper) = self.coins(&utxo_id_key) else {
                //     return Ok(None);
                // };
                // let fuel_address = FuelAddress::new(*coins_holder_helper.address());
                // let (account, _is_cold) = self.sdk.account(&fuel_address.fluent_address());
                // let amount = account.balance / U256::from(1_000_000_000);
                // let compressed_coin = CompressedCoin::V1(CompressedCoinV1 {
                //     owner: *coins_holder_helper.address(),
                //     amount: coins_holder_helper.balance(),
                //     asset_id: *coins_holder_helper.asset_id(),
                //     tx_pointer: Default::default(),
                // });
                // let r =
                //     postcard::to_allocvec(&compressed_coin).expect("compressed coin serialized");
                // Ok(Some(r))
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

impl<'a, SDK: SharedAPI> KeyValueMutate for WasmStorage<'a, SDK> {
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

                let key: Bytes34 = key.try_into().expect("34 bytes key");
                self.coins_update(&key, buf).map_err(|_| {
                    fuel_core_storage::Error::Other(anyhow::Error::msg("failed to update coins"))
                })?;

                // if buf.len() <= 0 {
                //     deletion process
                //     let old_value = KeyValueInspect::get(&self, key, column)?;
                //     if let Some(old_value) = old_value {
                //         let compressed_coin: CompressedCoin =
                //             postcard::from_bytes(old_value.as_slice())
                //                 .expect("compressed coin");
                //         let fuel_address = FuelAddress::new(*compressed_coin.owner());
                //         let (mut account, _) = self.sdk.account(&fuel_address.fluent_address());
                //         account.balance -= U256::from(1_000_000_000)
                //             * U256::from(compressed_coin.amount().as_u64());
                //         self.sdk.write_account(account, AccountStatus::Modified);
                //     }
                //     delete existing mapping
                //     let coins_owner_with_balance = CoinsHolderHelper::default();
                //     self.coins_update(&utxo_id_key, &coins_owner_with_balance);
                //     self.coins_update(&utxo_id_key, &[]);
                //
                //     return Ok(0);
                // }

                // let compressed_coin: CompressedCoin =
                //     postcard::from_bytes(buf).expect("compressed coin");
                // let coins = CoinsHolderHelper::from(
                //     compressed_coin.owner(),
                //     *compressed_coin.asset_id(),
                //     *compressed_coin.amount(),
                // );
                // self.coins_update(&utxo_id_key, &coins);
                //
                // let fuel_address = FuelAddress::new(*coins.address());
                // let (mut account, _) = self.sdk.account(&fuel_address.fluent_address());
                // let coin_amount = U256::from(1_000_000_000) * U256::from(coins.balance());
                // account.balance += coin_amount;
                // self.sdk.write_account(account, AccountStatus::Modified);
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

impl<'a, SDK: SharedAPI> Modifiable for WasmStorage<'a, SDK> {
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

// fn utxo_id_to_bytes34(utxo_id: &UtxoId) -> Bytes34 {
//     let mut res: Bytes34 = [0u8; 34];
//     res.as_mut_slice()[..32].copy_from_slice(utxo_id.tx_id().as_slice());
//     res.as_mut_slice()[32..].copy_from_slice(&utxo_id.output_index().to_le_bytes());
//
//     res
// }
//
// fn utxo_id_from_bytes34(utxo_id_bytes: &Bytes34) -> UtxoId {
//     let utxo_id_slice = utxo_id_bytes.as_slice();
//     let mut utxo_id = UtxoId::new(
//         TxId::from_bytes(&utxo_id_slice[..32]).expect("failed to extract tx_id from utxo slice"),
//         u16::from_le_bytes(
//             utxo_id_slice[32..]
//                 .try_into()
//                 .expect("failed to extract output index utxo slice"),
//         ),
//     );
//     utxo_id
// }

#[cfg(test)]
mod tests {
    use fuel_core::txpool::types::TxId;
    use fuel_core_types::{
        entities::{
            coins::coin::{CompressedCoin, CompressedCoinV1},
            contract::{ContractUtxoInfo, ContractUtxoInfoV1},
        },
        fuel_tx::{Address, AssetId, TxPointer, UtxoId, Word},
        fuel_types::BlockHeight,
    };

    #[test]
    fn max_sizes_encoded() {
        const ASSET_ID_MAX: AssetId = AssetId::new([0xff; 32]);
        const UTXO_ID_MAX: UtxoId = UtxoId::new(TxId::new([0xffu8; 32]), u16::MAX);
        const TX_POINTER_MAX: TxPointer = TxPointer::new(BlockHeight::new(u32::MAX), u16::MAX);
        const ADDRESS_MAX: Address = Address::new([0xff; 32]);

        let v = ContractUtxoInfo::V1(ContractUtxoInfoV1 {
            utxo_id: UTXO_ID_MAX,
            tx_pointer: TX_POINTER_MAX,
        });
        let res = postcard::to_allocvec(&v).unwrap();
        assert_eq!(44, res.len());

        let v = CompressedCoin::V1(CompressedCoinV1 {
            owner: ADDRESS_MAX,
            amount: Word::MAX,
            asset_id: ASSET_ID_MAX,
            tx_pointer: TX_POINTER_MAX,
        });
        let res = postcard::to_allocvec(&v).unwrap();
        assert_eq!(83, res.len());
    }
}
