use fluentbase_sdk::{
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
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

pub fn _svm_create<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCreateMethodInput,
) -> EvmCreateMethodOutput {
    todo!("implement me")
}
