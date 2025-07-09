use solana_account_info::{next_account_info, AccountInfo};
use solana_program::{program::invoke, system_instruction};
use solana_program_entrypoint::{__msg, entrypoint_no_alloc, ProgramResult};
use solana_pubkey::Pubkey;

entrypoint_no_alloc!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    __msg!(
        "This is message from 'example solana program. program_id {:?} accounts.len {} instruction_data {:x?}",
        program_id,
        accounts.len(),
        instruction_data,
    );
    for (account_idx, account) in accounts.iter().enumerate() {
        __msg!("input account {}: {:?}", account_idx, account);
    }

    // simple transfer
    let transfer_amount = u64::from_be_bytes(
        instruction_data[0..core::mem::size_of::<u64>()]
            .try_into()
            .unwrap(),
    );
    __msg!("transfer_amount is {}", transfer_amount);
    let accounts_iter = &mut accounts.iter();
    let payer = next_account_info(accounts_iter)?;
    __msg!("payer.key: {:?}", payer.key.to_bytes());
    let recipient = next_account_info(accounts_iter)?;
    __msg!("recipient.key: {:?}", recipient.key.to_bytes());
    let system_program = next_account_info(accounts_iter)?;
    __msg!("system_program.key: {:?}", system_program.key.to_bytes());
    invoke(
        &system_instruction::transfer(payer.key, recipient.key, transfer_amount),
        &[payer.clone(), recipient.clone(), system_program.clone()],
    )?;

    Ok(())
}
