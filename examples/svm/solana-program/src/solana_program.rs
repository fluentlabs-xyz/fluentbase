use solana_account_info::AccountInfo;
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

    Ok(())
}
