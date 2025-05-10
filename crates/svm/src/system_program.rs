use crate::{
    account::BorrowedAccount,
    common::checked_add,
    context::{IndexOfAccount, InstructionContext, InvokeContext, TransactionContext},
    solana_program::nonce::{
        state::{AuthorizeNonceError, Data, DurableNonce, Versions},
        State,
    },
    system_instruction::SystemError,
};
use fluentbase_sdk::SharedAPI;
use hashbrown::HashSet;
use solana_instruction::error::InstructionError;
use solana_pubkey::{declare_id, Pubkey};
use solana_rent::Rent;

declare_id!("11111111111111111111111111111111");

pub fn advance_nonce_account<SDK: SharedAPI>(
    account: &mut BorrowedAccount,
    signers: &HashSet<Pubkey>,
    invoke_context: &InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    if !account.is_writable() {
        // ic_msg!(
        //     invoke_context,
        //     "Advance nonce account: Account {} must be writeable",
        //     account.get_key()
        // );
        return Err(InstructionError::InvalidArgument);
    }

    let state: Versions = account.get_state()?;
    match state.state() {
        State::Initialized(data) => {
            if !signers.contains(&data.authority) {
                // ic_msg!(
                //     invoke_context,
                //     "Advance nonce account: Account {} must be a signer",
                //     data.authority
                // );
                return Err(InstructionError::MissingRequiredSignature);
            }
            let next_durable_nonce =
                DurableNonce::from_blockhash(&invoke_context.environment_config.blockhash);
            if data.durable_nonce == next_durable_nonce {
                // ic_msg!(
                //     invoke_context,
                //     "Advance nonce account: nonce can only advance once per slot"
                // );
                return Err(SystemError::NonceBlockhashNotExpired.into());
            }

            let new_data = Data::new(
                data.authority,
                next_durable_nonce,
                invoke_context.environment_config.lamports_per_signature,
            );
            account.set_state(&Versions::new(State::Initialized(new_data)))
        }
        State::Uninitialized => {
            // ic_msg!(
            //     invoke_context,
            //     "Advance nonce account: Account {} state is invalid",
            //     account.get_key()
            // );
            Err(InstructionError::InvalidAccountData)
        }
    }
}

pub fn withdraw_nonce_account<SDK: SharedAPI>(
    from_account_index: IndexOfAccount,
    lamports: u64,
    to_account_index: IndexOfAccount,
    rent: &Rent,
    signers: &HashSet<Pubkey>,
    invoke_context: &InvokeContext<SDK>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
) -> Result<(), InstructionError> {
    let mut from = instruction_context
        .try_borrow_instruction_account(transaction_context, from_account_index)?;
    if !from.is_writable() {
        // ic_msg!(
        //     invoke_context,
        //     "Withdraw nonce account: Account {} must be writeable",
        //     from.get_key()
        // );
        return Err(InstructionError::InvalidArgument);
    }

    let state: Versions = from.get_state()?;
    let signer = match state.state() {
        State::Uninitialized => {
            if lamports > from.get_lamports() {
                // ic_msg!(
                //     invoke_context,
                //     "Withdraw nonce account: insufficient lamports {}, need {}",
                //     from.get_lamports(),
                //     lamports,
                // );
                return Err(InstructionError::InsufficientFunds);
            }
            *from.get_key()
        }
        State::Initialized(ref data) => {
            if lamports == from.get_lamports() {
                let durable_nonce =
                    DurableNonce::from_blockhash(&invoke_context.environment_config.blockhash);
                if data.durable_nonce == durable_nonce {
                    // ic_msg!(
                    //     invoke_context,
                    //     "Withdraw nonce account: nonce can only advance once per slot"
                    // );
                    return Err(SystemError::NonceBlockhashNotExpired.into());
                }
                from.set_state(&Versions::new(State::Uninitialized))?;
            } else {
                let min_balance = rent.minimum_balance(from.get_data().len());
                let amount = checked_add(lamports, min_balance)?;
                if amount > from.get_lamports() {
                    // ic_msg!(
                    //     invoke_context,
                    //     "Withdraw nonce account: insufficient lamports {}, need {}",
                    //     from.get_lamports(),
                    //     amount,
                    // );
                    return Err(InstructionError::InsufficientFunds);
                }
            }
            data.authority
        }
    };

    if !signers.contains(&signer) {
        // ic_msg!(
        //     invoke_context,
        //     "Withdraw nonce account: Account {} must sign",
        //     signer
        // );
        return Err(InstructionError::MissingRequiredSignature);
    }

    from.checked_sub_lamports(lamports)?;
    drop(from);
    let mut to = instruction_context
        .try_borrow_instruction_account(transaction_context, to_account_index)?;
    to.checked_add_lamports(lamports)?;

    Ok(())
}

pub fn initialize_nonce_account<SDK: SharedAPI>(
    account: &mut BorrowedAccount,
    nonce_authority: &Pubkey,
    rent: &Rent,
    invoke_context: &InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    if !account.is_writable() {
        // ic_msg!(
        //     invoke_context,
        //     "Initialize nonce account: Account {} must be writeable",
        //     account.get_key()
        // );
        return Err(InstructionError::InvalidArgument);
    }

    match account.get_state::<Versions>()?.state() {
        State::Uninitialized => {
            let min_balance = rent.minimum_balance(account.get_data().len());
            if account.get_lamports() < min_balance {
                // ic_msg!(
                //     invoke_context,
                //     "Initialize nonce account: insufficient lamports {}, need {}",
                //     account.get_lamports(),
                //     min_balance
                // );
                return Err(InstructionError::InsufficientFunds);
            }
            let durable_nonce =
                DurableNonce::from_blockhash(&invoke_context.environment_config.blockhash);
            let data = Data::new(
                *nonce_authority,
                durable_nonce,
                invoke_context.environment_config.lamports_per_signature,
            );
            let state = State::Initialized(data);
            account.set_state(&Versions::new(state))
        }
        State::Initialized(_) => {
            // ic_msg!(
            //     invoke_context,
            //     "Initialize nonce account: Account {} state is invalid",
            //     account.get_key()
            // );
            Err(InstructionError::InvalidAccountData)
        }
    }
}

pub fn authorize_nonce_account<SDK: SharedAPI>(
    account: &mut BorrowedAccount,
    nonce_authority: &Pubkey,
    signers: &HashSet<Pubkey>,
    _invoke_context: &InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    if !account.is_writable() {
        // ic_msg!(
        //     invoke_context,
        //     "Authorize nonce account: Account {} must be writeable",
        //     account.get_key()
        // );
        return Err(InstructionError::InvalidArgument);
    }
    match account
        .get_state::<Versions>()?
        .authorize(signers, *nonce_authority)
    {
        Ok(versions) => account.set_state(&versions),
        Err(AuthorizeNonceError::Uninitialized) => {
            // ic_msg!(
            //     invoke_context,
            //     "Authorize nonce account: Account {} state is invalid",
            //     account.get_key()
            // );
            Err(InstructionError::InvalidAccountData)
        }
        Err(AuthorizeNonceError::MissingRequiredSignature(_account_authority)) => {
            // ic_msg!(
            //     invoke_context,
            //     "Authorize nonce account: Account {} must sign",
            //     account_authority
            // );
            Err(InstructionError::MissingRequiredSignature)
        }
    }
}
