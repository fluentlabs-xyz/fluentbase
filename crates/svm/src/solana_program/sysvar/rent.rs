use crate::{
    impl_sysvar_get,
    solana_program::{program_error::ProgramError, sysvar::Sysvar},
};
pub use solana_rent::{
    sysvar::{check_id, id, ID},
    Rent,
};

impl Sysvar for Rent {
    impl_sysvar_get!(sol_get_rent_sysvar);
}
