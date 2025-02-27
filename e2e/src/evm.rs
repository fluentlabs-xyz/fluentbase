use crate::utils::{EvmTestingContext, TxBuilder};
use core::str::from_utf8;
use fluentbase_sdk::{
    address,
    calc_create_address,
    testing::TestingContextNativeAPI,
    Address,
    U256,
};
use hex_literal::hex;
use revm::interpreter::opcode;

#[test]
fn test_evm_greeting() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033").into());
    // call greeting EVM contract
    println!("\n\n\n");
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        hex!("45773e4e").into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    let bytes = &bytes[64..75];
    assert_eq!("Hello World", from_utf8(bytes.as_ref()).unwrap());
    assert_eq!(result.gas_used(), 21792);
}

#[test]
fn test_evm_storage() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address_1 = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, hex!("608060405260645f81905550606460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550606460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055506103ed806100d95f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c806320965255146100385780635524107714610056575b5f80fd5b610040610072565b60405161004d91906102cd565b60405180910390f35b610070600480360381019061006b9190610314565b6101b5565b005b5f805460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146100f3576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100ea90610399565b60405180910390fd5b5f5460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146101ae576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a590610399565b60405180910390fd5b5f54905090565b805f819055508060015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508060025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055507f63a242a632efe33c0e210e04e4173612a17efa4f16aa4890bc7e46caece80de05f546040516102aa91906102cd565b60405180910390a150565b5f819050919050565b6102c7816102b5565b82525050565b5f6020820190506102e05f8301846102be565b92915050565b5f80fd5b6102f3816102b5565b81146102fd575f80fd5b50565b5f8135905061030e816102ea565b92915050565b5f60208284031215610329576103286102e6565b5b5f61033684828501610300565b91505092915050565b5f82825260208201905092915050565b7f76616c7565206d69736d617463680000000000000000000000000000000000005f82015250565b5f610383600e8361033f565b915061038e8261034f565b602082019050919050565b5f6020820190508181035f8301526103b081610377565b905091905056fea26469706673582212204d28a306634cc4321dbd572eed851aa320f7b0ee31d73ccdffb30e2fd053355a64736f6c63430008180033").into());
    let contract_address_2 = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, hex!("608060405260645f81905550606460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550606460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055506103ed806100d95f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c806320965255146100385780635524107714610056575b5f80fd5b610040610072565b60405161004d91906102cd565b60405180910390f35b610070600480360381019061006b9190610314565b6101b5565b005b5f805460015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146100f3576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100ea90610399565b60405180910390fd5b5f5460025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054146101ae576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a590610399565b60405180910390fd5b5f54905090565b805f819055508060015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508060025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3073ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055507f63a242a632efe33c0e210e04e4173612a17efa4f16aa4890bc7e46caece80de05f546040516102aa91906102cd565b60405180910390a150565b5f819050919050565b6102c7816102b5565b82525050565b5f6020820190506102e05f8301846102be565b92915050565b5f80fd5b6102f3816102b5565b81146102fd575f80fd5b50565b5f8135905061030e816102ea565b92915050565b5f60208284031215610329576103286102e6565b5b5f61033684828501610300565b91505092915050565b5f82825260208201905092915050565b7f76616c7565206d69736d617463680000000000000000000000000000000000005f82015250565b5f610383600e8361033f565b915061038e8261034f565b602082019050919050565b5f6020820190508181035f8301526103b081610377565b905091905056fea26469706673582212204d28a306634cc4321dbd572eed851aa320f7b0ee31d73ccdffb30e2fd053355a64736f6c63430008180033").into());
    // call greeting EVM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address_1,
        hex!("20965255").into(),
        None,
        None,
    );
    assert!(result.is_success());
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
    // check result is 0x70
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address_2,
        hex!("20965255").into(),
        None,
        None,
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default().iter().as_slice();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000070",
        hex::encode(bytes)
    );
}

#[test]
fn test_simple_send() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    const RECIPIENT_ADDRESS: Address = address!("1092381297182319023812093812312309123132");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(1e9);
    let result = TxBuilder::call(&mut ctx, SENDER_ADDRESS, RECIPIENT_ADDRESS, None)
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    assert!(result.is_success());
    let tx_cost = gas_price * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(RECIPIENT_ADDRESS), U256::from(1e18));
}

#[test]
fn test_create_send() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(2e9);
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        include_bytes!("../../examples/greeting/lib.wasm").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    let contract_address = calc_create_address::<TestingContextNativeAPI>(&SENDER_ADDRESS, 0);
    assert!(result.is_success());
    let tx_cost = gas_price * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_evm_revert() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(0);
    let result = TxBuilder::create(&mut ctx, SENDER_ADDRESS, hex!("5f5ffd").into())
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    let contract_address = calc_create_address::<TestingContextNativeAPI>(&SENDER_ADDRESS, 0);
    assert!(!result.is_success());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(2e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(0e18));
    // now send success tx
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        include_bytes!("../../examples/greeting/lib.wasm").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    // here nonce must be 1 because we increment nonce for failed txs
    let contract_address = calc_create_address::<TestingContextNativeAPI>(&SENDER_ADDRESS, 1);
    println!("{}", contract_address);
    assert!(result.is_success());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_evm_self_destruct() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    // const DESTROYED_ADDRESS: Address = address!("f91c20c0cafbfdc150adff51bbfc5808edde7cb5");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(0);
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        hex!("6003600c60003960036000F36003ff").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    let contract_address = calc_create_address::<TestingContextNativeAPI>(&SENDER_ADDRESS, 0);
    assert!(result.is_success());
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
        hex!("6000600060006000600073f91c20c0cafbfdc150adff51bbfc5808edde7cb561FFFFF1").into(),
    )
    .exec();
    if !result.is_success() {
        println!("status: {:?}", result);
        println!(
            "utf8-output: {}",
            from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
        );
    }
    assert!(result.is_success());
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
    let mut ctx = EvmTestingContext::default();
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

hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001"
).into(),             None,
            None,
        );
        println!("{:?}", result);
        assert!(result.is_success());
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
    const OWNER_ADDRESS: Address = Address::with_last_byte(1);
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
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default();
    ctx.add_bytecode(Address::with_last_byte(255), bytecode.into());
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        Address::with_last_byte(255),
        hex!("").into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());
    let output = result.into_output().unwrap_or_default();
    assert_eq!(output.len(), 32);
    let balance = U256::from_be_slice(output.as_ref());
    assert_eq!(
        balance,
        U256::from_str_radix("999999999997000000", 10).unwrap()
    );
}
