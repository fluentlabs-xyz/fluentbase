use crate::{
    clock::Clock,
    epoch_schedule::EpochSchedule,
    pubkey::Pubkey,
    solana_program::sysvar::{clock, epoch_schedule},
};
use alloc::sync::Arc;
use solana_bincode::deserialize;
use solana_instruction::error::InstructionError;

#[derive(Default, Clone, Debug)]
pub struct SysvarCache {
    clock: Option<Arc<Clock>>,
    epoch_schedule: Option<Arc<EpochSchedule>>,
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
    use crate::{
        clock::Clock,
        context::{IndexOfAccount, InstructionContext, InvokeContext, TransactionContext},
        solana_program::sysvar::Sysvar,
    };
    use alloc::sync::Arc;
    use fluentbase_sdk::SharedAPI;
    use solana_instruction::error::InstructionError;

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
}
