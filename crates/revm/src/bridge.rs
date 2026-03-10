use crate::RwasmFrame;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{derive::derive_keccak256_bytes4, PRECOMPILE_ROLLUP_BRIDGE};
use revm::{
    context::{journaled_state::account::JournaledAccountTr, ContextTr, JournalTr, Transaction},
    handler::{EvmTr, EvmTrError, FrameResult},
    state::EvmState,
};

const SIG_RECEIVE_MESSAGE: [u8; 4] = derive_keccak256_bytes4!(
    "receiveMessage(address,address,uint256,uint256,uint256,uint256,bytes)"
);
const SIG_SEND_MESSAGE: [u8; 4] = derive_keccak256_bytes4!("sendMessage(address,bytes)");

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
    ) external payable;

    function sendMessage(address to, bytes calldata message) external payable;
}

pub(crate) fn apply_bridge_pre_invocation_hook<EVM, ERROR>(evm: &mut EVM) -> Result<(), ERROR>
where
    EVM: EvmTr<Context: ContextTr<Journal: JournalTr<State = EvmState>>, Frame = RwasmFrame>,
    ERROR: EvmTrError<EVM>,
{
    let (_block, tx, _cfg, journal, _chain, _local) = evm.ctx_mut().all_mut();

    // Make sure the recipient and prefix are correct
    let Some(_) = tx.kind().to().filter(|to| **to == PRECOMPILE_ROLLUP_BRIDGE) else {
        return Ok(());
    };
    if !tx.input().starts_with(&SIG_RECEIVE_MESSAGE) {
        return Ok(());
    }

    // Load bridge account and it's balance
    let mut bridge_account = journal
        .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
        .data;

    // Decode ABI input to extract amount to be minted
    let Ok(message) = receiveMessageCall::abi_decode(tx.input().as_ref()) else {
        return Ok(());
    };

    // Mint an extra value for bridge since these funds are required for rollup deposit
    bridge_account.incr_balance(message.value);
    Ok(())
}

pub(crate) fn apply_bridge_post_invocation_hook<EVM, ERROR>(
    evm: &mut EVM,
    frame_result: &FrameResult,
) -> Result<(), ERROR>
where
    EVM: EvmTr<Context: ContextTr<Journal: JournalTr<State = EvmState>>, Frame = RwasmFrame>,
    ERROR: EvmTrError<EVM>,
{
    let (_block, tx, _cfg, journal, _chain, _local) = evm.ctx_mut().all_mut();

    // Make sure this is the message and recipient we're looking for
    let Some(_) = tx.kind().to().filter(|to| **to == PRECOMPILE_ROLLUP_BRIDGE) else {
        return Ok(());
    };
    if !tx.input().starts_with(&SIG_SEND_MESSAGE) {
        return Ok(());
    }

    // let sent_event_exists = journal
    //     .logs()
    //     .iter()
    //     .find(|log| log.topics()[0] == SentMessage::SIGNATURE_HASH)
    //     .is_some();
    // if !sent_event_exists {
    //     return Ok(());
    // }

    // Load bridge account with its balance
    let mut bridge_account = journal
        .load_account_with_code_mut(PRECOMPILE_ROLLUP_BRIDGE)?
        .data;

    // Decode an ABI message (it should be correct)
    let Ok(_message) = sendMessageCall::abi_decode(tx.input().as_ref()) else {
        return Ok(());
    };

    // Burn extra eth for bridge since these funds are required for rollup withdrawal
    if frame_result.interpreter_result().is_ok() {
        bridge_account.decr_balance(tx.value());
    }
    Ok(())
}
