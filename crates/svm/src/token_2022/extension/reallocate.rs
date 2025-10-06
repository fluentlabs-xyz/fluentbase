use crate::{
    error::TokenError,
    token_2022::{
        extension::{
            set_account_type, AccountType, BaseStateWithExtensions, ExtensionType,
            StateWithExtensions,
        },
        processor::Processor,
        state::Account,
    },
};
use alloc::vec::Vec;
use fluentbase_sdk::{debug_log, SharedAPI};
use solana_account_info::{next_account_info, AccountInfo};
use solana_program_error::ProgramResult;
use solana_pubkey::Pubkey;

/// Processes a [Reallocate](enum.TokenInstruction.html) instruction
pub fn process_reallocate<SDK: SharedAPI>(
    sdk: &mut SDK,
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    new_extension_types: Vec<ExtensionType>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let token_account_info = next_account_info(account_info_iter)?;
    let payer_info = next_account_info(account_info_iter)?;
    let system_program_info = next_account_info(account_info_iter)?;
    let authority_info = next_account_info(account_info_iter)?;
    let authority_info_data_len = authority_info.data_len();

    // check that account is the right type and validate owner
    let (mut current_extension_types, native_token_amount) = {
        let token_account = token_account_info.data.borrow();
        let account = StateWithExtensions::<Account>::unpack(&token_account)?;
        Processor::new(sdk).validate_owner(
            program_id,
            &account.base.owner,
            authority_info,
            authority_info_data_len,
            account_info_iter.as_slice(),
        )?;
        let native_token_amount = account.base.is_native().then_some(account.base.amount);
        (account.get_extension_types()?, native_token_amount)
    };

    // check that all desired extensions are for the right account type
    if new_extension_types
        .iter()
        .any(|extension_type| extension_type.get_account_type() != AccountType::Account)
    {
        return Err(TokenError::InvalidState.into());
    }
    // ExtensionType::try_calculate_account_len() dedupes types, so just a dumb
    // concatenation is fine here
    current_extension_types.extend_from_slice(&new_extension_types);
    let needed_account_len =
        ExtensionType::try_calculate_account_len::<Account>(&current_extension_types)?;

    // if account is already large enough, return early
    if token_account_info.data_len() >= needed_account_len {
        return Ok(());
    }

    // reallocate
    debug_log!(
        "account needs realloc, +{:?} bytes",
        needed_account_len - token_account_info.data_len()
    );
    token_account_info.realloc(needed_account_len, false)?;

    // set account_type, if needed
    let mut token_account_data = token_account_info.data.borrow_mut();
    set_account_type::<Account>(&mut token_account_data)?;

    Ok(())
}
