pub use crate::epoch_rewards::EpochRewards;
use crate::{
    impl_sysvar_get,
    solana_program::{program_error::ProgramError, sysvar::Sysvar},
};
use solana_sysvar_id::declare_sysvar_id;

declare_sysvar_id!("SysvarEpochRewards1111111111111111111111111", EpochRewards);

impl Sysvar for EpochRewards {
    impl_sysvar_get!(sol_get_epoch_rewards_sysvar);
}
