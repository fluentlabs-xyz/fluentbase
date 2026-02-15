use criterion::{criterion_main, Criterion};
use fluentbase_e2e::EvmTestingContextWithGenesis;
use fluentbase_sdk::{
    constructor::encode_constructor_params, Address, Bytes, ContractContextV1,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, U256,
};
use fluentbase_testing::EvmTestingContext;
use fluentbase_universal_token::{
    command::{TransferCommand, UniversalTokenCommand},
    storage::{InitialSettings, DECIMALS_DEFAULT},
};
use hex_literal::hex;
use std::time::Duration;

fn tokens_transfer_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("Tokens Transfer Comparison");

    // --- Benchmark 1: Original EVM ERC20 (rWasm disabled) ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        ctx.disabled_rwasm = true;
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            hex::decode(include_bytes!("../assets/ERC20.bin"))
                .unwrap()
                .into(),
        );
        let transfer_payload: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();

        group.bench_function("1_Original_EVM_ERC20", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 2: Emulated EVM ERC20 (rWasm enabled) ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            hex::decode(include_bytes!("../assets/ERC20.bin"))
                .unwrap()
                .into(),
        );
        let transfer_payload: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();

        group.bench_function("2_Emulated_EVM_ERC20", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 3: rWasm Contract ERC20 ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let bytecode: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_ERC20
            .wasm_bytecode
            .into();

        // constructor params for ERC20:
        //     name: "TestToken"
        //     symbol: "TST"
        //     initial_supply: 1_000_000
        // use examples/erc20/src/lib.rs print_constructor_params_hex() to regenerate
        let constructor_params = hex!("000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000000000954657374546f6b656e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000035453540000000000000000000000000000000000000000000000000000000000");

        let encoded_constructor_params = encode_constructor_params(&constructor_params);
        let mut input: Vec<u8> = Vec::new();
        input.extend(bytecode);
        input.extend(encoded_constructor_params);

        let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, input.into());

        let transfer_payload: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();

        group.bench_function("3_rWasm_Contract_ERC20", |b| {
            b.iter(|| {
                let result = ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
                if !result.is_success() {
                    println!("{:?}", result);
                }
                assert!(result.is_success())
            });
        });
    }

    // --- Benchmark 4: Precompiled Universal Token ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const DEPLOYER_ADDR: Address = Address::repeat_byte(1);
        const USER_ADDR: Address = Address::repeat_byte(2);
        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
            ..Default::default()
        });
        let total_supply = U256::from(0xffff_ffffu64);
        let initial_settings = InitialSettings {
            token_name: Default::default(),
            token_symbol: Default::default(),
            decimals: DECIMALS_DEFAULT,
            initial_supply: total_supply,
            minter: Address::ZERO,
            pauser: Address::ZERO,
        };
        let contract_address =
            ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings.encode_with_prefix());

        let mut input = Vec::<u8>::new();
        TransferCommand {
            to: USER_ADDR,
            amount: U256::from(1),
        }
        .encode_for_send(&mut input);
        let transfer_payload: Bytes = input.into();
        group.bench_function("4_Precompiled_Universal_token", |b| {
            // Note: Manual warmup calls are not needed. Criterion handles warmups automatically.
            b.iter(|| {
                let result = ctx.call_evm_tx(
                    DEPLOYER_ADDR,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
                assert!(result.is_success())
            });
        });
    }

    group.finish();
}

// criterion_group!(benches, erc20_transfer_benches);
pub fn benches() {
    let mut criterion: Criterion<_> = Criterion::default()
        .configure_from_args()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(1))
        .sample_size(1000);
    tokens_transfer_benches(&mut criterion);
}
criterion_main!(benches);
