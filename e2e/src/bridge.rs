use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolEvent};
use core::str::from_utf8;
use fluentbase_revm::RwasmHaltReason;
use fluentbase_sdk::{
    address, bytes, calc_create_address, Address, Bytes, B256, PRECOMPILE_ROLLUP_BRIDGE, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use revm::{
    bytecode::opcode,
    context::result::{ExecutionResult, Output},
};

sol! {
    event ReceivedMessage(bytes32 messageHash, bool successfulCall, bytes returnData);

    event RetriedFailedMessage(bytes32 messageHash, bool successfulCall, bytes returnData);

    event SentMessage(
        address indexed sender,
        address indexed to,
        uint256 value,
        uint256 chainId,
        uint256 blockNumber,
        uint256 nonce,
        bytes32 messageHash,
        bytes data
    );

    function receiveMessage(
        address from,
        address to,
        uint256 value,
        uint256 chainId,
        uint256 blockNumber,
        uint256 messageNonce,
        bytes calldata message
    ) external payable;

    function receiveFailedMessage(
        address from,
        address to,
        uint256 value,
        uint256 chainId,
        uint256 blockNumber,
        uint256 messageNonce,
        bytes calldata message
    ) external payable;

    function sendMessage(address to, bytes calldata message) external payable;
}

#[test]
fn test_bridge_mints_tokens_on_successful_deposit() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = ReceivedMessage {
        messageHash: B256::ZERO,
        successfulCall: true,
        returnData: Bytes::new(),
    }
    .encode_data();
    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 6 + 2 + 32 + 3;
    // copy log data (6)
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap()); // length
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap()); // offset
    bytecode.push(opcode::PUSH0); // data offset
    bytecode.push(opcode::CODECOPY);
    // call log1 (2 + 32 + 3)
    bytecode.push(opcode::PUSH32); // topic
    bytecode.extend_from_slice(ReceivedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1); // data length
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0); // data offset
    bytecode.push(opcode::LOG1);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);
    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let receive_message_input = receiveMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1e9),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();
    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        receive_message_input.into(),
        None,
        None,
    );
    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance + U256::from(1e9), new_balance);
}

#[test]
fn test_failed_send_message_does_not_burn_bridge_balance() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let mut bytecode = Vec::new();
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(SentMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let send_message_call = sendMessageCall {
        to: Address::repeat_byte(0x01),
        message: Bytes::new(),
    }
    .abi_encode();
    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        send_message_call.into(),
        None,
        Some(U256::from(123)),
    );
    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    // Balance remains the same since we burn ETH
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_bridge_transaction_fails_on_zero_topics_emitted() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let mut bytecode = Vec::new();
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(SentMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let send_message_call = sendMessageCall {
        to: Address::repeat_byte(0x01),
        message: Bytes::new(),
    }
    .abi_encode();
    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        send_message_call.into(),
        None,
        Some(U256::from(123)),
    );
    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    // Balance remains the same since we burn ETH
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_message_revert_restores_bridge_balance() {
    // deploy greeting EVM contract
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let mut bytecode = Vec::new();
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::REVERT);
    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let receive_message_input = receiveMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1e9),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();
    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        receive_message_input.into(),
        None,
        None,
    );
    assert!(!result.is_success());
    // Balance must remain the same
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_send_message_burns_balance_on_successful_valid_log() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = SentMessage {
        sender: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x02),
        value: U256::from(123),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        nonce: U256::ZERO,
        messageHash: B256::ZERO,
        data: Bytes::new(),
    };
    let encoded_log_data = log_data.encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 86;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(encoded_log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    let (topic0, topic1, topic2) = log_data.topics();
    bytecode.push(opcode::PUSH20);
    bytecode.extend_from_slice(topic2.as_slice());
    bytecode.push(opcode::PUSH20);
    bytecode.extend_from_slice(topic1.as_slice());
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(topic0.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(encoded_log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG3);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(encoded_log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, U256::ZERO);
    let input = sendMessageCall {
        to: Address::repeat_byte(0x02),
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        Some(U256::from(123)),
    );
    assert!(result.is_success());

    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(new_balance, U256::ZERO);
}

#[test]
fn test_send_message_fails_on_value_mismatch() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = SentMessage {
        sender: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x02),
        value: U256::from(999), // mismatch
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        nonce: U256::ZERO,
        messageHash: B256::ZERO,
        data: Bytes::new(),
    };
    let encoded_log_data = log_data.encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 86;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(encoded_log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    let (topic0, topic1, topic2) = log_data.topics();
    bytecode.push(opcode::PUSH20);
    bytecode.extend_from_slice(topic2.as_slice());
    bytecode.push(opcode::PUSH20);
    bytecode.extend_from_slice(topic1.as_slice());
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(topic0.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(encoded_log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG3);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(encoded_log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = sendMessageCall {
        to: Address::repeat_byte(0x02),
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        Some(U256::from(123)),
    );

    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_send_message_fails_when_no_matching_log_emitted() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, vec![opcode::STOP]);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = sendMessageCall {
        to: Address::repeat_byte(0x02),
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        Some(U256::from(123)),
    );

    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_message_fails_when_success_log_missing() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, vec![opcode::STOP]);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_message_burns_balance_when_log_marks_unsuccessful_call() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = ReceivedMessage {
        messageHash: B256::ZERO,
        successfulCall: false,
        returnData: Bytes::new(),
    }
    .encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 6 + 2 + 32 + 3 + 1;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(ReceivedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_failed_message_mints_on_successful_valid_log() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = ReceivedMessage {
        messageHash: B256::ZERO,
        successfulCall: true,
        returnData: Bytes::new(),
    }
    .encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 6 + 2 + 32 + 3 + 1;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(ReceivedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveFailedMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance + U256::from(1_000), new_balance);
}

#[test]
fn test_receive_failed_message_mints_on_successful_retried_failed_log() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = RetriedFailedMessage {
        messageHash: B256::ZERO,
        successfulCall: true,
        returnData: Bytes::new(),
    }
    .encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 6 + 2 + 32 + 3 + 1;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(RetriedFailedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveFailedMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance + U256::from(1_000), new_balance);
}

#[test]
fn test_receive_failed_message_unsuccessful_log_restores_balance() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = ReceivedMessage {
        messageHash: B256::ZERO,
        successfulCall: false,
        returnData: Bytes::new(),
    }
    .encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 6 + 2 + 32 + 3 + 1;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(ReceivedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveFailedMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_failed_message_unsuccessful_retried_failed_log_restores_balance() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let log_data = RetriedFailedMessage {
        messageHash: B256::ZERO,
        successfulCall: false,
        returnData: Bytes::new(),
    }
    .encode_data();

    let mut bytecode = Vec::new();
    const LOG_DATA_OFFSET: usize = 6 + 2 + 32 + 3 + 1;
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::CODECOPY);
    bytecode.push(opcode::PUSH32);
    bytecode.extend_from_slice(RetriedFailedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::LOG1);
    bytecode.push(opcode::STOP);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveFailedMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_failed_message_revert_restores_bridge_balance() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let mut bytecode = Vec::new();
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::PUSH0);
    bytecode.push(opcode::REVERT);
    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveFailedMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_receive_failed_message_fails_when_success_log_missing() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, vec![opcode::STOP]);

    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let input = receiveFailedMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1_000),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        input.into(),
        None,
        None,
    );

    assert!(!result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance, new_balance);
}

#[test]
fn test_non_bridge_call_is_ignored_by_hook() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let other = Address::repeat_byte(0xaa);

    ctx.add_evm_contract(other, vec![opcode::STOP]);

    let input = sendMessageCall {
        to: Address::repeat_byte(0x02),
        message: Bytes::new(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        other,
        input.into(),
        None,
        Some(U256::from(123)),
    );

    assert!(result.is_success());
}

#[test]
fn test_nested_call_inside_bridge_cant_fail_execution() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // Deploy an empty bytecode at zero address (to trigger new frame)
    ctx.add_evm_contract(Address::ZERO, vec![opcode::STOP]);

    let log_data = ReceivedMessage {
        messageHash: B256::ZERO,
        successfulCall: true,
        returnData: Bytes::new(),
    }
    .encode_data();
    let mut bytecode = Vec::new();
    bytecode.push(opcode::PUSH0); // ret length
    bytecode.push(opcode::PUSH0); // ret offset
    bytecode.push(opcode::PUSH0); // args length
    bytecode.push(opcode::PUSH0); // args offset
    bytecode.push(opcode::PUSH0); // value
    bytecode.push(opcode::PUSH0); // addr
    bytecode.push(opcode::GAS); // gas
    bytecode.push(opcode::CALL); // just a new dummy frame before log
    const LOG_DATA_OFFSET: usize = 8 + 6 + 2 + 32 + 3;
    // copy log data (6)
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(log_data.len()).unwrap()); // length
    bytecode.push(opcode::PUSH1);
    bytecode.push(u8::try_from(LOG_DATA_OFFSET).unwrap()); // offset
    bytecode.push(opcode::PUSH0); // data offset
    bytecode.push(opcode::CODECOPY);
    // call log1 (2 + 32 + 3)
    bytecode.push(opcode::PUSH32); // topic
    bytecode.extend_from_slice(ReceivedMessage::SIGNATURE_HASH.as_slice());
    bytecode.push(opcode::PUSH1); // data length
    bytecode.push(u8::try_from(log_data.len()).unwrap());
    bytecode.push(opcode::PUSH0); // data offset
    bytecode.push(opcode::LOG1);
    assert_eq!(bytecode.len(), LOG_DATA_OFFSET);
    bytecode.extend(log_data);
    ctx.add_evm_contract(PRECOMPILE_ROLLUP_BRIDGE, bytecode);

    let receive_message_input = receiveMessageCall {
        from: Address::repeat_byte(0x01),
        to: Address::repeat_byte(0x01),
        value: U256::from(1e9),
        chainId: U256::ONE,
        blockNumber: U256::ZERO,
        messageNonce: U256::ZERO,
        message: Bytes::new(),
    }
    .abi_encode();
    let old_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    let result = ctx.call_evm_tx(
        Address::repeat_byte(0x01),
        PRECOMPILE_ROLLUP_BRIDGE,
        receive_message_input.into(),
        None,
        None,
    );
    assert!(result.is_success());
    let new_balance = ctx.get_balance(PRECOMPILE_ROLLUP_BRIDGE);
    assert_eq!(old_balance + U256::from(1e9), new_balance);
}
