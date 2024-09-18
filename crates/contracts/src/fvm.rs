use alloc::{format, vec::Vec};
use alloy_sol_types::{SolType, SolValue};
use core::str::FromStr;
use fluentbase_core::fvm::{
    exec::_exec_fuel_tx,
    helpers::FUEL_TESTNET_BASE_ASSET_ID,
    types::{
        FvmDepositInput,
        FvmWithdrawInput,
        WasmStorage,
        FVM_DEPOSIT_SIG_BYTES,
        FVM_WITHDRAW_SIG_BYTES,
    },
};
use fluentbase_sdk::{basic_entrypoint, derive::Contract, Bytes34, ExitCode, SharedAPI, U256};
use fuel_core_storage::{
    codec::Encode,
    structured_storage::StructuredStorage,
    tables::Coins,
    StorageInspect,
    StorageMutate,
};
use fuel_core_types::{
    entities::coins::coin::{CompressedCoin, CompressedCoinV1},
    fuel_types::AssetId,
};
use fuel_tx::{TxId, UtxoId};

#[derive(Contract)]
pub struct FvmLoaderEntrypoint<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> FvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {
        self.sdk.exit(ExitCode::Ok.into_i32());
    }

    pub fn main(&mut self) {
        let exit_code = self.main_inner();
        self.sdk.exit(exit_code.into_i32());
    }

    pub fn main_inner(&mut self) -> ExitCode {
        let base_asset_id: AssetId = AssetId::from_str(FUEL_TESTNET_BASE_ASSET_ID).unwrap();
        let raw_tx_bytes = self.sdk.input();
        let raw_tx_bytes_as_ref = raw_tx_bytes.as_ref();
        if raw_tx_bytes_as_ref.starts_with(FVM_DEPOSIT_SIG_BYTES.as_slice()) {
            let deposit_input: FvmDepositInput =
                <FvmDepositInput as SolType>::abi_decode(&raw_tx_bytes_as_ref[4..], true)
                    .expect("valid fvm deposit input");
            let owner_address = fuel_core_types::fuel_types::Address::new(deposit_input.address.0);

            let contract_ctx = self.sdk.contract_context();
            let caller = contract_ctx.caller;
            let value = contract_ctx.value;

            let evm_balance = self.sdk.balance(&caller);
            if evm_balance < value {
                return ExitCode::InsufficientBalance;
            }
            if value == U256::default() {
                panic!("value must be greater 0 and is used as a deposit amount");
            }
            let value_gwei = value / U256::from(1_000_000_000);
            if value != value_gwei * U256::from(1_000_000_000) {
                panic!("can not convert deposit value into gwei without cutting least significant part");
            };

            let mut wasm_storage = WasmStorage { sdk: &mut self.sdk };
            let deposit_withdraw_tx_index =
                wasm_storage.deposit_withdraw_tx_next_index().to_be_bytes();
            let mut storage = StructuredStorage::new(wasm_storage);
            let coin_amount = value_gwei.as_limbs()[0];

            let tx_id: TxId = TxId::new(deposit_withdraw_tx_index);
            let utxo_id = UtxoId::new(tx_id, 0);

            let mut coin = CompressedCoin::V1(CompressedCoinV1::default());
            coin.set_owner(owner_address);
            coin.set_amount(coin_amount);
            coin.set_asset_id(base_asset_id);

            <StructuredStorage<WasmStorage<'_, SDK>> as StorageMutate<Coins>>::insert(
                &mut storage,
                &utxo_id,
                &coin,
            )
            .expect("failed to save deposit utxo");

            let utxo_as_a_key: Bytes34 =
                fuel_core_storage::codec::primitive::Primitive::<34>::encode(&utxo_id);
            storage
                .into_inner()
                .utxo_owner_update(&utxo_as_a_key, caller)
                .expect("failed to update utxo<->owner mapping");

            return ExitCode::Ok;
        } else if raw_tx_bytes_as_ref.starts_with(FVM_WITHDRAW_SIG_BYTES.as_slice()) {
            let contract_ctx = self.sdk.contract_context();
            let caller = contract_ctx.caller;
            let utxo_ids: FvmWithdrawInput =
                <FvmWithdrawInput as SolType>::abi_decode(&raw_tx_bytes_as_ref[4..], true)
                    .expect("valid fvm withdraw input");
            let FvmWithdrawInput {
                utxos,
                withdraw_amount,
            } = utxo_ids;
            let mut utxos_total_balance = 0;
            let withdraw_amount = withdraw_amount.as_limbs()[0];
            let utxos: Vec<UtxoId> = utxos
                .iter()
                .map(|v| {
                    UtxoId::new(
                        TxId::new(v.tx_id.0),
                        v.output_index.as_limbs()[0]
                            .try_into()
                            .expect("output index is a valid u16 number"),
                    )
                })
                .collect();
            if utxos.len() <= 0 {
                panic!("provide utxos when withdrawing funds")
            }
            let mut last_owner: Option<fuel_core_types::fuel_types::Address> = None;
            for utxo_id in &utxos {
                let utxo_as_a_key =
                    fuel_core_storage::codec::primitive::Primitive::<34>::encode(&utxo_id);
                let wasm_storage = WasmStorage { sdk: &mut self.sdk };
                let evm_owner = wasm_storage.utxo_owner(&utxo_as_a_key);
                let mut storage = StructuredStorage::new(wasm_storage);
                let coin = <StructuredStorage<WasmStorage<'_, SDK>> as StorageInspect<Coins>>::get(
                    &mut storage,
                    &utxo_id,
                )
                .expect(&format!("got error when fetching utxo: {}", &utxo_id))
                .expect(&format!("utxo {} doesnt exist", &utxo_id));
                utxos_total_balance += coin.amount();
                if coin.asset_id() != &base_asset_id {
                    panic!(
                        "utxo {} asset id doesn't match base asset id {}",
                        &utxo_id, &base_asset_id
                    )
                }
                // validate belong to the user
                if evm_owner != caller {
                    panic!("caller address doesnt match utxo owner")
                }
                if let Some(last_owner) = last_owner {
                    if &last_owner != coin.owner() {
                        panic!("all utxo owners must be the same")
                    }
                }
                last_owner = Some(coin.owner().clone());
            }
            // sum all the utxos balances and check if it is more than provided in input
            if utxos_total_balance < withdraw_amount {
                panic!(
                    "utxo balance ({}) must be greater withdraw amount ({})",
                    &utxos_total_balance, &withdraw_amount
                )
            }

            let mut wasm_storage = WasmStorage { sdk: &mut self.sdk };
            let deposit_withdraw_tx_index =
                wasm_storage.deposit_withdraw_tx_next_index().to_be_bytes();
            let mut storage = StructuredStorage::new(wasm_storage);

            // spend utxos (just delete them)
            for utxo in &utxos {
                <StructuredStorage<WasmStorage<'_, SDK>> as StorageMutate<Coins>>::remove(
                    &mut storage,
                    &utxo,
                )
                .expect(&format!("failed to remove spent utxo: {}", utxo));
            }
            let balance_left = utxos_total_balance - withdraw_amount;
            if balance_left > 0 {
                // if there is fvm balance left - create utxo based on balance
                let mut coin = CompressedCoin::V1(CompressedCoinV1::default());
                coin.set_owner(last_owner.expect("utxo owner not found"));
                coin.set_amount(balance_left);
                coin.set_asset_id(base_asset_id);
                // TODO need counter to form TxId dynamically and without collisions
                let tx_id = TxId::new(deposit_withdraw_tx_index);
                let output_index: u16 = 0;
                let utxo_id = UtxoId::new(tx_id, output_index);
                <StructuredStorage<WasmStorage<'_, SDK>> as StorageMutate<Coins>>::insert(
                    &mut storage,
                    &utxo_id,
                    &coin,
                )
                .expect("insert first utxo success");
            }

            // top up evm balance
            self.sdk.call(
                caller,
                U256::from(withdraw_amount * 1e9 as u64),
                &[],
                10_000,
            );

            return ExitCode::Ok;
        }
        let result = _exec_fuel_tx(&mut self.sdk, u64::MAX, raw_tx_bytes);
        result.exit_code.into()
    }
}

basic_entrypoint!(FvmLoaderEntrypoint);
