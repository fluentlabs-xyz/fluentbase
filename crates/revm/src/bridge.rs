use alloy_sol_types::{sol, SolCall, SolEvent};
use fluentbase_evm::{InterpreterAction, InterpreterResult};
use fluentbase_sdk::{Bytes, U256, PRECOMPILE_ROLLUP_BRIDGE};
use revm::{
    context::{
        journaled_state::account::JournaledAccountTr, ContextError, ContextTr, JournalTr,
        Transaction,
    },
    interpreter::{Gas, InstructionResult},
    Database,
};
use tracing::warn;

sol! {
    event ReceivedMessage(bytes32 messageHash, bool successfulCall, bytes returnData);

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
    ctx: &mut CTX,
) -> Result<(), ContextError<<CTX::Db as Database>::Error>> {
    let (tx, journal) = ctx.tx_journal_mut();

    // Make sure the recipient and prefix are correct
    let Some(_) = tx.kind().to().filter(|to| **to == PRECOMPILE_ROLLUP_BRIDGE) else {
        return Ok(());
    };

    // Load bridge account and it's balance
    let mut bridge_account = journal
        .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
        .data;

    // Mint an extra value for bridge since these funds are required for rollup receive/re-execution
    if let Some(message_value) = decode_receive_message_value(tx.input().as_ref()) {
        // Note: overflow here can't happen, we can't have more than 2**256 on bridge balance
        _ = bridge_account.incr_balance(message_value)
    }
    Ok(())
}

pub(crate) fn apply_bridge_post_invocation_hook<CTX: ContextTr>(
    ctx: &mut CTX,
    next_action: &mut InterpreterAction,
) -> Result<(), ContextError<<CTX::Db as Database>::Error>> {
    let (tx, journal) = ctx.tx_journal_mut();

    // Make sure this is the message and recipient we're looking for
    let Some(_) = tx.kind().to().filter(|to| **to == PRECOMPILE_ROLLUP_BRIDGE) else {
        return Ok(());
    };

    // Don't proceed if it's new frame creation (technically not possible, but just in case)
    let Some(instruction_result) = next_action.instruction_result() else {
        return Ok(());
    };

    if let Some(message_value) = decode_receive_message_value(tx.input().as_ref()) {
        let receive_message_logs = journal
            .logs()
            .iter()
            .filter_map(|log| {
                // Make sure event and emitter are correct
                if log.address != PRECOMPILE_ROLLUP_BRIDGE {
                    return None;
                }
                // If input data can't be decoded
                let Ok(received_message) = ReceivedMessage::decode_log(log) else {
                    return None;
                };
                Some(received_message)
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
            receive_message_logs.first().unwrap().successfulCall
        } else {
            // It can't happen, better to just terminate execution here
            assert!(
                receive_message_logs.is_empty(),
                "revm: found non-zero receive message logs on failed transaction, it can't happen"
            );
            false
        };

        // Load bridge account with its balance
        let mut bridge_account = journal
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
    } else if tx.input().starts_with(&sendMessageCall::SELECTOR) {
        // Decode an ABI message (it should be correct)
        let Ok(_message) = sendMessageCall::abi_decode(tx.input().as_ref()) else {
            return Ok(());
        };

        let send_message_logs = journal
            .logs()
            .iter()
            .filter_map(|log| {
                // Make sure event and emitter are correct
                if log.address != PRECOMPILE_ROLLUP_BRIDGE {
                    return None;
                }
                // If input data can't be decoded
                let Ok(sent_message) = SentMessage::decode_log(log) else {
                    return None;
                };
                Some(sent_message)
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
            Some(send_message_logs.first().unwrap().value)
        } else {
            // It can't happen, better to just terminate execution here
            assert!(
                send_message_logs.is_empty(),
                "revm: found non-zero sent message logs on failed transaction, it can't happen"
            );
            None
        };

        // Load bridge account with its balance
        let mut bridge_account = journal
            .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
            .data;

        // Burn extra eth for bridge since these funds are required for rollup withdrawal
        if let Some(amount_to_be_burned) = amount_to_be_burned {
            if amount_to_be_burned != tx.value() {
                let tx_value = tx.value();
                warn!(
                    %tx_value,
                    value = %amount_to_be_burned,
                    "Amount to be burned doesn't match passed transaction value"
                );
                *next_action = malformed_interpreter_action();
                return Ok(());
            }
            if !bridge_account.decr_balance(tx.value()) {
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

fn decode_receive_message_value(input: &[u8]) -> Option<U256> {
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

fn malformed_interpreter_action() -> InterpreterAction {
    InterpreterAction::Return(InterpreterResult::new(
        InstructionResult::MalformedBuiltinParams,
        Bytes::new(),
        Gas::new(0),
    ))
}
