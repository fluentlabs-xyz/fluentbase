extern crate alloc;
use num_derive::FromPrimitive;
use solana_account_info::{next_account_info, AccountInfo, MAX_PERMITTED_DATA_INCREASE};
use solana_msg::msg;
use solana_program::{
    program::invoke_signed,
    serialize_utils::cursor::read_u64,
    system_instruction,
};
use solana_program_entrypoint::{entrypoint_no_alloc, ProgramResult};
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;
use solana_sdk::{
    decode_error::DecodeError,
    serialize_utils::cursor::{read_u32, read_u8},
};
use std::{
    io::{Cursor, Read},
    str::from_utf8,
};

/// Custom program errors
#[derive(Debug, Clone, PartialEq, FromPrimitive)]
pub enum MyError {
    DefaultEnumStart,
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
        "process_instruction: program_id {:x?} accounts.len {} instruction_data {:x?}",
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
            "process_instruction: failed to deserialize 'instruction_data' (len: {}): {}",
            instruction_data.len(),
            e
        );
        ProgramError::InvalidInstructionData
    })?;
    msg!(
        "process_instruction: instruction data: {:x?}",
        &instruction_data
    );
    let mut cursor = Cursor::new(instruction_data);
    let command_id = read_u8(&mut cursor).map_err(|e| {
        msg!(
            "process_instruction: failed to read 'command_id' param: {}",
            e
        );
        ProgramError::InvalidInstructionData
    })?;
    msg!("process_instruction: command_id: {}", command_id);
    match command_id {
        1 => {
            msg!("process_instruction: applying modifications to account 1");

            let account = &accounts[1];
            account.realloc(account.data_len() + MAX_PERMITTED_DATA_INCREASE, false)?;
            account.data.borrow_mut()[0] = 123;

            msg!("process_instruction: Command finished");
        }
        2 => {
            msg!("process_instruction: creating account");

            let lamports = read_u64(&mut cursor).map_err(|e| {
                msg!(
                    "process_instruction: failed to read 'lamports' param: {}",
                    e
                );
                ProgramError::InvalidInstructionData
            })?;
            msg!("process_instruction: lamports {}", lamports);
            let space = read_u32(&mut cursor).map_err(|e| {
                msg!("process_instruction: failed to read 'space' param: {}", e);
                ProgramError::InvalidInstructionData
            })?;
            msg!("process_instruction: space {}", space,);
            let seed_len1 = read_u8(&mut cursor).map_err(|e| {
                msg!(
                    "process_instruction: failed to read 'seed_len' param: {}",
                    e
                );
                ProgramError::InvalidInstructionData
            })?;
            msg!("process_instruction: seed_len1: {}", seed_len1);
            // let mut seed1 = b"my_seed";
            let mut seed1 = vec![0u8; seed_len1 as usize];
            cursor.read_exact(&mut seed1).map_err(|e| {
                msg!("process_instruction: failed to read 'seed1' param: {}", e);
                ProgramError::InvalidInstructionData
            })?;
            msg!(
                "process_instruction: Create account: seed1: '{}'",
                from_utf8(&seed1).map_err(|e| {
                    msg!(
                        "process_instruction: failed to convert to a valid UTF-8 string: {}",
                        e
                    );
                    ProgramError::InvalidInstructionData
                })?
            );
            let byte_n_to_set = read_u32(&mut cursor).map_err(|e| {
                msg!(
                    "process_instruction: failed to read 'byte_n_to_set' param: {}",
                    e
                );
                ProgramError::InvalidInstructionData
            })?;
            msg!("process_instruction: byte_n_to_set: '{}'", byte_n_to_set);
            let byte_n_value = read_u8(&mut cursor).map_err(|e| {
                msg!(
                    "process_instruction: failed to read 'byte_n_value' param: {}",
                    e
                );
                ProgramError::InvalidInstructionData
            })?;
            msg!("process_instruction: byte_n_value: {}", byte_n_value);

            let account_info_iter = &mut accounts.iter();

            let payer = next_account_info(account_info_iter)?; // Signer
            let new_account: &AccountInfo = next_account_info(account_info_iter)?; // Account to create (can be a PDA)
            let system_program_account = next_account_info(account_info_iter)?;

            let seed2 = payer.key.as_ref();
            let seeds = &[&seed1, seed2];
            let seeds_addr = seeds.as_ptr() as u64;
            msg!(
                "process_instruction: deriving pda: seeds {:x?} (addr:{}) program_id {:x?}",
                seeds,
                seeds_addr,
                program_id.as_ref()
            );
            let (pda, bump) = Pubkey::find_program_address(seeds, program_id);
            msg!(
                "process_instruction: result pda: {:x?} bump: {}",
                &pda.to_bytes(),
                bump
            );

            let signer_seeds = &[&seed1, payer.key.as_ref(), &[bump]];

            msg!(
                "payer.key: {:x?} new_account.key: {:x?} lamports {} space {} program_id {:x?} signer_seeds {:x?}",
                payer.key.to_bytes(),
                new_account.key.to_bytes(),
                lamports,
                space,
                program_id.to_bytes(),
                signer_seeds
            );
            msg!("process_instruction: calling invoke");

            let account_infos = &[
                payer.clone(),
                new_account.clone(),
                system_program_account.clone(),
            ];

            // let accounts_addr = accounts.as_ptr() as usize;
            // msg!("process_instruction: accounts_addr: {}", accounts_addr);
            // let account_info_struct_size = core::mem::size_of::<AccountInfo>();
            // let account_infos_slice = unsafe {
            //     core::slice::from_raw_parts(accounts_addr as *const u8, account_info_struct_size)
            // };
            // msg!(
            //     "in process_instruction: account_infos_slice {:x?}",
            //     account_infos_slice,
            // );

            // invoke(
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    new_account.key,
                    lamports,
                    space as u64,
                    program_id, // Owner of your program
                ),
                account_infos,
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
