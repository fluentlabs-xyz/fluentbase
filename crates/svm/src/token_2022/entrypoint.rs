//! Program entrypoint

use crate::error::TokenError;
use crate::token_2022::processor::Processor;
use solana_account_info::AccountInfo;
use solana_program_error::{PrintProgramError, ProgramResult};
use solana_pubkey::Pubkey;
