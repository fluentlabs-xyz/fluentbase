use crate::consts::{
    SIG_ALLOWANCE, SIG_APPROVE, SIG_BALANCE, SIG_BALANCE_OF, SIG_DECIMALS, SIG_MINT, SIG_NAME,
    SIG_PAUSE, SIG_SYMBOL, SIG_TOTAL_SUPPLY, SIG_TRANSFER, SIG_TRANSFER_FROM, SIG_UNPAUSE,
};
use crate::storage::{init_services, Feature, InitialSettings};
use crate::types::input_commands::{
    AllowanceCommand, ApproveCommand, BalanceOfCommand, Encodable, MintCommand, TransferCommand,
    TransferFromCommand,
};
use alloc::vec::Vec;
use fluentbase_sdk::{Address, U256};

pub fn compute_storage_keys(sig: u32, input: &[u8], caller: &Address) -> Vec<U256> {
    let mut keys = Vec::new();
    let (mut s, b, a) = init_services();
    match sig {
        SIG_TOTAL_SUPPLY => {
            keys.reserve_exact(1);
            keys.push(s.total_supply_slot());
            keys
        }
        SIG_TRANSFER => {
            keys.reserve_exact(2);
            let c = TransferCommand::try_decode(input).unwrap();
            keys.push(b.key(caller));
            keys.push(b.key(&c.to));
            keys
        }
        SIG_TRANSFER_FROM => {
            keys.reserve_exact(3);
            let c = TransferFromCommand::try_decode(input).unwrap();
            keys.push(a.key(&c.from, &c.to));
            keys.push(b.key(&c.from));
            keys.push(b.key(&c.to));
            keys
        }
        SIG_BALANCE => {
            keys.reserve_exact(1);
            keys.push(b.key(caller));
            keys
        }
        SIG_BALANCE_OF => {
            keys.reserve_exact(1);
            let c = BalanceOfCommand::try_decode(input).unwrap();
            keys.push(b.key(&c.owner));
            keys
        }
        SIG_SYMBOL => {
            keys.reserve_exact(1);
            keys.push(s.symbol_slot());
            keys
        }
        SIG_NAME => {
            keys.reserve_exact(1);
            keys.push(s.name_slot());
            keys
        }
        SIG_DECIMALS => {
            keys.reserve_exact(1);
            keys.push(s.decimals_slot());
            keys
        }
        SIG_ALLOWANCE => {
            keys.reserve_exact(1);
            let c = AllowanceCommand::try_decode(input).unwrap();
            keys.push(a.key(&c.owner, &c.spender));
            keys
        }
        SIG_APPROVE => {
            keys.reserve_exact(1);
            let c = ApproveCommand::try_decode(input).unwrap();
            keys.push(a.key(&c.owner, &c.spender));
            keys
        }
        SIG_MINT => {
            keys.reserve_exact(4);
            let c = MintCommand::try_decode(input).unwrap();
            keys.push(s.flags_slot());
            keys.push(s.minter_slot());
            keys.push(s.total_supply_slot());
            keys.push(b.key(&c.to));
            keys
        }
        SIG_PAUSE => {
            keys.reserve_exact(2);
            keys.push(s.flags_slot());
            keys.push(s.pauser_slot());
            keys
        }
        SIG_UNPAUSE => {
            keys.reserve_exact(2);
            keys.push(s.flags_slot());
            keys.push(s.pauser_slot());
            keys
        }
        _ => {
            // for deploy
            keys.reserve_exact(2);
            let (initial_settings, _) = InitialSettings::try_decode_from_slice(&input).unwrap();
            for f in initial_settings.features() {
                match f {
                    Feature::Meta { name, symbol } => {}
                    Feature::InitialSupply {
                        amount,
                        owner,
                        decimals,
                    } => {
                        keys.push(b.key(&Address::from_slice(owner)));
                    }
                    Feature::Mintable { minter } => {
                        keys.push(s.flags_slot());
                        keys.push(s.minter_slot());
                        keys.push(s.total_supply_slot());
                    }
                    Feature::Pausable { pauser } => {
                        keys.push(s.flags_slot());
                        keys.push(s.pauser_slot());
                    }
                }
            }
            keys
        }
    }
}
