use serde::{Deserialize, Serialize};
use solana_instruction::TRANSACTION_LEVEL_STACK_HEIGHT;
use {
    // solana_program::instruction::{get_stack_height, TRANSACTION_LEVEL_STACK_HEIGHT},
    crate::token_2022::spl_pod::primitives::PodBool,
    crate::{
        token_2022::extension::{
            BaseStateWithExtensions, Extension, ExtensionType, StateWithExtensionsMut,
        },
        token_2022::state::Account,
    },
    bytemuck::{Pod, Zeroable},
};

/// CPI Guard extension instructions
pub mod instruction;

/// CPI Guard extension processor
pub mod processor;

/// CPI Guard extension for Accounts
#[repr(C)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct CpiGuard {
    /// Lock privileged token operations from happening via CPI
    pub lock_cpi: PodBool,
}
impl Extension for CpiGuard {
    const TYPE: ExtensionType = ExtensionType::CpiGuard;
}

/// Determine if CPI Guard is enabled for this account
pub fn cpi_guard_enabled(account_state: &StateWithExtensionsMut<Account>) -> bool {
    if let Ok(extension) = account_state.get_extension::<CpiGuard>() {
        return extension.lock_cpi.into();
    }
    false
}

/// Determine if we are in CPI
pub fn in_cpi() -> bool {
    get_stack_height() > TRANSACTION_LEVEL_STACK_HEIGHT
}
