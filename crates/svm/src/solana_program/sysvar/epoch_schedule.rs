use crate::{
    impl_sysvar_get,
    solana_program::{program_error::ProgramError, sysvar::Sysvar},
};
pub use solana_epoch_schedule::{
    sysvar::{check_id, id, ID},
    EpochSchedule,
};

impl Sysvar for EpochSchedule {
    impl_sysvar_get!(sol_get_epoch_schedule_sysvar);
}
