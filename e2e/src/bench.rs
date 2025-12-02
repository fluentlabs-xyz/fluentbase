use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{constructor::encode_constructor_params, hex, Address, Bytes};
use fluentbase_testing::EvmTestingContext;

#[test]
fn test_bench_erc20_transfer() {
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

    for _ in 0..1_000 {
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
    }
}
