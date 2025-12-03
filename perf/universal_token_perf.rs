#![no_main]

use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_sdk::{
    Address, Bytes, ContractContextV1, GenesisContract, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
    PRECOMPILE_WASM_RUNTIME, U256,
};
use fluentbase_testing::EvmTestingContext;
use fluentbase_universal_token::common::fixed_bytes_from_u256;
use fluentbase_universal_token::storage::{Feature, InitialSettings, DECIMALS_DEFAULT};
use fluentbase_universal_token::types::input_commands::{Encodable, TransferCommand};

pub trait EvmTestingContextWithGenesis {
    fn with_full_genesis(self) -> Self;

    fn with_minimal_genesis(self) -> Self;
}

impl EvmTestingContextWithGenesis for EvmTestingContext {
    fn with_full_genesis(self) -> EvmTestingContext {
        let contracts: Vec<GenesisContract> = GENESIS_CONTRACTS_BY_ADDRESS
            .iter()
            .map(|(_k, v)| v.clone())
            .collect();
        self.with_contracts(&contracts)
    }

    fn with_minimal_genesis(self) -> EvmTestingContext {
        let wasm_runtime = GENESIS_CONTRACTS_BY_ADDRESS
            .get(&PRECOMPILE_WASM_RUNTIME)
            .unwrap()
            .clone();
        self.with_contracts(&[wasm_runtime])
    }
}

#[no_mangle]
pub fn main() {
    // --- Benchmark 4: Precompiled Universal Token ---
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER_ADDR: Address = Address::repeat_byte(1);
    const USER_ADDR: Address = Address::repeat_byte(2);
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    let mut initial_settings = InitialSettings::new();
    let total_supply = U256::from(0xffff_ffffu64);
    initial_settings.add_feature(Feature::InitialSupply {
        amount: fixed_bytes_from_u256(&total_supply),
        owner: DEPLOYER_ADDR.into(),
        decimals: DECIMALS_DEFAULT,
    });
    let contract_address =
        ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings.encode_for_deploy().into());

    let mut input = Vec::<u8>::new();
    TransferCommand {
        to: USER_ADDR,
        amount: U256::from(1),
    }
    .encode_for_send(&mut input);
    let transfer_payload: Bytes = input.into();
    #[inline(never)]
    fn bench_iter(
        ctx: &mut EvmTestingContext,
        contract_address: &Address,
        transfer_payload: &Bytes,
    ) {
        let result = ctx.call_evm_tx(
            DEPLOYER_ADDR,
            *contract_address,
            transfer_payload.clone(),
            None,
            None,
        );
        assert!(result.is_success());
        core::hint::black_box(result.clone());
    }
    for _ in 0..1000 {
        bench_iter(&mut ctx, &contract_address, &transfer_payload);
    }
}
