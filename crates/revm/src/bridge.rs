use crate::RwasmFrame;
use alloy_primitives::address;
use alloy_sol_types::{sol, SolCall, SolEvent};
use fluentbase_evm::{InterpreterAction, InterpreterResult};
use fluentbase_sdk::{Bytes, PRECOMPILE_ROLLUP_BRIDGE, U256};
use revm::{
    context::{
        journaled_state::account::JournaledAccountTr, ContextError, ContextTr, JournalTr,
        Transaction,
    },
    interpreter::{CallInputs, Gas, InstructionResult},
    Database,
};
use tracing::warn;

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

    event SentMessage(
        address indexed sender,
        address indexed to,
        uint256 value,
        uint256 fee,
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
    ) external;

    function receiveFailedMessage(
        address from,
        address to,
        uint256 value,
        uint256 chainId,
        uint256 blockNumber,
        uint256 messageNonce,
        bytes calldata message
    ) external;

    function sendMessage(address to, bytes calldata message) external payable;
}

pub(crate) fn apply_bridge_pre_invocation_hook<CTX: ContextTr>(
    inputs: &CallInputs,
    ctx: &mut CTX,
) -> Result<(), ContextError<<CTX::Db as Database>::Error>> {
    // Make sure the recipient and prefix are correct
    if inputs.target_address != PRECOMPILE_ROLLUP_BRIDGE {
        return Ok(());
    }

    let input = inputs.input.bytes(ctx);

    // Mint an extra value for bridge since these funds are required for rollup receive/re-execution
    if let Some(message_value) = try_decode_receive_message_value(input) {
        // Load bridge account and it's balance
        let mut bridge_account = ctx
            .journal_mut()
            .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
            .data;

        if !bridge_account.incr_balance(message_value) {
            let bridge_balance = bridge_account.balance();
            warn!(
                %bridge_balance,
                value = %message_value,
                "Failed to increase bridge balance on receive/receiveFailed message"
            );
            return Err(ContextError::Custom(
                "bridge pre-hook: failed to increase bridge balance".to_string(),
            ));
        }
    }

    Ok(())
}

pub(crate) fn apply_bridge_post_invocation_hook<CTX: ContextTr>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    next_action: &mut InterpreterAction,
) -> Result<(), ContextError<<CTX::Db as Database>::Error>> {
    // A special case for corrupted bridge transaction on testnet, where we issued 3
    // transactions from relayer that caused execution failure. We keep it here only to
    // make blockchain syncable for testnet.
    //
    // Note: it can be removed once we have new snapshot for testnet
    if ctx.tx().chain_id() == Some(0x5202)
        && ctx.tx().caller() == address!("0x1C92DffBCe76670F69007F22A54e31ff3Ab45d5E")
        && [537u64, 538u64, 539u64].contains(&ctx.tx().nonce())
        && ctx.journal().depth() == 3
    {
        *next_action = malformed_interpreter_action();
        return Ok(());
    }

    // Proceed post-invocation hook only if frame is closed
    match next_action {
        InterpreterAction::Return(_) => {}
        _ => return Ok(()),
    };

    // Make sure this is the message and recipient we're looking for
    if frame.interpreter.input.target_address != PRECOMPILE_ROLLUP_BRIDGE {
        return Ok(());
    }

    // Don't proceed if it's new frame creation (technically not possible, but just in case)
    let Some(instruction_result) = next_action.instruction_result() else {
        return Ok(());
    };

    let input = frame.interpreter.input.input.bytes(ctx);

    if let Some(message_value) = try_decode_receive_message_value(&input) {
        let receive_message_logs = ctx
            .journal()
            .logs()
            .iter()
            .filter_map(|log| {
                // Make sure event and emitter are correct
                if log.address != PRECOMPILE_ROLLUP_BRIDGE {
                    return None;
                }
                // If input data can't be decoded
                if let Ok(received_message) = ReceivedMessage::decode_log(log) {
                    Some(received_message.successfulCall)
                } else if let Ok(received_message) = RetriedFailedMessage::decode_log(log) {
                    Some(received_message.successfulCall)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // We must have exactly one event
        let was_call_successful = if instruction_result.is_ok() {
            if receive_message_logs.len() != 1 {
                warn!(
                    num_logs = receive_message_logs.len(),
                    "Received multiple receive message events (must be exactly 1)"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
            // We count call as successful only if it executes w/o error and we have correct log
            receive_message_logs.first().copied().unwrap()
        } else {
            // Defensive guard: failed calls must not emit persistent bridge message logs.
            if !receive_message_logs.is_empty() {
                warn!(
                    num_logs = receive_message_logs.len(),
                    "Found receive message events on failed transaction"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
            false
        };

        // Load bridge account with its balance
        let mut bridge_account = ctx
            .journal_mut()
            .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
            .data;

        // Decrease balance back if execution failed
        if !was_call_successful && !bridge_account.decr_balance(message_value) {
            let bridge_balance = bridge_account.balance();
            warn!(
                %bridge_balance,
                value = %message_value,
                "Failed to decrease bridge balance on receive/receiveFailed message"
            );
            *next_action = malformed_interpreter_action();
            return Ok(());
        }
    } else if try_decode_send_message_value(input).is_some() {
        let send_message_logs = ctx
            .journal()
            .logs()
            .iter()
            .filter_map(|log| {
                // Make sure event and emitter are correct
                if log.address != PRECOMPILE_ROLLUP_BRIDGE {
                    return None;
                }
                // If input data can't be decoded
                if let Ok(sent_message) = SentMessage_0::decode_log(log) {
                    Some((sent_message.value, U256::ZERO))
                } else if let Ok(sent_message) = SentMessage_1::decode_log(log) {
                    if sent_message.value < sent_message.fee {
                        warn!("Malfunctioning bridge emitted event where fee greater than total amount: value={}, fee={}", sent_message.value, sent_message.fee);
                        return None;
                    }
                    Some((sent_message.value - sent_message.fee, sent_message.fee))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // We must have exactly one event
        let amount_to_be_burned = if instruction_result.is_ok() {
            if send_message_logs.len() != 1 {
                warn!(
                    num_logs = send_message_logs.len(),
                    "Received multiple sent message events (must be exactly 1)"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
            // We count call as successful only if it executes w/o error and we have correct log
            Some(send_message_logs.first().copied().unwrap())
        } else {
            // Defensive guard: failed calls must not emit persistent bridge message logs.
            if !send_message_logs.is_empty() {
                warn!(
                    num_logs = send_message_logs.len(),
                    "Found sent message events on failed transaction"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
            None
        };

        // Load bridge account with its balance
        let mut bridge_account = ctx
            .journal_mut()
            .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
            .data;

        let msg_value = frame.interpreter.input.call_value;

        // Burn extra eth for bridge since these funds are required for rollup withdrawal
        if let Some((amount_to_be_burned, relayer_fee)) = amount_to_be_burned {
            if amount_to_be_burned + relayer_fee != msg_value {
                warn!(
                    %msg_value,
                    value = %amount_to_be_burned,
                    "Amount to be burned doesn't match passed message value"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
            if !bridge_account.decr_balance(amount_to_be_burned) {
                let bridge_balance = bridge_account.balance();
                warn!(
                    %bridge_balance,
                    value = %amount_to_be_burned,
                    "Failed to decrease bridge balance on sent message"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
        }
    }

    Ok(())
}

fn try_decode_receive_message_value<T: AsRef<[u8]>>(input: T) -> Option<U256> {
    let input = input.as_ref();

    if input.starts_with(&receiveMessageCall::SELECTOR) {
        let Ok(message) = receiveMessageCall::abi_decode(input) else {
            return None;
        };
        return Some(message.value);
    }

    if input.starts_with(&receiveFailedMessageCall::SELECTOR) {
        let Ok(message) = receiveFailedMessageCall::abi_decode(input) else {
            return None;
        };
        return Some(message.value);
    }

    None
}

fn try_decode_send_message_value<T: AsRef<[u8]>>(input: T) -> Option<()> {
    let input = input.as_ref();

    if input.starts_with(&sendMessageCall::SELECTOR) {
        // Decode an ABI message (it should be correct)
        if let Ok(_message) = sendMessageCall::abi_decode(input) {
            return Some(());
        };
    }

    None
}

fn malformed_interpreter_action() -> InterpreterAction {
    InterpreterAction::Return(InterpreterResult::new(
        InstructionResult::MalformedBuiltinParams,
        Bytes::new(),
        Gas::new(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Log, B256};
    use revm::{
        context::{
            journaled_state::account::JournaledAccountTr, BlockEnv, CfgEnv, JournalTr, TxEnv,
        },
        database::InMemoryDB,
        interpreter::{CallInput, CallScheme, CallValue},
    };

    fn new_ctx() -> crate::RwasmContext<InMemoryDB> {
        let db = InMemoryDB::default();
        let mut ctx: crate::RwasmContext<InMemoryDB> =
            crate::RwasmContext::new(db, crate::RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(crate::RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        ctx
    }

    fn set_bridge_balance(ctx: &mut crate::RwasmContext<InMemoryDB>, balance: U256) {
        let mut bridge = ctx
            .journal_mut()
            .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)
            .unwrap();
        bridge.set_balance(balance);
    }

    fn bridge_balance(ctx: &mut crate::RwasmContext<InMemoryDB>) -> U256 {
        let bridge = ctx
            .journal_mut()
            .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)
            .unwrap();
        *bridge.balance()
    }

    fn receive_message_input(value: U256) -> Bytes {
        receiveMessageCall {
            from: address!("0x1000000000000000000000000000000000000001"),
            to: address!("0x2000000000000000000000000000000000000002"),
            value,
            chainId: U256::from(1),
            blockNumber: U256::from(10),
            messageNonce: U256::from(7),
            message: vec![0xAA, 0xBB].into(),
        }
        .abi_encode()
        .into()
    }

    fn receive_failed_message_input(value: U256) -> Bytes {
        receiveFailedMessageCall {
            from: address!("0x1000000000000000000000000000000000000001"),
            to: address!("0x2000000000000000000000000000000000000002"),
            value,
            chainId: U256::from(1),
            blockNumber: U256::from(10),
            messageNonce: U256::from(9),
            message: vec![0xCC].into(),
        }
        .abi_encode()
        .into()
    }

    fn send_message_input() -> Bytes {
        sendMessageCall {
            to: address!("0x3000000000000000000000000000000000000003"),
            message: vec![0x11, 0x22].into(),
        }
        .abi_encode()
        .into()
    }

    fn make_call_inputs(target: Address, input: Bytes, value: U256) -> CallInputs {
        CallInputs {
            input: CallInput::Bytes(input),
            return_memory_offset: Default::default(),
            gas_limit: 1_000_000,
            bytecode_address: target,
            known_bytecode: None,
            target_address: target,
            caller: address!("0x4000000000000000000000000000000000000004"),
            value: CallValue::Transfer(value),
            scheme: CallScheme::Call,
            is_static: false,
        }
    }

    fn make_bridge_frame(input: Bytes, msg_value: U256) -> crate::RwasmFrame {
        let mut frame = crate::RwasmFrame::default();
        frame.interpreter.input.target_address = PRECOMPILE_ROLLUP_BRIDGE;
        frame.interpreter.input.input = CallInput::Bytes(input);
        frame.interpreter.input.call_value = msg_value;
        frame
    }

    fn ok_action() -> InterpreterAction {
        InterpreterAction::Return(InterpreterResult::new(
            InstructionResult::Return,
            Bytes::new(),
            Gas::new(100_000),
        ))
    }

    fn revert_action() -> InterpreterAction {
        InterpreterAction::Return(InterpreterResult::new(
            InstructionResult::Revert,
            Bytes::new(),
            Gas::new(100_000),
        ))
    }

    fn assert_malformed(action: &InterpreterAction) {
        assert_eq!(
            action.instruction_result(),
            Some(InstructionResult::MalformedBuiltinParams)
        );
    }

    fn push_received_message_log(ctx: &mut crate::RwasmContext<InMemoryDB>, successful_call: bool) {
        let event = ReceivedMessage {
            messageHash: B256::ZERO,
            successfulCall: successful_call,
            returnData: Bytes::new(),
        };
        ctx.journal_mut().log(Log {
            address: PRECOMPILE_ROLLUP_BRIDGE,
            data: event.encode_log_data(),
        });
    }

    fn push_retried_failed_message_log(
        ctx: &mut crate::RwasmContext<InMemoryDB>,
        successful_call: bool,
    ) {
        let event = RetriedFailedMessage {
            messageHash: B256::ZERO,
            successfulCall: successful_call,
            returnData: Bytes::new(),
        };
        ctx.journal_mut().log(Log {
            address: PRECOMPILE_ROLLUP_BRIDGE,
            data: event.encode_log_data(),
        });
    }

    fn push_sent_message_log(ctx: &mut crate::RwasmContext<InMemoryDB>, value: U256) {
        let event = SentMessage_0 {
            sender: address!("0x4000000000000000000000000000000000000004"),
            to: address!("0x3000000000000000000000000000000000000003"),
            value,
            chainId: U256::from(1),
            blockNumber: U256::from(10),
            nonce: U256::from(1),
            messageHash: B256::ZERO,
            data: vec![0xAB].into(),
        };
        ctx.journal_mut().log(Log {
            address: PRECOMPILE_ROLLUP_BRIDGE,
            data: event.encode_log_data(),
        });
    }

    #[test]
    fn pre_hook_mints_on_receive_message() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(10));

        let inputs = make_call_inputs(
            PRECOMPILE_ROLLUP_BRIDGE,
            receive_message_input(U256::from(7)),
            U256::ZERO,
        );
        apply_bridge_pre_invocation_hook(&inputs, &mut ctx).unwrap();

        assert_eq!(bridge_balance(&mut ctx), U256::from(17));
    }

    #[test]
    fn pre_hook_mints_on_receive_failed_message() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(2));

        let inputs = make_call_inputs(
            PRECOMPILE_ROLLUP_BRIDGE,
            receive_failed_message_input(U256::from(5)),
            U256::ZERO,
        );
        apply_bridge_pre_invocation_hook(&inputs, &mut ctx).unwrap();

        assert_eq!(bridge_balance(&mut ctx), U256::from(7));
    }

    #[test]
    fn pre_hook_ignores_non_bridge_target() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(9));

        let inputs = make_call_inputs(
            address!("0x5000000000000000000000000000000000000005"),
            receive_message_input(U256::from(7)),
            U256::ZERO,
        );
        apply_bridge_pre_invocation_hook(&inputs, &mut ctx).unwrap();

        assert_eq!(bridge_balance(&mut ctx), U256::from(9));
    }

    #[test]
    fn pre_hook_ignores_malformed_receive_payload() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(9));

        let mut malformed = Vec::from(receiveMessageCall::SELECTOR);
        malformed.extend_from_slice(&[0x01, 0x02]);

        let inputs = make_call_inputs(PRECOMPILE_ROLLUP_BRIDGE, malformed.into(), U256::ZERO);
        apply_bridge_pre_invocation_hook(&inputs, &mut ctx).unwrap();

        assert_eq!(bridge_balance(&mut ctx), U256::from(9));
    }

    #[test]
    fn pre_hook_fails_on_balance_overflow() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::MAX);

        let inputs = make_call_inputs(
            PRECOMPILE_ROLLUP_BRIDGE,
            receive_message_input(U256::from(1)),
            U256::ZERO,
        );

        let err = apply_bridge_pre_invocation_hook(&inputs, &mut ctx).unwrap_err();

        assert!(matches!(err, ContextError::Custom(_)));
        assert_eq!(bridge_balance(&mut ctx), U256::MAX);
    }

    #[test]
    fn post_hook_short_circuits_known_corrupted_testnet_tx() {
        let mut ctx = new_ctx();
        ctx.tx.chain_id = Some(0x5202);
        ctx.tx.caller = address!("0x1C92DffBCe76670F69007F22A54e31ff3Ab45d5E");
        ctx.tx.nonce = 537;

        let _ = ctx.journal_mut().checkpoint();
        let _ = ctx.journal_mut().checkpoint();
        let _ = ctx.journal_mut().checkpoint();

        let mut frame = make_bridge_frame(receive_message_input(U256::from(1)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();
        assert_malformed(&next_action);
    }

    #[test]
    fn post_hook_receive_success_keeps_balance() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(30));
        push_received_message_log(&mut ctx, true);

        let mut frame = make_bridge_frame(receive_message_input(U256::from(7)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_eq!(
            next_action.instruction_result(),
            Some(InstructionResult::Return)
        );
        assert_eq!(bridge_balance(&mut ctx), U256::from(30));
    }

    #[test]
    fn post_hook_receive_failed_decreases_balance() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(30));
        push_received_message_log(&mut ctx, false);

        let mut frame = make_bridge_frame(receive_message_input(U256::from(7)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_eq!(bridge_balance(&mut ctx), U256::from(23));
    }

    #[test]
    fn post_hook_accepts_retried_failed_message_event() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(25));
        push_retried_failed_message_log(&mut ctx, false);

        let mut frame = make_bridge_frame(receive_failed_message_input(U256::from(5)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_eq!(bridge_balance(&mut ctx), U256::from(20));
    }

    #[test]
    fn post_hook_receive_with_missing_event_is_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(20));

        let mut frame = make_bridge_frame(receive_message_input(U256::from(5)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(20));
    }

    #[test]
    fn post_hook_receive_event_from_non_bridge_address_is_ignored_and_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(20));

        let event = ReceivedMessage {
            messageHash: B256::ZERO,
            successfulCall: true,
            returnData: Bytes::new(),
        };
        ctx.journal_mut().log(Log {
            address: address!("0x7000000000000000000000000000000000000007"),
            data: event.encode_log_data(),
        });

        let mut frame = make_bridge_frame(receive_message_input(U256::from(5)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(20));
    }

    #[test]
    fn post_hook_receive_with_multiple_events_is_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(20));
        push_received_message_log(&mut ctx, true);
        push_retried_failed_message_log(&mut ctx, true);

        let mut frame = make_bridge_frame(receive_message_input(U256::from(5)), U256::ZERO);
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(20));
    }

    #[test]
    fn post_hook_send_message_decreases_balance_when_log_value_matches() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(100));
        push_sent_message_log(&mut ctx, U256::from(11));

        let mut frame = make_bridge_frame(send_message_input(), U256::from(11));
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_eq!(
            next_action.instruction_result(),
            Some(InstructionResult::Return)
        );
        assert_eq!(bridge_balance(&mut ctx), U256::from(89));
    }

    #[test]
    fn post_hook_send_message_with_mismatch_value_is_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(100));
        push_sent_message_log(&mut ctx, U256::from(12));

        let mut frame = make_bridge_frame(send_message_input(), U256::from(11));
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(100));
    }

    #[test]
    fn post_hook_send_message_with_missing_event_is_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(100));

        let mut frame = make_bridge_frame(send_message_input(), U256::from(11));
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(100));
    }

    #[test]
    fn post_hook_send_event_from_non_bridge_address_is_ignored_and_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(100));

        let event = SentMessage_0 {
            sender: address!("0x4000000000000000000000000000000000000004"),
            to: address!("0x3000000000000000000000000000000000000003"),
            value: U256::from(11),
            chainId: U256::from(1),
            blockNumber: U256::from(10),
            nonce: U256::from(1),
            messageHash: B256::ZERO,
            data: vec![0xAB].into(),
        };
        ctx.journal_mut().log(Log {
            address: address!("0x7000000000000000000000000000000000000007"),
            data: event.encode_log_data(),
        });

        let mut frame = make_bridge_frame(send_message_input(), U256::from(11));
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(100));
    }

    #[test]
    fn post_hook_send_message_with_insufficient_bridge_balance_is_malformed() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(5));
        push_sent_message_log(&mut ctx, U256::from(11));

        let mut frame = make_bridge_frame(send_message_input(), U256::from(11));
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
        assert_eq!(bridge_balance(&mut ctx), U256::from(5));
    }

    #[test]
    fn post_hook_ignores_non_return_action() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(10));
        push_received_message_log(&mut ctx, false);

        let mut frame = make_bridge_frame(receive_message_input(U256::from(7)), U256::ZERO);
        let mut next_action = InterpreterAction::SystemInterruption;

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert!(matches!(next_action, InterpreterAction::SystemInterruption));
        assert_eq!(bridge_balance(&mut ctx), U256::from(10));
    }

    #[test]
    fn post_hook_ignores_non_bridge_target() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(10));
        push_received_message_log(&mut ctx, false);

        let mut frame = make_bridge_frame(receive_message_input(U256::from(7)), U256::ZERO);
        frame.interpreter.input.target_address =
            address!("0x6000000000000000000000000000000000000006");
        let mut next_action = ok_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_eq!(
            next_action.instruction_result(),
            Some(InstructionResult::Return)
        );
        assert_eq!(bridge_balance(&mut ctx), U256::from(10));
    }

    #[test]
    fn post_hook_marks_malformed_if_receive_logs_exist_on_failed_tx() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(20));
        push_received_message_log(&mut ctx, true);

        let mut frame = make_bridge_frame(receive_message_input(U256::from(5)), U256::ZERO);
        let mut next_action = revert_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
    }

    #[test]
    fn post_hook_marks_malformed_if_send_logs_exist_on_failed_tx() {
        let mut ctx = new_ctx();
        set_bridge_balance(&mut ctx, U256::from(20));
        push_sent_message_log(&mut ctx, U256::from(5));

        let mut frame = make_bridge_frame(send_message_input(), U256::from(5));
        let mut next_action = revert_action();

        apply_bridge_post_invocation_hook(&mut frame, &mut ctx, &mut next_action).unwrap();

        assert_malformed(&next_action);
    }
}
