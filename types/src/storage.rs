use crate::Account;
use alloy_primitives::{Address, U256};

pub trait AccountDb {
    fn get_account(&self, address: &Address) -> Option<Account>;

    fn update_account(&mut self, address: &Address, account: &Account);

    fn get_storage(&self, address: &Address, index: &U256) -> Option<U256>;

    fn update_storage(&self, address: &Address, index: &U256, value: &U256);

    fn get_node(&self, key: &[u8]) -> Option<Vec<u8>>;

    fn update_node(&mut self, key: &[u8], value: &Vec<u8>);

    fn get_preimage(&self, key: &[u8]) -> Option<Vec<u8>>;

    fn update_preimage(&mut self, key: &[u8], value: &Vec<u8>);
}
