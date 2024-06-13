use fluentbase_sdk::{
    types::{EvmCallMethodInput, EvmCallMethodOutput},
    AccountManager,
    ContextReader,
};

pub fn _svm_call<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    todo!("implement me")
}
