use fluentbase_sdk::{Bytes, SharedAPI};

pub trait FluentAPI {
    fn exec_evm_tx<SDK: SharedAPI>(&self, rlp_evm_tx: Bytes);
}
