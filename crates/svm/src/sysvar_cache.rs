use crate::{
    clock::Clock,
    epoch_rewards::EpochRewards,
    epoch_schedule::EpochSchedule,
    pubkey::Pubkey,
    rent::Rent,
    solana_program::sysvar::{clock, epoch_rewards, epoch_schedule, rent},
};
use alloc::sync::Arc;
use solana_bincode::deserialize;
use solana_instruction::error::InstructionError;

#[derive(Default, Clone, Debug)]
pub struct SysvarCache {
    clock: Option<Arc<Clock>>,
    epoch_schedule: Option<Arc<EpochSchedule>>,
    epoch_rewards: Option<Arc<EpochRewards>>,
    rent: Option<Arc<Rent>>,
}

impl SysvarCache {
    pub fn get_clock(&self) -> Result<Arc<Clock>, InstructionError> {
        self.clock
            .clone()
            .ok_or(InstructionError::UnsupportedSysvar)
    }

    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = Some(Arc::new(clock));
    }

    pub fn get_epoch_schedule(&self) -> Result<Arc<EpochSchedule>, InstructionError> {
        self.epoch_schedule
            .clone()
            .ok_or(InstructionError::UnsupportedSysvar)
    }

    pub fn set_epoch_schedule(&mut self, epoch_schedule: EpochSchedule) {
        self.epoch_schedule = Some(Arc::new(epoch_schedule));
    }

    pub fn get_epoch_rewards(&self) -> Result<Arc<EpochRewards>, InstructionError> {
        self.epoch_rewards
            .clone()
            .ok_or(InstructionError::UnsupportedSysvar)
    }

    pub fn set_epoch_rewards(&mut self, epoch_rewards: EpochRewards) {
        self.epoch_rewards = Some(Arc::new(epoch_rewards));
    }

    pub fn get_rent(&self) -> Result<Arc<Rent>, InstructionError> {
        self.rent.clone().ok_or(InstructionError::UnsupportedSysvar)
    }

    pub fn set_rent(&mut self, rent: Rent) {
        self.rent = Some(Arc::new(rent));
    }

    pub fn fill_missing_entries<F: FnMut(&Pubkey, &mut dyn FnMut(&[u8]))>(
        &mut self,
        mut get_account_data: F,
    ) {
        if self.clock.is_none() {
            get_account_data(&clock::id(), &mut |data: &[u8]| {
                if let Ok(clock) = deserialize(data) {
                    self.set_clock(clock);
                }
            });
        }
        if self.epoch_schedule.is_none() {
            get_account_data(&epoch_schedule::id(), &mut |data: &[u8]| {
                if let Ok(epoch_schedule) = deserialize(data) {
                    self.set_epoch_schedule(epoch_schedule);
                }
            });
        }

        if self.epoch_rewards.is_none() {
            get_account_data(&epoch_rewards::id(), &mut |data: &[u8]| {
                if let Ok(epoch_rewards) = deserialize(data) {
                    self.set_epoch_rewards(epoch_rewards);
                }
            });
        }

        if self.rent.is_none() {
            get_account_data(&rent::id(), &mut |data: &[u8]| {
                if let Ok(rent) = deserialize(data) {
                    self.set_rent(rent);
                }
            });
        }
    }

    pub fn reset(&mut self) {
        *self = SysvarCache::default();
    }
}

/// These methods facilitate a transition from fetching sysvars from keyed
/// accounts to fetching from the sysvar cache without breaking consensus. In
/// order to keep consistent behavior, they continue to enforce the same checks
/// as `solana_sdk::keyed_account::from_keyed_account` despite dynamically
/// loading them instead of deserializing from account data.
pub mod get_sysvar_with_account_check {
    fn check_sysvar_account<S: Sysvar>(
        transaction_context: &TransactionContext,
        instruction_context: &InstructionContext,
        instruction_account_index: IndexOfAccount,
    ) -> Result<(), InstructionError> {
        let index_in_transaction = instruction_context
            .get_index_of_instruction_account_in_transaction(instruction_account_index)?;
        if !S::check_id(transaction_context.get_key_of_account_at_index(index_in_transaction)?) {
            return Err(InstructionError::InvalidArgument);
        }
        Ok(())
    }

    pub fn clock<SDK: SharedAPI>(
        invoke_context: &InvokeContext<SDK>,
        instruction_context: &InstructionContext,
        instruction_account_index: IndexOfAccount,
    ) -> Result<Arc<Clock>, InstructionError> {
        check_sysvar_account::<Clock>(
            &invoke_context.transaction_context,
            instruction_context,
            instruction_account_index,
        )?;
        invoke_context.get_sysvar_cache().get_clock()
    }

    use crate::{
        clock::Clock,
        context::{IndexOfAccount, InstructionContext, InvokeContext, TransactionContext},
        rent::Rent,
        solana_program::sysvar::Sysvar,
    };
    use alloc::sync::Arc;
    use fluentbase_sdk::SharedAPI;
    use solana_instruction::error::InstructionError;

    pub fn rent<SDK: SharedAPI>(
        invoke_context: &InvokeContext<SDK>,
        instruction_context: &InstructionContext,
        instruction_account_index: IndexOfAccount,
    ) -> Result<Arc<Rent>, InstructionError> {
        check_sysvar_account::<Rent>(
            &invoke_context.transaction_context,
            instruction_context,
            instruction_account_index,
        )?;
        invoke_context.get_sysvar_cache().get_rent()
    }
}
