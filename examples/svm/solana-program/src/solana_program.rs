#![feature(error_in_core)]

use solana_account_info::AccountInfo;
use solana_program_entrypoint::{__msg, entrypoint, ProgramResult};
use solana_pubkey::Pubkey;

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    // log a message to the blockchain
    __msg!("This is message from 'example solana program. program_id {:?} accounts {} instruction_data {:x?}", program_id, accounts.len(), instruction_data);
    let message = "it is some message";
    __msg!("message='{}'", &message);
    // TODO bug in `keccak_hash` function doesnt allow to pass tests
    // let keccak_hash_res = keccak_hash(message.as_bytes());
    // msg!("message's keccak_hash_res={:x?}", &keccak_hash_res.to_bytes());
    // let poseidon_hash_res = poseidon_hash(Parameters::Bn254X5, Endianness::LittleEndian, message.as_bytes()).expect("poseidon hash computation");
    // msg!("message's poseidon_hash_res={:x?}", &poseidon_hash_res.to_bytes());

    // let (pda, bump_seed) = Pubkey::find_program_address(&[b"seed"], program_id);
    // msg!("Pubkey::find_program_address results: pda '{:x?}', bump_seed '{}'", &pda.to_bytes(), bump_seed);

    Ok(())
}

entrypoint!(process_instruction);
