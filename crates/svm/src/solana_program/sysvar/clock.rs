use crate::{
    impl_sysvar_get,
    solana_program::{program_error::ProgramError, sysvar::Sysvar},
};
pub use solana_clock::{
    sysvar::{check_id, id, ID},
    Clock,
};

impl Sysvar for Clock {
    impl_sysvar_get!(sol_get_clock_sysvar);
}
