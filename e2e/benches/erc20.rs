use criterion::{criterion_main, Criterion};
use fluentbase_e2e::EvmTestingContextWithGenesis;
use fluentbase_sdk::{address, constructor::encode_constructor_params, Address, Bytes};
use fluentbase_svm::{
    error::SvmError,
    helpers::serialize_svm_program_params_from_instruction,
    solana_program::instruction::Instruction,
    token_2022,
    token_2022::instruction::{initialize_account, initialize_mint, mint_to},
};
use fluentbase_svm_common::common::pubkey_from_evm_address;
use fluentbase_testing::{try_print_utf8_error, EvmTestingContext};
use fluentbase_types::{
    ContractContextV1, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, UNIVERSAL_TOKEN_MAGIC_BYTES,
};
use fluentbase_universal_token::{common::sig_to_bytes, consts::SIG_TOKEN2022};
use hex_literal::hex;
use revm::context::result::ExecutionResult;
use std::time::Duration;

fn erc20_transfer_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("ERC20 Transfer Comparison");

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
        const USER_ADDRESS1: Address = address!("1111111111111111111111111111111111111111");
        const USER_ADDRESS2: Address = address!("2222222222222222222222222222222222222222");
        const USER_ADDRESS5: Address = address!("5555555555555555555555555555555555555555");
        const USER_ADDRESS6: Address = address!("6666666666666666666666666666666666666666");
        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
            ..Default::default()
        });
        ctx.sdk
            .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

        pub fn build_input(prefix: &[u8], instruction: &Instruction) -> Result<Vec<u8>, SvmError> {
            let mut input: Vec<u8> = prefix.to_vec();
            serialize_svm_program_params_from_instruction(&mut input, instruction)
                .expect("failed to serialize program params into init_bytecode");
            Ok(input)
        }

        pub fn call_with_sig(
            ctx: &mut EvmTestingContext,
            input: Bytes,
            caller: &Address,
            callee: &Address,
        ) -> Result<Vec<u8>, u32> {
            let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
            match &result {
                ExecutionResult::Revert {
                    gas_used: _,
                    output,
                } => {
                    let output_vec = output.to_vec();
                    try_print_utf8_error(&output_vec);
                    let error_code = u32::from_be_bytes(output_vec[32..].try_into().unwrap());
                    Err(error_code)
                }
                ExecutionResult::Success { output, .. } => Ok(output.data().to_vec()),
                _ => {
                    panic!("expected revert, got: {:?}", &result)
                }
            }
        }

        let program_id = token_2022::lib::id();
        let account1_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
        let account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
        let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
        let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);

        let initialize_mint_instruction =
            initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap();

        let init_bytecode = build_input(&UNIVERSAL_TOKEN_MAGIC_BYTES, &initialize_mint_instruction)
            .expect("failed to build input");
        let contract_address = ctx.deploy_evm_tx(USER_ADDRESS5, init_bytecode.clone().into());

        ctx.commit_db_to_sdk();

        let initialize_account1_instruction =
            initialize_account(&program_id, &account1_key, &mint_key, &account1_key).unwrap();
        let input = build_input(
            &sig_to_bytes(SIG_TOKEN2022),
            &initialize_account1_instruction,
        )
        .expect("failed to build input");
        let _output_data =
            call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

        let initialize_account2_instruction =
            initialize_account(&program_id, &account2_key, &mint_key, &owner_key).unwrap();
        let input = build_input(
            &sig_to_bytes(SIG_TOKEN2022),
            &initialize_account2_instruction,
        )
        .expect("failed to build input");
        let _output_data =
            call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

        // mint to account
        let mint_to_instruction = mint_to(
            &program_id,
            &mint_key,
            &account1_key,
            &owner_key,
            &[],
            u64::MAX,
        )
        .unwrap();
        let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &mint_to_instruction)
            .expect("failed to build input");
        let _output_data =
            call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();

        // source-owner transfer
        #[allow(deprecated)]
        let transfer_instruction = token_2022::instruction::transfer(
            &program_id,
            &account1_key,
            &account2_key,
            &account1_key,
            &[],
            1,
        )
        .unwrap();
        let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &transfer_instruction)
            .expect("failed to build input");

        group.bench_function("4_Precompiled_UniversalToken", |b| {
            // Note: Manual warmup calls are not needed. Criterion handles warmups automatically.
            b.iter(|| {
                let _result = ctx.call_evm_tx(
                    USER_ADDRESS1,
                    contract_address,
                    input.clone().into(),
                    None,
                    None,
                );
            });
        });
    }

    group.finish();
}

// criterion_group!(benches, erc20_transfer_benches);
pub fn benches() {
    let mut criterion: Criterion<_> = Criterion::default()
        .configure_from_args()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(1))
        .sample_size(1000);
    erc20_transfer_benches(&mut criterion);
}
criterion_main!(benches);
