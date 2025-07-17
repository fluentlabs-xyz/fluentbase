extern crate alloc;
use fluentbase_examples_svm_bindings::{
    big_mod_exp_3,
    curve_group_op_native,
    curve_validate_point_native,
    get_return_data,
    log_data_native,
    log_pubkey_native,
    secp256k1_recover_native,
    set_return_data_native,
    sol_blake3_native,
    sol_keccak256_native,
    sol_sha256_native,
};
use fluentbase_svm_shared::{bincode_helpers::deserialize, test_structs::TestCommand};
use num_derive::FromPrimitive;
use solana_account_info::{next_account_info, AccountInfo, MAX_PERMITTED_DATA_INCREASE};
use solana_msg::msg;
use solana_program::{program::invoke_signed, system_instruction};
use solana_program_entrypoint::{entrypoint_no_alloc, ProgramResult};
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;
use solana_sdk::decode_error::DecodeError;
use std::str::from_utf8;

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

    let instruction_data: Vec<u8> = deserialize(instruction_data).map_err(|e| {
        msg!(
            "process_instruction: failed to deserialize 'instruction_data' (len: {}): {}",
            instruction_data.len(),
            e
        );
        ProgramError::InvalidInstructionData
    })?;
    msg!(
        "process_instruction:  {}): {:x?}",
        instruction_data.len(),
        &instruction_data
    );
    let test_command: TestCommand =
        deserialize(&instruction_data).expect("failed to deserialize test command");
    msg!("processing test_command: {:?}", test_command);
    match test_command {
        TestCommand::ModifyAccount1(p) => {
            msg!("process_instruction: applying modifications to account 1");

            let account = &accounts[p.account_idx];
            account.realloc(account.data_len() + MAX_PERMITTED_DATA_INCREASE, false)?;
            account.data.borrow_mut()[p.byte_n_to_set as usize] = p.byte_n_val;

            msg!("process_instruction: Command finished");
        }
        TestCommand::CreateAccountAndModifySomeData1(p) => {
            msg!("process_instruction: creating account");
            msg!("process_instruction: lamports {}", p.lamports_to_send);
            msg!("process_instruction: space {}", p.space,);
            let mut p_seeds = p.seeds.clone();
            for (idx, seed) in p_seeds.iter().enumerate() {
                msg!(
                    "process_instruction: Create account: seed{}: '{}'",
                    idx,
                    from_utf8(&seed).map_err(|e| {
                        msg!(
                            "process_instruction: failed to convert to a valid UTF-8 string: {}",
                            e
                        );
                        ProgramError::InvalidInstructionData
                    })?
                );
            }
            msg!("process_instruction: byte_n_to_set: '{}'", p.byte_n_to_set);
            msg!("process_instruction: byte_n_value: {}", p.byte_n_value);

            let account_info_iter = &mut accounts.iter();

            let payer = next_account_info(account_info_iter)?; // Signer
            let new_account: &AccountInfo = next_account_info(account_info_iter)?; // Account to create (can be a PDA)
            let system_program_account = next_account_info(account_info_iter)?;

            p_seeds.push(payer.key.as_ref().to_vec());
            let seeds_addr = p_seeds.as_ptr() as u64;
            msg!(
                "process_instruction: deriving pda: seeds {:x?} (addr:{}) program_id {:x?}",
                p_seeds,
                seeds_addr,
                program_id.as_ref()
            );
            let seeds = p_seeds.iter().map(|v| v.as_slice()).collect::<Vec<_>>();
            let (pda, bump) = Pubkey::find_program_address(seeds.as_slice(), program_id);
            msg!(
                "process_instruction: result pda: {:x?} bump: {}",
                &pda.to_bytes(),
                bump
            );

            let signer_seeds = &[&p_seeds[0], payer.key.as_ref(), &[bump]];

            msg!(
                "payer.key: {:x?} new_account.key: {:x?} lamports {} space {} program_id {:x?} signer_seeds {:x?}",
                payer.key.to_bytes(),
                new_account.key.to_bytes(),
                p.lamports_to_send,
                p.space,
                program_id.to_bytes(),
                signer_seeds
            );
            msg!("process_instruction: calling invoke");

            let account_infos = &[
                payer.clone(),
                new_account.clone(),
                system_program_account.clone(),
            ];

            // invoke(
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    new_account.key,
                    p.lamports_to_send,
                    p.space as u64,
                    program_id, // Owner of your program
                ),
                account_infos,
                &[signer_seeds], // optional, only if using PDA
            )?;

            new_account.data.borrow_mut()[p.byte_n_to_set as usize] = p.byte_n_value;

            msg!("Create account: end");
        }
        TestCommand::SolBigModExp(p) => {
            let modulus: [u8; 32] = p.modulus.try_into().unwrap();
            let result = big_mod_exp_3(&p.base, &p.exponent, &modulus);
            assert_eq!(&p.expected, &result.1);
        }
        TestCommand::SolSecp256k1Recover(p) => {
            let message = p.message;
            let message_hash = {
                let mut hasher = solana_program::keccak::Hasher::default();
                hasher.hash(&message);
                hasher.result()
            };

            let signature =
                libsecp256k1::Signature::parse_standard_slice(&p.signature_bytes).unwrap();

            // Flip the S value in the signature to make a different but valid signature.
            let mut alt_signature = signature;
            alt_signature.s = -alt_signature.s;
            let alt_recovery_id = libsecp256k1::RecoveryId::parse(p.recovery_id ^ 1).unwrap();

            let alt_signature_bytes = alt_signature.serialize();
            let alt_recovery_id = alt_recovery_id.serialize();

            let recovered_pubkey = secp256k1_recover_native(
                &message_hash.0,
                p.recovery_id as u64,
                &p.signature_bytes.try_into().unwrap(),
            );
            msg!("recovered_pubkey {:x?}", &recovered_pubkey);
            assert_eq!(&recovered_pubkey.1, p.pubkey_bytes.as_slice());

            let alt_recovered_pubkey = secp256k1_recover_native(
                &message_hash.0,
                alt_recovery_id as u64,
                &alt_signature_bytes,
            );
            msg!("alt_recovered_pubkey {:x?}", &alt_recovered_pubkey);
            assert_eq!(alt_recovered_pubkey.1, p.pubkey_bytes.as_slice());
        }
        TestCommand::Keccak256(p) => {
            let data: Vec<&[u8]> = p.data.iter().map(|v| v.as_slice()).collect();
            let expected_result = p.expected_result;
            let result = sol_keccak256_native(&data);
            assert_eq!(&result.1, expected_result.as_slice());
            let mut data_solid = Vec::new();
            for v in &p.data {
                data_solid.extend_from_slice(v);
            }
            let result = sol_keccak256_native(&[data_solid.as_slice()]);
            assert_eq!(&result.1, expected_result.as_slice());
        }
        TestCommand::Sha256(p) => {
            let data: Vec<&[u8]> = p.data.iter().map(|v| v.as_slice()).collect();
            let expected_result = p.expected_result;
            let result = sol_sha256_native(&data);
            assert_eq!(&result.1, expected_result.as_slice());
            let mut data_solid = Vec::new();
            for v in &p.data {
                data_solid.extend_from_slice(v);
            }
            let result = sol_sha256_native(&[data_solid.as_slice()]);
            assert_eq!(&result.1, expected_result.as_slice());
        }
        TestCommand::Blake3(p) => {
            let data: Vec<&[u8]> = p.data.iter().map(|v| v.as_slice()).collect();
            let expected_result = p.expected_result;
            let result = sol_blake3_native(data.as_slice());
            assert_eq!(&result.1, expected_result.as_slice());
            let data_solid: &[&[u8]] = &[&[1u8, 2, 3, 4, 5, 6]];
            let result = sol_blake3_native(data_solid);
            assert_eq!(&result.1, expected_result.as_slice());
        }
        TestCommand::SetGetReturnData(p) => {
            let data: &[u8] = p.data.as_slice();
            let return_data_before_set = get_return_data();
            assert_eq!(return_data_before_set, None);
            set_return_data_native(data);
            let return_data_after_set =
                get_return_data().expect("return data must exists as it has already been set");
            assert_eq!(&return_data_after_set.1, data);
        }
        TestCommand::CurvePointValidation(p) => {
            let result = curve_validate_point_native(p.curve_id, &p.point.try_into().unwrap());
            assert_eq!(result, p.expected_ret);
        }
        TestCommand::CurveGroupOp(p) => {
            let mut point = [0u8; 32];
            let result = curve_group_op_native(
                p.curve_id,
                p.group_op,
                &p.left_input.try_into().unwrap(),
                &p.right_input.try_into().unwrap(),
                &mut point,
            );
            assert_eq!(result, p.expected_ret);
            let expected_point: [u8; 32] = p.expected_point.try_into().unwrap();
            assert_eq!(&expected_point, &point);
        }
    }

    Ok(())
}
