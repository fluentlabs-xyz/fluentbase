use crate::{
    clock::Clock,
    epoch_rewards::EpochRewards,
    epoch_schedule::EpochSchedule,
    pubkey::Pubkey,
    rent::Rent,
    solana_program::{
        stake_history::StakeHistory,
        sysvar::{
            clock,
            epoch_rewards,
            epoch_schedule,
            recent_blockhashes,
            recent_blockhashes::RecentBlockhashes,
            rent,
            // slot_hashes,
            stake_history,
        },
    },
};
use alloc::sync::Arc;
use solana_bincode::deserialize;
use solana_instruction::error::InstructionError;
use solana_slot_hashes::SlotHashes;

#[derive(Default, Clone, Debug)]
pub struct SysvarCache {
    clock: Option<Arc<Clock>>,
    epoch_schedule: Option<Arc<EpochSchedule>>,
    epoch_rewards: Option<Arc<EpochRewards>>,
    // #[allow(deprecated)]
    // fees: Option<Arc<Fees>>,
    rent: Option<Arc<Rent>>,
    // slot_hashes: Option<Arc<SlotHashes>>,
    #[allow(deprecated)]
    recent_blockhashes: Option<Arc<RecentBlockhashes>>,
    stake_history: Option<Arc<StakeHistory>>,
    // last_restart_slot: Option<Arc<LastRestartSlot>>,
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

    // #[deprecated]
    // #[allow(deprecated)]
    // pub fn get_fees(&self) -> Result<Arc<Fees>, InstructionError> {
    //     self.fees.clone().ok_or(InstructionError::UnsupportedSysvar)
    // }

    // #[deprecated]
    // #[allow(deprecated)]
    // pub fn set_fees(&mut self, fees: Fees) {
    //     self.fees = Some(Arc::new(fees));
    // }

    pub fn get_rent(&self) -> Result<Arc<Rent>, InstructionError> {
        self.rent.clone().ok_or(InstructionError::UnsupportedSysvar)
    }

    pub fn set_rent(&mut self, rent: Rent) {
        self.rent = Some(Arc::new(rent));
    }

    // pub fn get_last_restart_slot(&self) -> Result<Arc<LastRestartSlot>, InstructionError> {
    //     self.last_restart_slot
    //         .clone()
    //         .ok_or(InstructionError::UnsupportedSysvar)
    // }
    //
    // pub fn set_last_restart_slot(&mut self, last_restart_slot: LastRestartSlot) {
    //     self.last_restart_slot = Some(Arc::new(last_restart_slot));
    // }

    // pub fn get_slot_hashes(&self) -> Result<Arc<SlotHashes>, InstructionError> {
    //     self.slot_hashes
    //         .clone()
    //         .ok_or(InstructionError::UnsupportedSysvar)
    // }
    //
    // pub fn set_slot_hashes(&mut self, slot_hashes: SlotHashes) {
    //     self.slot_hashes = Some(Arc::new(slot_hashes));
    // }

    #[deprecated]
    #[allow(deprecated)]
    pub fn get_recent_blockhashes(&self) -> Result<Arc<RecentBlockhashes>, InstructionError> {
        self.recent_blockhashes
            .clone()
            .ok_or(InstructionError::UnsupportedSysvar)
    }

    #[deprecated]
    #[allow(deprecated)]
    pub fn set_recent_blockhashes(&mut self, recent_blockhashes: RecentBlockhashes) {
        self.recent_blockhashes = Some(Arc::new(recent_blockhashes));
    }

    pub fn get_stake_history(&self) -> Result<Arc<StakeHistory>, InstructionError> {
        self.stake_history
            .clone()
            .ok_or(InstructionError::UnsupportedSysvar)
    }

    pub fn set_stake_history(&mut self, stake_history: StakeHistory) {
        self.stake_history = Some(Arc::new(stake_history));
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

        // #[allow(deprecated)]
        // if self.fees.is_none() {
        //     get_account_data(&fees::id(), &mut |data: &[u8]| {
        //         if let Ok(fees) = deserialize(data) {
        //             self.set_fees(fees);
        //         }
        //     });
        // }
        if self.rent.is_none() {
            get_account_data(&rent::id(), &mut |data: &[u8]| {
                if let Ok(rent) = deserialize(data) {
                    self.set_rent(rent);
                }
            });
        }
        // if self.slot_hashes.is_none() {
        //     get_account_data(&slot_hashes::id(), &mut |data: &[u8]| {
        //         if let Ok(slot_hashes) = deserialize(data) {
        //             self.set_slot_hashes(slot_hashes);
        //         }
        //     });
        // }
        #[allow(deprecated)]
        if self.recent_blockhashes.is_none() {
            get_account_data(&recent_blockhashes::id(), &mut |data: &[u8]| {
                if let Ok(recent_blockhashes) = deserialize(data) {
                    self.set_recent_blockhashes(recent_blockhashes);
                }
            });
        }
        if self.stake_history.is_none() {
            get_account_data(&stake_history::id(), &mut |data: &[u8]| {
                if let Ok(stake_history) = deserialize(data) {
                    self.set_stake_history(stake_history);
                }
            });
        }
        // if self.last_restart_slot.is_none() {
        //     get_account_data(&last_restart_slot::id(), &mut |data: &[u8]| {
        //         if let Ok(last_restart_slot) = deserialize(data) {
        //             self.set_last_restart_slot(last_restart_slot);
        //         }
        //     });
        // }
    }

    pub fn reset(&mut self) {
        *self = SysvarCache::default();
    }
}

// /// These methods facilitate a transition from fetching sysvars from keyed
// /// accounts to fetching from the sysvar cache without breaking consensus. In
// /// order to keep consistent behavior, they continue to enforce the same checks
// /// as `solana_sdk::keyed_account::from_keyed_account` despite dynamically
// /// loading them instead of deserializing from account data.
pub mod get_sysvar_with_account_check {
    //     use super::*;

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
        solana_program::sysvar::{recent_blockhashes::RecentBlockhashes, Sysvar},
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

    //     pub fn slot_hashes(
    //         invoke_context: &InvokeContext,
    //         instruction_context: &InstructionContext,
    //         instruction_account_index: IndexOfAccount,
    //     ) -> Result<Arc<SlotHashes>, InstructionError> {
    //         check_sysvar_account::<SlotHashes>(
    //             invoke_context.transaction_context,
    //             instruction_context,
    //             instruction_account_index,
    //         )?;
    //         invoke_context.get_sysvar_cache().get_slot_hashes()
    //     }

    #[allow(deprecated)]
    pub fn recent_blockhashes<SDK: SharedAPI>(
        invoke_context: &InvokeContext<SDK>,
        instruction_context: &InstructionContext,
        instruction_account_index: IndexOfAccount,
    ) -> Result<Arc<RecentBlockhashes>, InstructionError> {
        check_sysvar_account::<RecentBlockhashes>(
            &invoke_context.transaction_context,
            instruction_context,
            instruction_account_index,
        )?;
        invoke_context.get_sysvar_cache().get_recent_blockhashes()
    }

    //     pub fn stake_history(
    //         invoke_context: &InvokeContext,
    //         instruction_context: &InstructionContext,
    //         instruction_account_index: IndexOfAccount,
    //     ) -> Result<Arc<StakeHistory>, InstructionError> {
    //         check_sysvar_account::<StakeHistory>(
    //             invoke_context.transaction_context,
    //             instruction_context,
    //             instruction_account_index,
    //         )?;
    //         invoke_context.get_sysvar_cache().get_stake_history()
    //     }
    //
    //     pub fn last_restart_slot(
    //         invoke_context: &InvokeContext,
    //         instruction_context: &InstructionContext,
    //         instruction_account_index: IndexOfAccount,
    //     ) -> Result<Arc<LastRestartSlot>, InstructionError> {
    //         check_sysvar_account::<LastRestartSlot>(
    //             invoke_context.transaction_context,
    //             instruction_context,
    //             instruction_account_index,
    //         )?;
    //         invoke_context.get_sysvar_cache().get_last_restart_slot()
    //     }
}
