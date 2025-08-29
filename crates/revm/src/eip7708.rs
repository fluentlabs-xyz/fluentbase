use fluentbase_sdk::{Address, B256, NATIVE_TRANSFER_KECCAK, SYSTEM_ADDRESS, U256};
use revm::context::{ContextTr, JournalTr};
use revm::primitives::{Log, LogData};

pub(crate) fn emit_native_transfer_log<CTX: ContextTr>(
    ctx: &mut CTX,
    caller: Address,
    callee: Address,
    transfer_value: U256,
) {
    let transfer_value: B256 = transfer_value.into();
    let log = Log {
        address: SYSTEM_ADDRESS,
        data: LogData::new_unchecked(
            vec![
                NATIVE_TRANSFER_KECCAK,
                caller.into_word(),
                callee.into_word(),
            ],
            transfer_value.into(),
        ),
    };
    ctx.journal_mut().log(log);
}
