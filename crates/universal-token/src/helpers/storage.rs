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
    let (mut settings, balance, allowance) = init_services();
    match sig {
        SIG_TOTAL_SUPPLY => {
            keys.reserve_exact(1);
            keys.push(settings.total_supply_slot());
        }
        SIG_TRANSFER => {
            keys.reserve_exact(2);
            let c = TransferCommand::try_decode(input).unwrap();
            keys.push(balance.key(caller));
            keys.push(balance.key(&c.to));
        }
        SIG_TRANSFER_FROM => {
            keys.reserve_exact(3);
            let c = TransferFromCommand::try_decode(input).unwrap();
            keys.push(allowance.key(&c.from, &c.to));
            keys.push(balance.key(&c.from));
            keys.push(balance.key(&c.to));
        }
        SIG_BALANCE => {
            keys.reserve_exact(1);
            keys.push(balance.key(caller));
        }
        SIG_BALANCE_OF => {
            keys.reserve_exact(1);
            let c = BalanceOfCommand::try_decode(input).unwrap();
            keys.push(balance.key(&c.owner));
        }
        SIG_SYMBOL => {
            keys.reserve_exact(1);
            keys.push(settings.symbol_slot());
        }
        SIG_NAME => {
            keys.reserve_exact(1);
            keys.push(settings.name_slot());
        }
        SIG_DECIMALS => {
            keys.reserve_exact(1);
            keys.push(settings.decimals_slot());
        }
        SIG_ALLOWANCE => {
            keys.reserve_exact(1);
            let c = AllowanceCommand::try_decode(input).unwrap();
            keys.push(allowance.key(&c.owner, &c.spender));
        }
        SIG_APPROVE => {
            keys.reserve_exact(1);
            let c = ApproveCommand::try_decode(input).unwrap();
            keys.push(allowance.key(&c.owner, &c.spender));
        }
        SIG_MINT => {
            keys.reserve_exact(4);
            let c = MintCommand::try_decode(input).unwrap();
            keys.push(settings.flags_slot());
            keys.push(settings.minter_slot());
            keys.push(settings.total_supply_slot());
            keys.push(balance.key(&c.to));
        }
        SIG_PAUSE => {
            keys.reserve_exact(2);
            keys.push(settings.flags_slot());
            keys.push(settings.pauser_slot());
        }
        SIG_UNPAUSE => {
            keys.reserve_exact(2);
            keys.push(settings.flags_slot());
            keys.push(settings.pauser_slot());
        }
        _ => {
            keys.reserve_exact(3);
            let (initial_settings, _) = InitialSettings::try_decode_from_slice(&input).unwrap();
            for f in initial_settings.features() {
                match f {
                    Feature::Meta { .. } => {
                        keys.push(settings.name_slot());
                        keys.push(settings.symbol_slot());
                    }
                    Feature::InitialSupply { owner, .. } => {
                        keys.push(balance.key(&Address::from_slice(owner)));
                    }
                    Feature::Mintable { .. } => {
                        keys.push(settings.flags_slot());
                        keys.push(settings.minter_slot());
                        keys.push(settings.total_supply_slot());
                    }
                    Feature::Pausable { .. } => {
                        keys.push(settings.flags_slot());
                        keys.push(settings.pauser_slot());
                    }
                }
            }
        }
    }
    keys
}
