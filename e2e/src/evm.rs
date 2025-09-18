use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use core::str::from_utf8;
use fluentbase_sdk::{address, bytes, calc_create_address, constructor, Address, U256};
use fluentbase_sdk_testing::{
    try_print_utf8_error,
    EvmTestingContext,
    HostTestingContextNativeAPI,
    TxBuilder,
};
use fluentbase_types::{PRECOMPILE_BLAKE2F, PRECOMPILE_SECP256K1_RECOVER};
use hex_literal::hex;
use revm::{
    bytecode::opcode,
    context::result::ExecutionResult::Revert,
    primitives::hardfork::SpecId,
};
use fluentbase_sdk::constructor::encode_constructor_params;

#[test]
fn test_evm_greeting() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let (contract_address, gas_used) = ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033").into());
    assert_eq!(gas_used, 365_537);
    // call greeting EVM contract
    println!("\n\n\n");
    ctx.add_balance(DEPLOYER_ADDRESS, U256::from(1e18));
    let result = TxBuilder::call(&mut ctx, DEPLOYER_ADDRESS, contract_address, None)
        .input(bytes!("45773e4e"))
        .exec();
    println!("{:?}", result);
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert!(!bytes.is_empty());
    let bytes = &bytes[64..75];
    assert_eq!("Hello World", from_utf8(bytes.as_ref()).unwrap());
    assert_eq!(result.gas_used(), 21_792);
}

#[test]
fn test_evm_storage() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let (contract_address_1, gas_used) = ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, hex!("608060405260645f81905550606460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550606460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055506103ed806100d95f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c806320965255146100385780635524107714610056575b5f80fd5b610040610072565b60405161004d91906102cd565b60405180910390f35b610070600480360381019061006b9190610314565b6101b5565b005b5f805460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146100f3576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100ea90610399565b60405180910390fd5b5f5460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146101ae576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a590610399565b60405180910390fd5b5f54905090565b805f819055508060015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508060025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055507f63a242a632efe33c0e210e04e4173612a17efa4f16aa4890bc7e46caece80de05f546040516102aa91906102cd565b60405180910390a150565b5f819050919050565b6102c7816102b5565b82525050565b5f6020820190506102e05f8301846102be565b92915050565b5f80fd5b6102f3816102b5565b81146102fd575f80fd5b50565b5f8135905061030e816102ea565b92915050565b5f60208284031215610329576103286102e6565b5b5f61033684828501610300565b91505092915050565b5f82825260208201905092915050565b7f76616c7565206d69736d617463680000000000000000000000000000000000005f82015250565b5f610383600e8361033f565b915061038e8261034f565b602082019050919050565b5f6020820190508181035f8301526103b081610377565b905091905056fea26469706673582212204d28a306634cc4321dbd572eed851aa320f7b0ee31d73ccdffb30e2fd053355a64736f6c63430008180033").into());
    assert_eq!(gas_used, 339371);
    let contract_address_2 = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, hex!("608060405260645f81905550606460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550606460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055506103ed806100d95f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c806320965255146100385780635524107714610056575b5f80fd5b610040610072565b60405161004d91906102cd565b60405180910390f35b610070600480360381019061006b9190610314565b6101b5565b005b5f805460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146100f3576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100ea90610399565b60405180910390fd5b5f5460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146101ae576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a590610399565b60405180910390fd5b5f54905090565b805f819055508060015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508060025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055507f63a242a632efe33c0e210e04e4173612a17efa4f16aa4890bc7e46caece80de05f546040516102aa91906102cd565b60405180910390a150565b5f819050919050565b6102c7816102b5565b82525050565b5f6020820190506102e05f8301846102be565b92915050565b5f80fd5b6102f3816102b5565b81146102fd575f80fd5b50565b5f8135905061030e816102ea565b92915050565b5f60208284031215610329576103286102e6565b5b5f61033684828501610300565b91505092915050565b5f82825260208201905092915050565b7f76616c7565206d69736d617463680000000000000000000000000000000000005f82015250565b5f610383600e8361033f565b915061038e8261034f565b602082019050919050565b5f6020820190508181035f8301526103b081610377565b905091905056fea26469706673582212204d28a306634cc4321dbd572eed851aa320f7b0ee31d73ccdffb30e2fd053355a64736f6c63430008180033").into());
    assert_eq!(gas_used, 339371);
    // call greeting EVM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address_1,
        hex!("20965255").into(),
        None,
        None,
    );
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 28179);
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000064",
        hex::encode(bytes)
    );
    // call greeting EVM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address_2,
        hex!("20965255").into(),
        None,
        None,
    );
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 28179);
    let bytes = result.output().unwrap_or_default().iter().as_slice();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000064",
        hex::encode(bytes)
    );
    // set value to 0x70
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address_2,
        hex!("552410770000000000000000000000000000000000000000000000000000000000000070").into(),
        None,
        None,
    );
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 38194);
    // check result is 0x70
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address_2,
        hex!("20965255").into(),
        None,
        None,
    );
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 28179);
    let bytes = result.output().unwrap_or_default().iter().as_slice();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000070",
        hex::encode(bytes)
    );
}

#[test]
fn test_evm_simple_send() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    const RECIPIENT_ADDRESS: Address = address!("1092381297182319023812093812312309123132");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = 1e9 as u128;
    let result = TxBuilder::call(&mut ctx, SENDER_ADDRESS, RECIPIENT_ADDRESS, None)
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 21_000);
    let tx_cost = U256::from(gas_price) * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(RECIPIENT_ADDRESS), U256::from(1e18));
}

#[test]
fn test_evm_create_and_send() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = 2e9 as u128;
    let result = TxBuilder::create(&mut ctx, SENDER_ADDRESS, crate::EXAMPLE_GREETING.into())
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&SENDER_ADDRESS, 0);
    assert!(result.is_success());
    let tx_cost = U256::from(gas_price) * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_evm_revert() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = 0;
    let result = TxBuilder::create(&mut ctx, SENDER_ADDRESS, hex!("5f5ffd").into())
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&SENDER_ADDRESS, 0);
    assert!(!result.is_success());
    assert_eq!(result.gas_used(), 53054);
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(2e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(0e18));
    // now send success tx
    let result = TxBuilder::create(&mut ctx, SENDER_ADDRESS, crate::EXAMPLE_GREETING.into())
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    println!("{:?}", result);
    // here nonce must be 1 because we increment nonce for failed txs
    let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&SENDER_ADDRESS, 1);
    println!("{}", contract_address);
    assert!(result.is_success());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_evm_self_destruct() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = 0;
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        hex!("6003600c60003960036000F36003ff").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&SENDER_ADDRESS, 0);
    println!("deployed contract address: {}", contract_address); // 0xF91c20C0Cafbfdc150adFf51BBfC5808EdDE7CB5
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 53842);
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
    // call self-destructed contract
    let result = TxBuilder::call(&mut ctx, SENDER_ADDRESS, contract_address, None)
        .gas_price(gas_price)
        .exec();
    if !result.is_success() {
        println!(
            "{}",
            from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
        );
    }
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 51003);
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(0e18));
    assert_eq!(
        ctx.get_balance(address!("0000000000000000000000000000000000000003")),
        U256::from(1e18)
    );
    // destruct in nested call
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        // Calling 0xF91c20C0Cafbfdc150adFf51BBfC5808EdDE7CB5
        hex!("6000600060006000600073f91c20c0cafbfdc150adff51bbfc5808edde7cb561FFFFF1").into(),
    )
    .exec();
    if !result.is_success() {
        println!("status: {:?}", result);
        try_print_utf8_error(result.output().cloned().unwrap_or_default().as_ref());
    }
    assert!(result.is_success());
    assert_eq!(result.gas_used(), 61128);
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(0e18));
    assert_eq!(
        ctx.get_balance(address!("0000000000000000000000000000000000000003")),
        U256::from(1e18)
    );
}

#[test]
fn test_evm_erc20() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../assets/ERC20.bin"))
            .unwrap()
            .into(),
    );
    let transfer_coin = |ctx: &mut EvmTestingContext| {
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into(), None,
            None,
        );
        println!("{:?}", result);
        assert!(result.is_success());
        assert!(result.gas_used() == 52353 || result.gas_used() == 35253);
    };
    let check_balance = |ctx: &mut EvmTestingContext, expected: U256| {
        // retrieve balance
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("70a082310000000000000000000000001111111111111111111111111111111111111111").into(),
            None,
            None,
        );
        println!("{:?}", result);
        assert!(result.is_success());
        assert_eq!(result.gas_used(), 24282);
        // check balance
        let output = result.output().unwrap_or_default().clone();
        assert_eq!(&expected.to_be_bytes::<32>(), output.as_ref());
    };
    // transfer 1 coin
    transfer_coin(&mut ctx);
    check_balance(&mut ctx, U256::from(1));
    transfer_coin(&mut ctx);
    check_balance(&mut ctx, U256::from(2));
}

#[test]
fn test_evm_balance() {
    const OWNER_ADDRESS: Address = address!("1111111111111111111111111111111111111111");
    let mut bytecode = Vec::new();
    bytecode.push(opcode::PUSH20);
    bytecode.extend_from_slice(OWNER_ADDRESS.as_slice());
    bytecode.push(opcode::BALANCE);
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::MSTORE);
    bytecode.push(opcode::PUSH1);
    bytecode.extend_from_slice(&[32]);
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::RETURN);
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.cfg.spec = SpecId::PRAGUE;
    let contract_address = ctx.deploy_evm_tx(
        Address::with_last_byte(255),
        wrap_to_init_code(&bytecode).into(),
    );
    let result = ctx.call_evm_tx(OWNER_ADDRESS, contract_address, hex!("").into(), None, None);
    println!("{:?}", result);
    assert!(result.is_success());
    let output = result.into_output().unwrap_or_default();
    assert_eq!(output.len(), 32);
    // assert_eq!(result.gas_used(), 21116);
    let balance = U256::from_be_slice(output.as_ref());
    assert_eq!(
        balance,
        U256::from_str_radix("999999999997000000", 10).unwrap()
    );
}

#[test]
fn test_wasm_erc20() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let bytecode: &[u8] = crate::EXAMPLE_ERC20.into();

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
    let transfer_coin = |ctx: &mut EvmTestingContext| {
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into(),
            None,
            None,
        );
        assert!(result.is_success());
        println!("{:?}", result);
    };
    transfer_coin(&mut ctx);
}

fn wrap_to_init_code(runtime: &[u8]) -> Vec<u8> {
    use opcode::*;

    let runtime_len = runtime.len();
    assert!(runtime_len <= 255, "runtime too long for PUSH1");

    // This will be calculated after the init prefix is formed
    let mut init = Vec::new();

    // Placeholder: assume PUSH1 for all pushes
    init.push(PUSH1);
    init.push(runtime_len as u8); // PUSH1 <runtime_len>
    init.push(PUSH1);
    init.push(0x00); // PUSH1 <offset> (patched later)
    init.push(PUSH1);
    init.push(0x00); // PUSH1 0x00
    init.push(CODECOPY);

    init.push(PUSH1);
    init.push(runtime_len as u8); // PUSH1 <runtime_len>
    init.push(PUSH1);
    init.push(0x00); // PUSH1 0x00
    init.push(RETURN);

    let code_offset = init.len(); // runtime starts here

    // Patch the offset in the second PUSH1 (which is at index 3)
    init[3] = code_offset as u8;

    // Append actual runtime code
    init.extend_from_slice(runtime);

    init
}

#[test]
fn test_evm_blake2f() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    // ctx.disabled_rwasm = true;
    const OWNER_ADDRESS: Address = Address::ZERO;

    // Deploy contract from bytecode (should match Blake2FCaller)
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../assets/Blake2FCaller.bin"))
            .unwrap()
            .into(),
    );

    // Method selector for `callBlake2F()`
    let call_selector = hex!("41f32a3a");

    // Call `callBlake2F()` on deployed contract
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        contract_address,
        call_selector.into(),
        None,
        None,
    );

    println!("{:?}", result);
    assert!(result.is_success());
    let output = result.output().unwrap_or_default();
    assert!(!output.is_empty());
    let blake2f_output = &output[64..]; // skip 2 32-byte words (offset and length)
    assert_eq!(blake2f_output.len(), 64);
    assert_eq!(result.gas_used(), 22579);
}

/// This test deploys the `HelloWorld` contract and directly calls its `sayHelloWorld()`
/// function to verify it returns the expected string. Do it using `SolCall` macro for better
/// readability.
#[test]
fn test_evm_greeting_using_sol_macro() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const OWNER_ADDRESS: Address = Address::ZERO;
    ctx.add_balance(OWNER_ADDRESS, U256::from(1e18));

    // Deploy HelloWorld contract
    let hello_world_bytecode = hex::decode(include_bytes!("../assets/HelloWorld.bin")).unwrap();
    let hello_world_address = ctx.deploy_evm_tx(OWNER_ADDRESS, hello_world_bytecode.into());

    // Encode sayHelloWorld() call
    sol! {
        function sayHelloWorld() public pure returns (string);
    }
    let input_data = sayHelloWorldCall {}.abi_encode();

    // Call contract directly
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        hello_world_address,
        input_data.into(),
        None,
        None,
    );
    assert!(result.is_success(), "call to sayHelloWorld() failed");

    // Decode result
    let output = result.output().unwrap_or_default();
    let decoded = sayHelloWorldCall::abi_decode_returns_validate(&output).unwrap();
    assert_eq!(decoded, "Hello, World");
}

/// This test deploys two contracts: `HelloWorld` and `Caller`.
/// It uses the `Caller` contract to perform a low-level `call` to the `sayHelloWorld()`
/// function of the `HelloWorld` contract via the `callExternal(address, bytes)` method.
#[test]
fn test_evm_caller() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    // ctx.cfg.disable_rwasm_proxy = true;
    const OWNER_ADDRESS: Address = Address::ZERO;
    ctx.add_balance(OWNER_ADDRESS, U256::from(1e18));

    // Step 1: Deploy HelloWorld contract
    let hello_world_bytecode = hex::decode(include_bytes!("../assets/HelloWorld.bin")).unwrap();
    let hello_world_address = ctx.deploy_evm_tx(OWNER_ADDRESS, hello_world_bytecode.into());
    // Step 2: Deploy Caller contract
    let caller_bytecode = hex::decode(include_bytes!("../assets/Caller.bin")).unwrap();
    let caller_contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, caller_bytecode.into());
    // Step 3: Encode sayHelloWorld() call (target function)
    sol! {
        function sayHelloWorld() public pure returns (string memory);
    }
    let say_hello_data = sayHelloWorldCall {}.abi_encode();

    // Step 4: Encode callExternal(address, bytes)
    sol! {
        function callExternal(address target, bytes calldata data) external returns (bool success, bytes memory result) ;
    }
    let call_input = callExternalCall {
        target: hello_world_address,
        data: say_hello_data.into(),
    }
    .abi_encode();

    // Step 5: Execute Caller.callExternal(hello_world_address, encoded(sayHelloWorld()))
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        caller_contract_address,
        call_input.into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success(), "call failed: {:?}", result);

    // Step 6: Decode return value
    let output = result.output().unwrap_or_default();
    try_print_utf8_error(&output);

    let return_data = callExternalCall::abi_decode_returns_validate(&output).unwrap();
    assert!(return_data.success);
    let return_data = return_data.result.to_vec();

    // ABI return is padded: decode inner string manually
    let hello_string = sayHelloWorldCall::abi_decode_returns_validate(&return_data).unwrap();
    assert_eq!(hello_string, "Hello, World");

    assert_eq!(result.gas_used(), 26788);
}

#[test]
fn test_evm_ecrecover_out_of_gas() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    // ctx.cfg.disable_rwasm_proxy = true;
    const OWNER_ADDRESS: Address = address!("1234121212121212121212121212121212121234");

    // Some random input data for ecrecover precompile
    let input = hex!("11223344556677889900aabbccddeeff00112233445566778899aabbccddeeff000000000000000000000000000000000000000000000000000000000000001b3c8f1a1c9d6cc4b11bd8b32c98f627f7796fbc1db6d3fa4a51d87061b512b5b55b81a37853a38a91dc4fc8a3a64b105f334cf5dfd0f28ad89a78533d817c6a19");

    // Call `callBlake2F()` on deployed contract
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        PRECOMPILE_SECP256K1_RECOVER, // calling ecrecover precompile
        input.into(),
        Some(25650),         // gas limit
        Some(U256::from(1)), // value
    );

    println!("{:?}", result);
    assert_eq!(result.gas_used(), 25650);
    assert!(result.is_halt());
}

/// This test calls a contract function that attempts to transfer 1 wei to `TARGET_ADDRESS`.
/// The transfer is expected to fail and revert because the contract itself holds no balance
/// and cannot cover the 1 wei being sent.
///
/// Test is ignored because it is an example of a test where our EVM implementation differs from
/// the original one.
#[test]
#[ignore]
fn test_evm_send_one_wei_to_precompile() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    // ctx.disabled_rwasm = true;
    const OWNER_ADDRESS: Address = Address::ZERO;
    const TARGET_ADDRESS: Address = PRECOMPILE_BLAKE2F; // any precompile
    ctx.add_balance(OWNER_ADDRESS, U256::from(1e18));

    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../assets/SendOneWei.bin"))
            .unwrap()
            .into(),
    );
    sol! {
        function sendOneWei(address payable target) external;
    }
    let call_data = sendOneWeiCall {
        target: TARGET_ADDRESS,
    }
    .abi_encode();
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        contract_address,
        call_data.into(),
        None,
        Some(U256::ZERO),
    );
    println!("{:?}", result);
    assert!(matches!(result, Revert { .. }));
    let message = "Transfer failed".as_bytes();
    let found = result
        .output()
        .unwrap()
        .windows(message.len())
        .any(|w| w == message);
    assert!(found, "Expected revert message not found");
    assert_eq!(result.gas_used(), 29015);
}
