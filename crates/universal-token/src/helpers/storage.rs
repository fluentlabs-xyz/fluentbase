use crate::consts::{
    SIG_ALLOWANCE, SIG_APPROVE, SIG_BALANCE, SIG_BALANCE_OF, SIG_DECIMALS, SIG_MINT, SIG_NAME,
    SIG_PAUSE, SIG_SYMBOL, SIG_TOTAL_SUPPLY, SIG_TRANSFER, SIG_TRANSFER_FROM, SIG_UNPAUSE,
};
use crate::storage::{allowance_service, balance_service, init_services, settings_service};
use crate::types::input_commands::{
    AllowanceCommand, ApproveCommand, BalanceOfCommand, Encodable, MintCommand, TransferCommand,
    TransferFromCommand,
};
use alloc::vec::Vec;
use fluentbase_sdk::{debug_log, Address, U256};
use hashbrown::HashMap;

pub fn compute_storage_keys(sig: u32, input: &[u8], caller: &Address) -> Vec<U256> {
    let mut keys = Vec::with_capacity(4);
    let (mut s, b, a) = init_services(false);
    match sig {
        SIG_TOTAL_SUPPLY => {
            keys.push(s.total_supply_slot());
        }
        SIG_TRANSFER => {
            let c = TransferCommand::try_decode(input).unwrap();
            keys.push(b.key(caller));
            keys.push(b.key(&c.to));
        }
        SIG_TRANSFER_FROM => {
            let c = TransferFromCommand::try_decode(input).unwrap();
            keys.push(a.key(&c.from, &c.to));
            keys.push(b.key(&c.from));
            keys.push(b.key(&c.to));
        }
        SIG_BALANCE => {
            keys.push(b.key(caller));
        }
        SIG_BALANCE_OF => {
            let c = BalanceOfCommand::try_decode(input).unwrap();
            keys.push(b.key(&c.owner));
        }
        SIG_SYMBOL => {
            keys.push(s.symbol_slot());
        }
        SIG_NAME => {
            keys.push(s.name_slot());
        }
        SIG_DECIMALS => {
            keys.push(s.decimals_slot());
        }
        SIG_ALLOWANCE => {
            let c = AllowanceCommand::try_decode(input).unwrap();
            keys.push(a.key(&c.owner, &c.spender));
        }
        SIG_APPROVE => {
            let c = ApproveCommand::try_decode(input).unwrap();
            keys.push(a.key(&c.owner, &c.spender));
        }
        SIG_MINT => {
            let c = MintCommand::try_decode(input).unwrap();
            keys.push(s.flags_slot());
            keys.push(s.minter_slot());
            keys.push(s.total_supply_slot());
            keys.push(b.key(&c.to));
        }
        SIG_PAUSE => {
            keys.push(s.flags_slot());
            keys.push(s.pauser_slot());
        }
        SIG_UNPAUSE => {
            keys.push(s.flags_slot());
            keys.push(s.pauser_slot());
        }
        _ => {
            debug_log!();
        }
    }
    keys
}
