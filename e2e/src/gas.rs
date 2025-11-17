use crate::EvmTestingContextWithGenesis;
use core::str::from_utf8;
use fluentbase_codec::byteorder::LittleEndian;
use fluentbase_contracts::FLUENTBASE_EXAMPLES_GREETING;
use fluentbase_sdk::{
    address, byteorder::ByteOrder, bytes, syscall::SYSCALL_ID_CALL, Address, SysFuncIdx,
    FUEL_DENOM_RATE, STATE_MAIN, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use revm::context::result::{ExecutionResult, Output};
use rwasm::{instruction_set, RwasmModule, RwasmModuleInner};

#[test]
fn test_simple_nested_call() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const ACCOUNT1_ADDRESS: Address = address!("1111111111111111111111111111111111111111");
    const ACCOUNT2_ADDRESS: Address = address!("1111111111111111111111111111111111111112");
    const ACCOUNT3_ADDRESS: Address = address!("1111111111111111111111111111111111111113");
    let _account1 = ctx.add_wasm_contract(
        ACCOUNT1_ADDRESS,
        RwasmModule::with_one_function(instruction_set! {
            ConsumeFuel(1 * FUEL_DENOM_RATE)
            // add one memory page
            I32Const(1)
            MemoryGrow
            Drop
            // write exit code into 0 memory offset
            I32Const(0)
            I32Const(100)
            I32Store(0)
            // call write output (offset=0, length=1)
            I32Const(0)
            I32Const(1)
            Call(SysFuncIdx::WRITE_OUTPUT)
            // exit 0
            I32Const(0)
            Call(SysFuncIdx::EXIT)
        }),
    );
    let _account2 = ctx.add_wasm_contract(
        ACCOUNT2_ADDRESS,
        RwasmModule::with_one_function(instruction_set! {
            ConsumeFuel(2 * FUEL_DENOM_RATE)
            // add one memory page
            I32Const(1)
            MemoryGrow
            Drop
            // write exit code into 0 memory offset
            I32Const(0)
            I32Const(20)
            I32Store(0)
            // call write output (offset=0, length=1)
            I32Const(0)
            I32Const(1)
            Call(SysFuncIdx::WRITE_OUTPUT)
            // exit 0
            I32Const(0)
            Call(SysFuncIdx::EXIT)
        }),
    );
    let mut data_section = vec![];
    data_section.extend_from_slice(&SYSCALL_ID_CALL.0); // 0..32
    data_section.extend_from_slice(ACCOUNT1_ADDRESS.as_slice()); // 32..
    data_section.extend_from_slice(U256::ZERO.as_le_slice());
    data_section.extend_from_slice(ACCOUNT2_ADDRESS.as_slice()); // 84..
    data_section.extend_from_slice(U256::ZERO.as_le_slice());
    data_section.extend_from_slice(&[0, 0, 0, 0]); // 136..
    assert_eq!(data_section.len(), 140);
    let code_section = instruction_set! {
        // alloc and init memory
        I32Const(1)
        MemoryGrow
        Drop
        I32Const(0)
        I32Const(0)
        I32Const(data_section.len() as u32)
        MemoryInit(0)
        DataDrop(0)
        // sys exec hash
        ConsumeFuel(1 * FUEL_DENOM_RATE)
        I32Const(0) // hash32_ptr
        I32Const(32) // input_ptr
        I32Const(52) // input_len
        I32Const(0) // fuel_ptr
        I32Const(STATE_MAIN) // state
        Call(SysFuncIdx::EXEC)
        Drop
        I32Const(200) // target offset
        I32Const(0) // source offest
        I32Const(1) // buffer length
        Call(SysFuncIdx::READ_OUTPUT)
        // sys exec hash
        ConsumeFuel(2 * FUEL_DENOM_RATE)
        I32Const(0) // hash32_ptr
        I32Const(84) // input_ptr
        I32Const(52) // input_len
        I32Const(0) // fuel_ptr
        I32Const(STATE_MAIN) // state
        Call(SysFuncIdx::EXEC)
        Drop
        I32Const(201) // target offset
        I32Const(0) // source offest
        I32Const(1) // buffer length
        Call(SysFuncIdx::READ_OUTPUT)
        // write the sum of two result codes into 1 byte result
        ConsumeFuel(3 * FUEL_DENOM_RATE)
        I32Const(200)
        I32Load8U(0)
        I32Const(201)
        I32Load8U(0)
        I32Add
        LocalGet(1)
        I32Const(136)
        LocalSet(2)
        I32Store(0)
        // call "_write" func
        I32Const(136) // offset
        I32Const(4) // length
        Call(SysFuncIdx::WRITE_OUTPUT)
        // exit with 0 exit code
        ConsumeFuel(4 * FUEL_DENOM_RATE)
        I32Const(0)
        Call(SysFuncIdx::EXIT)
    };
    ctx.add_wasm_contract(
        ACCOUNT3_ADDRESS,
        RwasmModuleInner {
            code_section,
            data_section,
            ..Default::default()
        },
    );
    let result = TxBuilder::call(&mut ctx, Address::ZERO, ACCOUNT3_ADDRESS, None)
        .gas_price(0)
        .gas_limit(26219)
        .exec();
    println!("{:?}", result);
    assert!(result.is_success());
    let output = result.output().unwrap_or_default();
    assert!(output.len() >= 4);
    let value = LittleEndian::read_i32(output.as_ref());
    assert_eq!(value, 120);
    assert!(result.is_success());
    // 21k is tx cost
    // + 2600 * 2 nested calls
    // + account1 call 1000 = 1 gas
    // + account2 call 2000 = 2 gas
    // + account3 call 1024 (mem) + 1000 + 2000 + 3000 + 4000 = 12 gas
    // Result: 21000+2600*2 + 3+4+12 = 26219
    // TODO(dmitry123): "we don't do ceil rounding for consumed fuel"
    assert_eq!(result.gas_used(), 26213);
    // assert_eq!(result.gas_used(), 26219);
}

#[test]
#[ignore]
fn test_deploy_gas_spend() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;

    let result = TxBuilder::create(
        &mut ctx,
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode.into(),
    )
    .exec();
    if !result.is_success() {
        println!("{:?}", result);
        println!(
            "{}",
            from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
        );
    }
    // 62030 - init contract cost
    // 67400 - store space cost in fuel
    // 5126  - opcode cost in fuel
    assert_eq!(result.gas_used(), 62030 + (67400 + 5126) / 1000 + 1);
}

#[test]
#[ignore]
fn test_blended_gas_spend_wasm_from_evm() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const ACCOUNT1_ADDRESS: Address = address!("1111111111111111111111111111111111111111");
    const ACCOUNT2_ADDRESS: Address = address!("1111111111111111111111111111111111111112");
    const DEPLOYER_ADDRESS: Address = Address::ZERO;

    let _account1 = ctx.add_wasm_contract(
        ACCOUNT1_ADDRESS,
        RwasmModule::with_one_function(instruction_set! {
            ConsumeFuel(1000u32)
            I32Const(-1)
            Call(SysFuncIdx::EXIT)
        }),
    );
    let _account2 = ctx.add_wasm_contract(
        ACCOUNT2_ADDRESS,
        RwasmModule::with_one_function(instruction_set! {
            ConsumeFuel(2000u32)
            I32Const(-20)
            Call(SysFuncIdx::EXIT)
        }),
    );

    let result = TxBuilder::create(
        &mut ctx,
        DEPLOYER_ADDRESS,
        hex!("608060405260b780600f5f395ff3fe6080604052348015600e575f5ffd5b50600436106026575f3560e01c806365becaf314602a575b5f5ffd5b60306032565b005b5f73111111111111111111111111111111111111111190505f60405180602001604052805f81525090505f5f9050604051825160208401818184375f5f83855f8a6107d0f1935050505050505056fea26469706673582212207eef72b1ab13ead60c06c9a0f00f0a2c74c2438d873e963f3b2bf9a2e092874564736f6c634300081c0033").into(),
    ).exec();
    if !result.is_success() {
        println!("Result: {:?}", result);
        println!(
            "{}",
            from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
        );
    }
    let address = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Create(_, address) => address.unwrap(),
            _ => panic!("expected 'create'"),
        },
        _ => panic!("expected 'success'"),
    };
    println!("Contract address: {:?}", address);
    let result = TxBuilder::call(
        &mut ctx,
        address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        address,
        None,
    )
    .input(bytes!("65becaf3"))
    .exec();

    assert!(result.is_success());
    println!("Result: {:?}", result);
    // 21064 is tx cost
    // + 2600 call cost
    // + 1 call wasm code
    // + 255 evm opcodes cost
    assert_eq!(result.gas_used(), 21064 + 1 + 2600 + 255);
}

#[test]
fn test_blended_gas_spend_evm_from_wasm() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    const ACCOUNT3_ADDRESS: Address = address!("1111111111111111111111111111111111111113");

    let result = TxBuilder::create(
        &mut ctx,
        DEPLOYER_ADDRESS,
        hex!("6080604052610594806100115f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f5ffd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a9190610380565b60405180910390f35b61007b6100dd565b6040516100889190610380565b60405180910390f35b61009961011a565b6040516100a69190610380565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103d6565b915050600a8261019d919061044a565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be61047a565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104a7565b925050600a8561021791906104ce565b603061022391906104fe565b60f81b81838151811061023957610238610531565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a85610274919061044a565b94506101f5565b8093505050505b919050565b5f5ffd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f5ffd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b8281835e5f83830152505050565b5f601f19601f8301169050919050565b5f61035282610310565b61035c818561031a565b935061036c81856020860161032a565b61037581610338565b840191505092915050565b5f6020820190508181035f8301526103988184610348565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103e0826103cd565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8203610412576104116103a0565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f610454826103cd565b915061045f836103cd565b92508261046f5761046e61041d565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104b1826103cd565b91505f82036104c3576104c26103a0565b5b600182039050919050565b5f6104d8826103cd565b91506104e3836103cd565b9250826104f3576104f261041d565b5b828206905092915050565b5f610508826103cd565b9150610513836103cd565b925082820190508082111561052b5761052a6103a0565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea26469706673582212205f47a1f79c07854bad446dbc1572d306c5758cabc8296071e80f814e5ca99c8b64736f6c634300081c0033").into(),
    ).exec();
    if !result.is_success() {
        println!("Result: {:?}", result);
        println!(
            "{}",
            from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
        );
    }
    let address = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Create(_, address) => address.unwrap(),
            _ => panic!("expected 'create'"),
        },
        _ => panic!("expected 'success'"),
    };
    println!("Contract address: {:?}", address);

    let mut data_section = vec![];
    data_section.extend_from_slice(&SYSCALL_ID_CALL.0); // 0..32
    data_section.extend_from_slice(address.as_slice()); // 32..
    data_section.extend_from_slice(U256::ZERO.as_le_slice());
    data_section.extend_from_slice(bytes!("45773e4e").to_vec().as_slice());

    let code_section = instruction_set! {
        // alloc and init memory
        I32Const(1)
        MemoryGrow
        Drop
        I32Const(0)
        I32Const(0)
        I32Const(data_section.len() as u32)
        MemoryInit(0)
        DataDrop(0)
        // sys exec hash
        ConsumeFuel(10u32)
        I32Const(0) // hash32_ptr
        I32Const(32) // input_ptr
        I32Const(56) // input_len
        I32Const(0) // fuel_ptr
        I32Const(STATE_MAIN) // state
        Call(SysFuncIdx::EXEC)
        I32Const(0) // hash32_ptr
        I32Const(32) // input_ptr
        I32Const(56) // input_len
        I32Const(0) // fuel_ptr
        I32Const(STATE_MAIN) // state
        Call(SysFuncIdx::EXEC)
        // what's on the stack?
        Return
    };
    ctx.add_wasm_contract(
        ACCOUNT3_ADDRESS,
        RwasmModuleInner {
            code_section,
            data_section,
            ..Default::default()
        },
    );
    let result = TxBuilder::call(
        &mut ctx,
        address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        ACCOUNT3_ADDRESS,
        None,
    )
    .gas_price(0)
    .exec();

    println!("Result: {:?}", result);
    assert!(result.is_success());

    // 21064 is tx cost
    // + 2600 cold call cost
    // + 637 evm opcodes cost
    // + 100 warm call cost
    // + 637 evm opcodes cost
    // + 1 call wasm code
    assert_eq!(result.gas_used(), 21000 + 2600 + 637 + 100 + 637 + 1);
    // TODO(dmitry123): "wasm code cost should be 2, not 1"
}
