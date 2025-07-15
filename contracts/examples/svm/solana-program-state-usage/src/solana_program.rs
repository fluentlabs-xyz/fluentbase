extern crate alloc;
use fluentbase_examples_svm_bindings::{
    big_mod_exp_3,
    get_return_data,
    log_data_native,
    log_pubkey_native,
    secp256k1_recover_native,
    set_return_data_native,
    sol_blake3_native,
    sol_keccak256_native,
    sol_sha256_native,
};
use hex_literal::hex;
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
    log_pubkey_native(&program_id);

    let test_data_for_log: &[&[u8]] = &[&[1, 2, 3], &[4, 5, 6]];
    log_data_native(test_data_for_log);

    let test_data_for_set_get_return_data: &[u8] = &[7, 6, 5, 4, 3, 2, 1];
    let return_data_before_set = get_return_data();
    msg!("return_data_before_set {:x?}", return_data_before_set);
    set_return_data_native(test_data_for_set_get_return_data);
    let return_data_after_set =
        get_return_data().expect("return data must exists as it has already been set");
    assert_eq!(&return_data_after_set.1, test_data_for_set_get_return_data);
    msg!(
        "return_data_after_set {:x?} (pk hex bytes: {:x?})",
        return_data_after_set.0,
        return_data_after_set.0.to_bytes()
    );

    let test_data_for_keccak256: &[&[u8]] = &[&[1u8, 2, 3], &[4, 5, 6]];
    let keccak256_result_expected =
        hex!("13a08e3cd39a1bc7bf9103f63f83273cced2beada9f723945176d6b983c65bd2");
    let keccak256_result = sol_keccak256_native(test_data_for_keccak256);
    msg!(
        "test_data_for_keccak256 {:x?} keccak256_result {:x?}",
        test_data_for_keccak256,
        hex::encode(keccak256_result)
    );
    assert_eq!(&keccak256_result, &keccak256_result_expected);
    let test_data_for_keccak256: &[&[u8]] = &[&[1u8, 2, 3, 4, 5, 6]];
    let keccak256_result = sol_keccak256_native(test_data_for_keccak256);
    assert_eq!(&keccak256_result, &keccak256_result_expected);

    let test_data_for_sha256: &[&[u8]] = &[&[1u8, 2, 3], &[4, 5, 6]];
    let sha256_result_expected =
        hex!("7192385c3c0605de55bb9476ce1d90748190ecb32a8eed7f5207b30cf6a1fe89");
    let sha256_result = sol_sha256_native(test_data_for_sha256);
    msg!(
        "test_data_for_sha256 {:x?} sha256_result {}",
        test_data_for_sha256,
        hex::encode(&sha256_result)
    );
    assert_eq!(&sha256_result, &sha256_result_expected);
    let test_data_for_sha256: &[&[u8]] = &[&[1u8, 2, 3, 4, 5, 6]];
    let sha256_result = sol_sha256_native(test_data_for_sha256);
    assert_eq!(&sha256_result, &sha256_result_expected);

    let test_data_for_blake3: &[&[u8]] = &[&[1u8, 2, 3], &[4, 5, 6]];
    let blake3_result_expected =
        hex!("828a8660ae86b86f1ebf951a6f84349520cc1501fb6fcf95b05df01200be9fa2");
    let blake3_result = sol_blake3_native(test_data_for_blake3);
    msg!(
        "test_data_for_blake3 {:x?} blake3_result {}",
        test_data_for_blake3,
        hex::encode(&blake3_result)
    );
    assert_eq!(&blake3_result, &blake3_result_expected);
    let test_data_for_blake3: &[&[u8]] = &[&[1u8, 2, 3, 4, 5, 6]];
    let blake3_result = sol_blake3_native(test_data_for_blake3);
    assert_eq!(&blake3_result, &blake3_result_expected);

    // sol_secp256k1_recover
    {
        let message = b"hello world";
        let message_hash = {
            let mut hasher = solana_program::keccak::Hasher::default();
            hasher.hash(message);
            hasher.result()
        };

        let pubkey_bytes: [u8; 64] = [
            0x9B, 0xEE, 0x7C, 0x18, 0x34, 0xE0, 0x18, 0x21, 0x7B, 0x40, 0x14, 0x9B, 0x84, 0x2E,
            0xFA, 0x80, 0x96, 0x00, 0x1A, 0x9B, 0x17, 0x88, 0x01, 0x80, 0xA8, 0x46, 0x99, 0x09,
            0xE9, 0xC4, 0x73, 0x6E, 0x39, 0x0B, 0x94, 0x00, 0x97, 0x68, 0xC2, 0x28, 0xB5, 0x55,
            0xD3, 0x0C, 0x0C, 0x42, 0x43, 0xC1, 0xEE, 0xA5, 0x0D, 0xC0, 0x48, 0x62, 0xD3, 0xAE,
            0xB0, 0x3D, 0xA2, 0x20, 0xAC, 0x11, 0x85, 0xEE,
        ];
        let signature_bytes: [u8; 64] = [
            0x93, 0x92, 0xC4, 0x6C, 0x42, 0xF6, 0x31, 0x73, 0x81, 0xD4, 0xB2, 0x44, 0xE9, 0x2F,
            0xFC, 0xE3, 0xF4, 0x57, 0xDD, 0x50, 0xB3, 0xA5, 0x20, 0x26, 0x3B, 0xE7, 0xEF, 0x8A,
            0xB0, 0x69, 0xBB, 0xDE, 0x2F, 0x90, 0x12, 0x93, 0xD7, 0x3F, 0xA0, 0x29, 0x0C, 0x46,
            0x4B, 0x97, 0xC5, 0x00, 0xAD, 0xEA, 0x6A, 0x64, 0x4D, 0xC3, 0x8D, 0x25, 0x24, 0xEF,
            0x97, 0x6D, 0xC6, 0xD7, 0x1D, 0x9F, 0x5A, 0x26,
        ];
        let recovery_id: u8 = 0;

        let signature = libsecp256k1::Signature::parse_standard_slice(&signature_bytes).unwrap();

        // Flip the S value in the signature to make a different but valid signature.
        let mut alt_signature = signature;
        alt_signature.s = -alt_signature.s;
        let alt_recovery_id = libsecp256k1::RecoveryId::parse(recovery_id ^ 1).unwrap();

        let alt_signature_bytes = alt_signature.serialize();
        let alt_recovery_id = alt_recovery_id.serialize();

        let recovered_pubkey =
            secp256k1_recover_native(&message_hash.0, recovery_id as u64, &signature_bytes);
        msg!("recovered_pubkey {:x?}", &recovered_pubkey);
        // TODO errors. looks like uses unimplemented builtin (sol_mem* or sol_alloc_free_). need better error handling
        // assert_eq!(&recovered_pubkey, &pubkey_bytes);

        let alt_recovered_pubkey = secp256k1_recover_native(
            &message_hash.0,
            alt_recovery_id as u64,
            &alt_signature_bytes,
        );
        msg!("alt_recovered_pubkey {:x?}", &alt_recovered_pubkey);
        // TODO errors. looks like uses unimplemented builtin (sol_mem* or sol_alloc_free_). need better error handling
        // assert_eq!(alt_recovered_pubkey, pubkey_bytes);
    }

    // sol_big_mod_exp
    {
        // {
        //             "Base":     "1111111111111111111111111111111111111111111111111111111111111111",
        //             "Exponent": "1111111111111111111111111111111111111111111111111111111111111111",
        //             "Modulus":  "111111111111111111111111111111111111111111111111111111111111110A",
        //             "Expected": "0A7074864588D6847F33A168209E516F60005A0CEC3F33AAF70E8002FE964BCD"
        //         },
        let base = hex::decode("1111111111111111111111111111111111111111111111111111111111111111")
            .expect("failed to decode 'base'");
        let exponent =
            hex::decode("1111111111111111111111111111111111111111111111111111111111111111")
                .expect("failed to decode 'exponent'");
        let modulus =
            hex::decode("111111111111111111111111111111111111111111111111111111111111110A")
                .expect("failed to decode 'modulus");
        let expected =
            hex::decode("0A7074864588D6847F33A168209E516F60005A0CEC3F33AAF70E8002FE964BCD")
                .expect("failed to decode 'expected'");
        assert_eq!(base.len(), 32);
        assert_eq!(exponent.len(), 32);
        assert_eq!(modulus.len(), 32);
        assert_eq!(expected.len(), 32);
        let modulus: [u8; 32] = modulus.try_into().unwrap();
        let result = big_mod_exp_3(&base, &exponent, &modulus);
        msg!("big_mod_exp_3 result {:x?}", result);
        assert_eq!(&expected, &result);
    }

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
