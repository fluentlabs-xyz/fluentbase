use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolValue};
use fluentbase_contracts::{FLUENTBASE_EXAMPLES_ERC20, FLUENTBASE_EXAMPLES_GREETING};
use fluentbase_sdk::{constructor::encode_constructor_params, hex, Address, Bytes};
use fluentbase_testing::EvmTestingContext;
use revm::context::result::ExecutionResult;
use std::time::Instant;

/// Contract `ContractDeployer.sol` is a smart contract that deploys
/// the given smart contract using the CREATE opcode of the EVM.
/// Through this opcode, we should be able to deploy both WASM
/// and EVM bytecode.
fn deploy_via_deployer(ctx: &mut EvmTestingContext, bytecode: Bytes) -> Address {
    let owner: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        owner,
        hex::decode(include_bytes!("../assets/ContractDeployer.bin"))
            .unwrap()
            .into(),
    );
    sol! {
        function deploy(bytes memory bytecode) public returns (address contractAddress);
    }
    let encoded_call = deployCall { bytecode }.abi_encode();
    let result = ctx.call_evm_tx(
        owner,
        contract_address,
        encoded_call.into(),
        Some(10_000_000),
        None,
    );
    println!("{:?}", result);
    assert!(
        result.is_success(),
        "call to \"deploy\" method of ContractDeployer.sol failed"
    );
    let address = <Address>::abi_decode_validate(result.output().unwrap()).unwrap();
    address
}

#[test]
#[ignore]
fn test_evm_create_evm_contract() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let owner: Address = Address::ZERO;
    let bytecode = hex::decode(include_bytes!("../assets/HelloWorld.bin")).unwrap();
    let contract_address = deploy_via_deployer(&mut ctx, bytecode.into());
    sol! {
        function sayHelloWorld() public pure returns (string memory);
    }
    let encoded_call = sayHelloWorldCall {}.abi_encode();
    let result = ctx.call_evm_tx(owner, contract_address, encoded_call.into(), None, None);
    assert!(result.is_success());
    let string = <String>::abi_decode_validate(result.output().unwrap()).unwrap();
    assert_eq!(string, "Hello, World");
}

#[test]
#[ignore]
fn test_evm_create_wasm_contract() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let owner: Address = Address::ZERO;
    let contract_address =
        deploy_via_deployer(&mut ctx, FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode.into());
    let result = ctx.call_evm_tx(owner, contract_address, Bytes::new(), None, None);
    println!("{:#?}", result);
    assert!(result.is_success());
    let output = result.output().unwrap().to_vec();
    assert_eq!(String::from_utf8(output).unwrap(), "Hello, World");
}

#[test]
#[ignore]
fn test_evm_create_large_wasm_contract() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // Add constructor parameters for ERC20
    let bytecode: &[u8] = FLUENTBASE_EXAMPLES_ERC20.wasm_bytecode.into();
    let constructor_params = hex!("000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000000000954657374546f6b656e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000035453540000000000000000000000000000000000000000000000000000000000");
    let encoded_constructor_params = encode_constructor_params(&constructor_params);
    let mut input: Vec<u8> = Vec::new();
    input.extend(bytecode);
    input.extend(encoded_constructor_params);

    deploy_via_deployer(&mut ctx, input.into());
}

#[test]
fn test_locals_amplification_find_limit() {
    let test_cases: &[(u32, bool)] = &[(4, true), (20, false)];
    let owner: Address = Address::ZERO;
    // Test various function counts to find limits
    for (num_funcs, expected_ok) in test_cases.iter().cloned() {
        let result = try_deploy(owner, num_funcs);
        if expected_ok {
            assert!(result.is_ok(), "expected result is OK");
        } else {
            assert!(result.is_err(), "expected result is ERR");
        }
    }
}

fn try_deploy(owner: Address, num_funcs: u32) -> Result<(), ExecutionResult> {
    let wasm = build_max_locals_module(num_funcs);
    let wasm_len = wasm.len();

    // Create fresh context for each test to avoid state interference
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let start = Instant::now();
    match ctx.deploy_evm_tx_with_gas_result(owner, wasm.into()) {
        Ok((addr, gas)) => {
            let size = ctx.get_code(addr).unwrap().len();
            println!("funcs #{:>2}: initcode {:>4} bytes; {:>12} gas; deployed {:>9} bytes; time {}ms; OK", num_funcs, wasm_len, gas, size, start.elapsed().as_millis());
            Ok(())
        }
        Err(result) => {
            println!("funcs #{}: initcode {:>12} bytes; {:>12} gas; deployed {:>12} bytes; time {}ms; Err", num_funcs, wasm_len, "-", 0, start.elapsed().as_millis());
            Err(result)
        }
    }
}

fn leb128(mut n: u32) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (n & 0x7F) as u8;
        n >>= 7;
        if n != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
    out
}

/// Build WASM module with N functions, each with 32767 i64 locals
fn build_max_locals_module(num_funcs: u32) -> Vec<u8> {
    let num_funcs_leb = leb128(num_funcs);

    let func_section_size = num_funcs_leb.len() + num_funcs as usize;
    let func_section_size_leb = leb128(func_section_size as u32);

    // Each function body: size=6, 1 local decl, 32767 (0xFF 0xFF 0x01), i64, end
    let body: &[u8] = &[0x06, 0x01, 0xff, 0xff, 0x01, 0x7e, 0x0b];
    let code_section_size = num_funcs_leb.len() + (num_funcs as usize * body.len());
    let code_section_size_leb = leb128(code_section_size as u32);

    let mut wasm = vec![
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type section: () -> ()
    ];

    // Function section
    wasm.push(0x03);
    wasm.extend_from_slice(&func_section_size_leb);
    wasm.extend_from_slice(&num_funcs_leb);
    for _ in 0..num_funcs {
        wasm.push(0x00);
    }

    // Export section (export first func as "main")
    wasm.extend_from_slice(&[0x07, 0x08, 0x01, 0x04, 0x6d, 0x61, 0x69, 0x6e, 0x00, 0x00]);

    // Code section
    wasm.push(0x0a);
    wasm.extend_from_slice(&code_section_size_leb);
    wasm.extend_from_slice(&num_funcs_leb);
    for _ in 0..num_funcs {
        wasm.extend_from_slice(body);
    }

    wasm
}

/// Single function with 32767 i64 locals
const SINGLE_FUNC_MAX_LOCALS_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
    0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type section: () -> ()
    0x03, 0x02, 0x01, 0x00, // function section: 1 func, type 0
    0x07, 0x08, 0x01, 0x04, 0x6d, 0x61, 0x69, 0x6e, 0x00, 0x00, // export "main"
    0x0a, 0x08, 0x01, 0x06, 0x01, 0xff, 0xff, 0x01, 0x7e, 0x0b, // code: 32767 i64 locals
];
