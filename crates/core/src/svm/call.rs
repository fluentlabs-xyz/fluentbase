use fluentbase_sdk::{types::EvmCallMethodOutput, Bytes, ContextReader, SovereignAPI};

pub fn _svm_exec_tx<CR: ContextReader, AM: SovereignAPI>(
    cr: &CR,
    am: &AM,
    solana_raw_tx: Bytes,
) -> () {
    todo!("implement me")
}
