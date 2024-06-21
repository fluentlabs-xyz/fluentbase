use fluentbase_sdk::{
    basic_entrypoint,
    derive::Contract,
    AccountManager,
    ContextReader,
    SharedAPI,
};

#[derive(Contract)]
pub struct LOADER<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> LOADER<'a, CR, AM> {
    pub fn deploy<SDK: SharedAPI>(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main<SDK: SharedAPI>(&self) {}
}

basic_entrypoint!(
    LOADER<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
