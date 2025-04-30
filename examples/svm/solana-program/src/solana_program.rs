#![feature(error_in_core)]

use solana_account_info::{next_account_info, AccountInfo};
use solana_program::{program::invoke, system_instruction};
use solana_program_entrypoint::{__msg, entrypoint_no_alloc, ProgramResult};
use solana_pubkey::Pubkey;

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    // log a message to the blockchain
    __msg!(
        "This is message from 'example solana program. program_id {:?} accounts.len {} instruction_data {:x?}",
        program_id,
        accounts.len(),
        instruction_data,
    );
    for (account_idx, account) in accounts.iter().enumerate() {
        __msg!("input account {}: {:?}", account_idx, account);
    }

    // simple transfer below
    let amount = u64::from_be_bytes(instruction_data[0..8].try_into().unwrap());
    // let accounts_iter = &mut accounts.iter();
    // let payer = next_account_info(accounts_iter)?;
    // let recipient = next_account_info(accounts_iter)?;
    // let system_program = next_account_info(accounts_iter)?;
    // invoke(
    //     &system_instruction::transfer(payer.key, recipient.key, amount),
    //     &[payer.clone(), recipient.clone(), system_program.clone()],
    // )?;
    __msg!("amount is {}", amount);

    // TODO bug in `keccak_hash` function doesnt allow to pass tests
    // let keccak_hash_res = keccak_hash(message.as_bytes());
    // msg!("message's keccak_hash_res={:x?}", &keccak_hash_res.to_bytes());
    // let poseidon_hash_res = poseidon_hash(Parameters::Bn254X5, Endianness::LittleEndian, message.as_bytes()).expect("poseidon hash computation");
    // msg!("message's poseidon_hash_res={:x?}", &poseidon_hash_res.to_bytes());

    // let (pda, bump_seed) = Pubkey::find_program_address(&[b"seed"], program_id);
    // msg!("Pubkey::find_program_address results: pda '{:x?}', bump_seed '{}'", &pda.to_bytes(), bump_seed);

    Ok(())
}

entrypoint_no_alloc!(process_instruction);
