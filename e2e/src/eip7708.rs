use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{address, Address, Bytes, Log, B256, U256};
use fluentbase_sdk_testing::EvmTestingContext;
use fluentbase_types::NATIVE_TRANSFER_KECCAK;
use revm::context::result::ExecutionResult;

fn call_success(
    ctx: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    value: U256,
) -> ExecutionResult {
    let result = ctx.call_evm_tx(caller, callee, Bytes::default(), None, Some(value));
    println!("result: {:?}", result);
    assert!(result.is_success());
    result
}

fn find_native_transfer_log(logs: &[Log], from: Address, to: Address, value: U256) -> bool {
    let value: B256 = value.into();
    logs.iter().any(|log| {
        log.topics()[0] == NATIVE_TRANSFER_KECCAK
            && log.topics()[1] == from.into_word()
            && log.topics()[2] == to.into_word()
            && log.data.data.as_ref() == value.as_slice()
    })
}

#[test]
fn eip7708_simple_send() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let result = call_success(
        &mut ctx,
        address!("0x1111111111111111111111111111111111111111"),
        address!("0x2222222222222222222222222222222222222222"),
        U256::from(1),
    );
    println!("result: {:?}", result);
    assert!(find_native_transfer_log(
        result.logs(),
        address!("0x1111111111111111111111111111111111111111"),
        address!("0x2222222222222222222222222222222222222222"),
        U256::from(1),
    ));
}

#[test]
fn eip7708_test() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // simple send
    let result = call_success(
        &mut ctx,
        address!("0x1111111111111111111111111111111111111111"),
        address!("0x2222222222222222222222222222222222222222"),
        U256::from(1),
    );
    println!("result: {:?}", result);
    assert!(find_native_transfer_log(
        result.logs(),
        address!("0x1111111111111111111111111111111111111111"),
        address!("0x2222222222222222222222222222222222222222"),
        U256::from(1),
    ));

    let sender_init_bytecode = [
        0x60, 0x10, // PUSH1 0x10
        0x60, 0x06, // PUSH1 0x06
        0x5F, // PUSH0
        0x39, // CODECOPY
        0x60, 0x10, // PUSH1 0x10
        0x5F, // PUSH0
        0xF3, // RETURN
        0x5F, // PUSH0 (retLength)
        0x5F, // PUSH0 (retOffset)
        0x5F, // PUSH0 (argsLength)
        0x5F, // PUSH0 (argsOffset)
        0x60, 0x01, // PUSH1 0x01 (value)
        0x64, 0x02, 0x02, 0x02, 0x02, // PUSH4 0x02020202 (addr)
        0x61, 0xFF, 0xFF, // PUSH2 0xFFFF (gas)
        0xF1, // CALL
        0x00, // STOP
    ];
    let deployed_address = ctx.deploy_evm_tx(
        address!("0x2222222222222222222222222222222222222222"),
        sender_init_bytecode.into(),
    );
    let deployed_bytecode = ctx.get_code(deployed_address).unwrap();
    assert!(!deployed_bytecode.is_empty());

    // success transfer (no bytecode)
    let result = call_success(
        &mut ctx,
        address!("0x1111111111111111111111111111111111111111"),
        deployed_address,
        U256::from(1),
    );
    println!("result: {:?}", result);
    assert!(find_native_transfer_log(
        result.logs(),
        address!("0x1111111111111111111111111111111111111111"),
        deployed_address,
        U256::from(1),
    ));

    let recipient_bytecode = [
        0x5F, // PUSH0 (retLength)
        0x5F, // PUSH0 (retOffset)
        0xFD, // REVERT
    ];
    ctx.add_bytecode(
        address!("0x0000000000000000000000000000000002020202"),
        recipient_bytecode.into(),
    );
}
