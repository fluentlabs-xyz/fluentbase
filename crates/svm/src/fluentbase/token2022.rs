use crate::account::{Account, AccountSharedData};
use crate::helpers::{
    deserialize_svm_program_params, storage_read_metadata_params, storage_write_account_data,
};
use crate::token_2022;
use crate::token_2022::helpers::{
    flush_accounts, normalize_account_metas, reconstruct_account_infos, reconstruct_accounts,
};
use crate::token_2022::instruction::decode_instruction_type;
use crate::token_2022::pod_instruction::PodTokenInstruction;
use crate::token_2022::processor::Processor;
use crate::token_2022::state::Mint;
use alloc::vec::Vec;
use fluentbase_sdk::debug_log_ext;
use fluentbase_sdk::ContextReader;
use fluentbase_svm_common::common::evm_address_from_pubkey;
use fluentbase_types::{SharedAPI, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME};
use solana_instruction::AccountMeta;
use solana_program_error::{ProgramError, ProgramResult};
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;

pub fn token2022_process<const IS_DEPLOY: bool, SDK: SharedAPI>(
    sdk: &mut SDK,
    program_id: &Pubkey,
    account_metas: &[AccountMeta],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction_type: PodTokenInstruction =
        decode_instruction_type(instruction_data).expect("failed to decode instruction type");
    let contract_caller = sdk.context().contract_caller();

    for account_meta in account_metas {
        if account_meta.is_signer {
            assert_eq!(
                evm_address_from_pubkey::<true>(&account_meta.pubkey).expect("evm compatible pk"),
                contract_caller,
                "cannot be writable nor signer"
            );
        }
    }
    let mut account_metas = account_metas.to_vec();
    normalize_account_metas(&mut account_metas);

    let accounts = Processor::new(sdk).process_extended::<IS_DEPLOY>(
        program_id,
        &account_metas,
        instruction_data,
    )?;
    flush_accounts::<_, true>(
        sdk,
        &account_metas
            .iter()
            .zip(accounts.iter())
            .collect::<Vec<_>>(),
    )
    .expect("failed to flush accounts");
    Ok(())
}

pub fn token2022_process_raw<const IS_DEPLOY: bool, SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> ProgramResult {
    let (program_id, account_metas, instruction_data) =
        deserialize_svm_program_params(input).map_err(|_e| ProgramError::Custom(1))?;

    token2022_process::<IS_DEPLOY, SDK>(sdk, &program_id, &account_metas, instruction_data)
}
