use fluentbase_core::Account;
use fluentbase_genesis::Genesis;
use fluentbase_types::Address;
use std::collections::HashMap;

struct TestingContext {
    genesis: Genesis,
    accounts: HashMap<Address, Account>,
}

impl TestingContext {
    fn load_from_genesis(genesis: Genesis) {
        genesis.alloc;
    }
}
