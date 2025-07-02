use crate::{
    impl_sysvar_get,
    solana_program::{program_error::ProgramError, sysvar::Sysvar},
};
pub use solana_last_restart_slot::{
    sysvar::{check_id, id, ID},
    LastRestartSlot,
};

impl Sysvar for LastRestartSlot {
    impl_sysvar_get!(sol_get_last_restart_slot);
}
