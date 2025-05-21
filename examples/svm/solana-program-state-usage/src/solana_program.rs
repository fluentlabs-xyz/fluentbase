use num_derive::FromPrimitive;
use solana_account_info::{next_account_info, AccountInfo, MAX_PERMITTED_DATA_INCREASE};
use solana_msg::msg;
use solana_program::{
    program::{invoke, invoke_signed},
    serialize_utils::cursor::read_u64,
    system_instruction,
    sysvar::Sysvar,
};
use solana_program_entrypoint::{entrypoint_no_alloc, ProgramResult};
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;
use solana_sdk::{
    decode_error::DecodeError,
    rent::Rent,
    serialize_utils::{
        cursor::{read_u32, read_u8},
        read_slice,
    },
};
use std::{
    io::{Cursor, Read},
    str::from_utf8,
};

extern crate alloc;

/// Custom program errors
#[derive(Debug, Clone, PartialEq, FromPrimitive)]
pub enum MyError {
    // #[error("Default enum start")]
    DefaultEnumStart,
    // #[error("The Answer")]
    TheAnswer = 42,
}
impl From<MyError> for ProgramError {
    fn from(e: MyError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
impl<T> DecodeError<T> for MyError {
    fn type_of() -> &'static str {
        "MyError"
    }
}

entrypoint_no_alloc!(process_instruction);
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    msg!(
        "This is message from 'example solana program. program_id {:x?} accounts.len {} instruction_data {:x?}",
        program_id.to_bytes(),
        accounts.len(),
        instruction_data,
    );
    for (account_idx, account) in accounts.iter().enumerate() {
        msg!(
            "input account {}: {:?} key {:x?} owner {:x?}",
            account_idx,
            account,
            account.key.to_bytes(),
            account.owner.to_bytes()
        );
    }

    let instruction_data: Vec<u8> = bincode::deserialize(instruction_data).map_err(|e| {
        msg!(
            "failed to deserialize 'instruction_data' (len: {})",
            instruction_data.len()
        );
        ProgramError::InvalidInstructionData
    })?;
    msg!("instruction data: {:x?}", &instruction_data);
    let mut cursor = Cursor::new(instruction_data);
    let command_id = read_u8(&mut cursor).map_err(|e| {
        msg!("failed to read 'command_id' param");
        ProgramError::InvalidInstructionData
    })?;
    msg!("command_id: {}", command_id);
    match command_id {
        1 => {
            msg!("Apply modifications to account 1");

            let account = &accounts[1];
            account.realloc(account.data_len() + MAX_PERMITTED_DATA_INCREASE, false)?;
            account.data.borrow_mut()[0] = 123;

            msg!("Command finished");
        }
        2 => {
            msg!("Create account");
            let lamports = read_u64(&mut cursor).map_err(|e| {
                msg!("failed to read 'lamports' param");
                ProgramError::InvalidInstructionData
            })?;
            msg!("Create account: lamports {}", lamports);
            let space = read_u32(&mut cursor).map_err(|e| {
                msg!("failed to read 'space' param");
                ProgramError::InvalidInstructionData
            })?;
            msg!("Create account: space {}", space,);
            let seed_len1 = read_u8(&mut cursor).map_err(|e| {
                msg!("failed to read 'seed_len' param");
                ProgramError::InvalidInstructionData
            })?;
            msg!("Create account: seed_len1: {}", seed_len1);
            // let mut seed1 = b"my_seed";
            let mut seed1 = vec![0u8; seed_len1 as usize];
            cursor.read_exact(&mut seed1).map_err(|e| {
                msg!("failed to read 'seed1' param");
                ProgramError::InvalidInstructionData
            })?;
            msg!(
                "Create account: seed1: '{}'",
                from_utf8(&seed1).map_err(|e| ProgramError::InvalidInstructionData)?
            );
            let byte_n_to_set = read_u32(&mut cursor).map_err(|e| {
                msg!("failed to read 'byte_n_to_set' param");
                ProgramError::InvalidInstructionData
            })?;
            msg!("Create account: byte_n_to_set: '{}'", byte_n_to_set);
            let byte_n_value = read_u8(&mut cursor).map_err(|e| {
                msg!("failed to read 'byte_n_value' param");
                ProgramError::InvalidInstructionData
            })?;
            msg!("Create account: byte_n_value: '{}'", byte_n_value);

            let account_info_iter = &mut accounts.iter();

            let payer = next_account_info(account_info_iter)?; // Signer
            let new_account: &AccountInfo = next_account_info(account_info_iter)?; // Account to create (can be a PDA)
            let system_program_account = next_account_info(account_info_iter)?;

            let seed2 = payer.key.as_ref();
            let seeds = &[&seed1, seed2];
            let seeds_addr = seeds.as_ptr() as u64;
            msg!(
                "deriving pda: seeds {:x?} (addr:{}) program_id {:x?}",
                seeds,
                seeds_addr,
                program_id.as_ref()
            );
            let (pda, bump) = Pubkey::find_program_address(seeds, program_id);
            msg!("result pda: {:x?} bump: {}", &pda.to_bytes(), bump);

            let signer_seeds = &[&seed1, payer.key.as_ref(), &[bump]];

            msg!("pda: {:x?}", pda.to_bytes());
            msg!("payer.key: {:x?}", payer.key.to_bytes());
            msg!("new_account.key: {:x?}", new_account.key.to_bytes());
            msg!("calling invoke");
            // invoke(
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    new_account.key,
                    lamports,
                    space as u64,
                    program_id, // Owner of your program
                ),
                &[
                    payer.clone(),
                    new_account.clone(),
                    system_program_account.clone(),
                ],
                &[signer_seeds], // optional, only if using PDA
            )?;

            new_account.data.borrow_mut()[byte_n_to_set as usize] = byte_n_value;

            msg!("Create account: end");
        }
        _ => {
            msg!("Unrecognized command");
            return Err(ProgramError::InvalidArgument);
        }
    }

    Ok(())
}
