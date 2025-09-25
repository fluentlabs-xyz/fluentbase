#![allow(clippy::arithmetic_side_effects)]
#![deny(missing_docs)]
#![cfg_attr(not(test), forbid(unsafe_code))]

//! An ERC20-like Token program for the Solana blockchain

use crate::system_program;
use alloc::format;
use alloc::string::String;
use solana_program_error::{ProgramError, ProgramResult};
use solana_pubkey::{declare_id, Pubkey};

/// Convert the UI representation of a token amount (using the decimals field
/// defined in its mint) to the raw amount
pub fn ui_amount_to_amount(ui_amount: f64, decimals: u8) -> u64 {
    (ui_amount * 10_usize.pow(decimals as u32) as f64) as u64
}

/// Convert a raw amount to its UI representation (using the decimals field
/// defined in its mint)
pub fn amount_to_ui_amount(amount: u64, decimals: u8) -> f64 {
    amount as f64 / 10_usize.pow(decimals as u32) as f64
}

//     let after_decimal = parts.next().unwrap_or("");
//     let after_decimal = after_decimal.trim_end_matches('0');
//     if (amount_str.is_empty() && after_decimal.is_empty())
//         || parts.next().is_some()
//         || after_decimal.len() > decimals
//     {
//         return Err(ProgramError::InvalidArgument);
//     }
//
//     amount_str.push_str(after_decimal);
//     for _ in 0..decimals.saturating_sub(after_decimal.len()) {
//         amount_str.push('0');
//     }
//     amount_str
//         .parse::<u64>()
//         .map_err(|_| ProgramError::InvalidArgument)
// }

declare_id!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Checks that the supplied program ID is correct for spl-token-2022
pub fn check_program_account(spl_token_program_id: &Pubkey) -> ProgramResult {
    if spl_token_program_id != &id() {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

/// Checks that the supplied program ID is correct for spl-token or
/// spl-token-2022
pub fn check_spl_token_program_account(spl_token_program_id: &Pubkey) -> ProgramResult {
    if spl_token_program_id != &id()
    /* && spl_token_program_id != &spl_token::id()*/
    {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

/// Checks if the spplied program ID is that of the system program
pub fn check_system_program_account(system_program_id: &Pubkey) -> ProgramResult {
    if system_program_id != &system_program::id() {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}
