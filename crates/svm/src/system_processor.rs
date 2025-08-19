use crate::{
    account::{AccountSharedData, BorrowedAccount, ReadableAccount},
    context::{IndexOfAccount, InstructionContext, InvokeContext, TransactionContext},
    declare_process_instruction,
    pubkey::Pubkey,
    system_instruction::{SystemError, SystemInstruction, MAX_PERMITTED_DATA_LENGTH},
    system_program,
};
use alloc::boxed::Box;
use fluentbase_sdk::SharedAPI;
use hashbrown::HashSet;
use solana_bincode::deserialize;
use solana_instruction::error::InstructionError;

// represents an address that may or may not have been generated from a seed
#[derive(PartialEq, Eq, Default, Debug)]
pub struct Address {
    address: Pubkey,
    base: Option<Pubkey>,
}

impl Address {
    pub fn is_signer(&self, signers: &HashSet<Pubkey>) -> bool {
        if let Some(base) = self.base {
            signers.contains(&base)
        } else {
            signers.contains(&self.address)
        }
    }
    pub fn create<SDK: SharedAPI>(
        address: &Pubkey,
        with_seed: Option<(&Pubkey, &str, &Pubkey)>,
        _invoke_context: &InvokeContext<SDK>,
    ) -> Result<Self, InstructionError> {
        let base = if let Some((base, seed, owner)) = with_seed {
            let address_with_seed = Pubkey::create_with_seed(base, seed, owner);
            let address_with_seed = match address_with_seed {
                Ok(v) => v,
                Err(err) => {
                    return Err(InstructionError::from(err));
                }
            };
            // re-derive the address, must match the supplied address
            if *address != address_with_seed {
                return Err(SystemError::AddressWithSeedMismatch.into());
            }
            Some(*base)
        } else {
            None
        };

        Ok(Self {
            address: *address,
            base,
        })
    }
}

fn allocate<SDK: SharedAPI>(
    account: &mut BorrowedAccount,
    address: &Address,
    space: u64,
    signers: &HashSet<Pubkey>,
    _invoke_context: &InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    if !address.is_signer(signers) {
        // ic_msg!(
        //     invoke_context,
        //     "Allocate: 'to' account {:?} must sign",
        //     address
        // );
        return Err(InstructionError::MissingRequiredSignature);
    }

    // if it looks like the `to` account is already in use, bail
    //   (note that the id check is also enforced by message_processor)
    if !account.get_data().is_empty() || !system_program::check_id(account.get_owner()) {
        // ic_msg!(
        //     invoke_context,
        //     "Allocate: account {:?} already in use",
        //     address
        // );
        return Err(SystemError::AccountAlreadyInUse.into());
    }

    if space > MAX_PERMITTED_DATA_LENGTH {
        // ic_msg!(
        //     invoke_context,
        //     "Allocate: requested {}, max allowed {}",
        //     space,
        //     MAX_PERMITTED_DATA_LENGTH
        // );
        return Err(SystemError::InvalidAccountDataLength.into());
    }

    account.set_data_length(space as usize)?;

    Ok(())
}

fn assign<SDK: SharedAPI>(
    account: &mut BorrowedAccount,
    address: &Address,
    owner: &Pubkey,
    signers: &HashSet<Pubkey>,
    _invoke_context: &InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    // no work to do, just return
    if account.get_owner() == owner {
        return Ok(());
    }

    if !address.is_signer(signers) {
        // ic_msg!(invoke_context, "Assign: account {:?} must sign", address);
        return Err(InstructionError::MissingRequiredSignature);
    }

    account.set_owner(&owner.to_bytes())
}

fn allocate_and_assign<SDK: SharedAPI>(
    to: &mut BorrowedAccount,
    to_address: &Address,
    space: u64,
    owner: &Pubkey,
    signers: &HashSet<Pubkey>,
    invoke_context: &InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    allocate(to, to_address, space, signers, invoke_context)?;
    assign(to, to_address, owner, signers, invoke_context)
}

#[allow(clippy::too_many_arguments)]
fn create_account<SDK: SharedAPI>(
    from_account_index: IndexOfAccount,
    to_account_index: IndexOfAccount,
    to_address: &Address,
    lamports: u64,
    space: u64,
    owner: &Pubkey,
    signers: &HashSet<Pubkey>,
    invoke_context: &InvokeContext<SDK>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
) -> Result<(), InstructionError> {
    // if it looks like the `to` account is already in use, bail
    {
        let mut to = instruction_context
            .try_borrow_instruction_account(transaction_context, to_account_index)?;

        if to.get_lamports() > 0 {
            // ic_msg!(
            //     invoke_context,
            //     "Create Account: account {:?} already in use",
            //     to_address
            // );

            return Err(SystemError::AccountAlreadyInUse.into());
        }

        allocate_and_assign(&mut to, to_address, space, owner, signers, invoke_context)?;
    }

    transfer(
        from_account_index,
        to_account_index,
        lamports,
        invoke_context,
        transaction_context,
        instruction_context,
    )
}

fn transfer_verified<SDK: SharedAPI>(
    from_account_index: IndexOfAccount,
    to_account_index: IndexOfAccount,
    lamports: u64,
    _invoke_context: &InvokeContext<SDK>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
) -> Result<(), InstructionError> {
    let mut from = instruction_context
        .try_borrow_instruction_account(transaction_context, from_account_index)?;
    if !from.get_data().is_empty() {
        // ic_msg!(invoke_context, "Transfer: `from` must not carry data");

        return Err(InstructionError::InvalidArgument);
    }
    let from_lamports = from.get_lamports();
    if lamports > from_lamports {
        // ic_msg!(
        //     invoke_context,
        //     "Transfer: insufficient lamports {}, need {}",
        //     from.get_lamports(),
        //     lamports
        // );

        return Err(SystemError::ResultWithNegativeLamports.into());
    }

    from.checked_sub_lamports(lamports)?;

    drop(from);

    let mut to = instruction_context
        .try_borrow_instruction_account(transaction_context, to_account_index)?;

    to.checked_add_lamports(lamports)?;

    Ok(())
}

fn transfer<SDK: SharedAPI>(
    from_account_index: IndexOfAccount,
    to_account_index: IndexOfAccount,
    lamports: u64,
    invoke_context: &InvokeContext<SDK>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
) -> Result<(), InstructionError> {
    if !instruction_context.is_instruction_account_signer(from_account_index)? {
        // ic_msg!(
        //     invoke_context,
        //     "Transfer: `from` account {} must sign",
        //     transaction_context.get_key_of_account_at_index(
        //         instruction_context
        //             .get_index_of_instruction_account_in_transaction(from_account_index)?,
        //     )?,
        // );

        return Err(InstructionError::MissingRequiredSignature);
    }

    transfer_verified(
        from_account_index,
        to_account_index,
        lamports,
        invoke_context,
        transaction_context,
        instruction_context,
    )
}

fn transfer_with_seed<SDK: SharedAPI>(
    from_account_index: IndexOfAccount,
    from_base_account_index: IndexOfAccount,
    from_seed: &str,
    from_owner: &Pubkey,
    to_account_index: IndexOfAccount,
    lamports: u64,
    invoke_context: &InvokeContext<SDK>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
) -> Result<(), InstructionError> {
    if !instruction_context.is_instruction_account_signer(from_base_account_index)? {
        return Err(InstructionError::MissingRequiredSignature);
    }
    let address_from_seed = Pubkey::create_with_seed(
        transaction_context.get_key_of_account_at_index(
            instruction_context
                .get_index_of_instruction_account_in_transaction(from_base_account_index)?,
        )?,
        from_seed,
        from_owner,
    )?;

    let from_key = transaction_context.get_key_of_account_at_index(
        instruction_context.get_index_of_instruction_account_in_transaction(from_account_index)?,
    )?;
    if *from_key != address_from_seed {
        return Err(SystemError::AddressWithSeedMismatch.into());
    }

    transfer_verified(
        from_account_index,
        to_account_index,
        lamports,
        invoke_context,
        transaction_context,
        instruction_context,
    )
}

declare_process_instruction!(Entrypoint<SDK: SharedAPI>, DEFAULT_COMPUTE_UNITS, |invoke_context| {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let instruction_data = instruction_context.get_instruction_data();
    let instruction = deserialize(instruction_data)
        .map_err(|_| InstructionError::InvalidInstructionData);
    let instruction = instruction?;

    let signers = instruction_context.get_signers(transaction_context)?;
    match instruction {
        SystemInstruction::CreateAccount {
            lamports,
            space,
            owner,
        } => {

            instruction_context.check_number_of_instruction_accounts(2)?;

            let to_address = Address::create(
                transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(1)?,
                )?,
                None,
                invoke_context,
            )?;
            let result = create_account(
                0,
                1,
                &to_address,
                lamports,
                space,
                &owner,
                &signers,
                invoke_context,
                transaction_context,
                instruction_context,
            );
            result
        }
        SystemInstruction::CreateAccountWithSeed {
            base,
            seed,
            lamports,
            space,
            owner,
        } => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let to_address = Address::create(
                transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(1)?,
                )?,
                Some((&base, &seed, &owner)),
                invoke_context,
            )?;
            let result = create_account(
                0,
                1,
                &to_address,
                lamports,
                space,
                &owner,
                &signers,
                invoke_context,
                transaction_context,
                instruction_context,
            );
            result
        }
        SystemInstruction::Assign { owner } => {
            instruction_context.check_number_of_instruction_accounts(1)?;
            let mut account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let address = Address::create(
                transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(0)?,
                )?,
                None,
                invoke_context,
            )?;
            let result = assign(&mut account, &address, &owner, &signers, invoke_context);
            result
        }
        SystemInstruction::Transfer { lamports } => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let result = transfer(
                0,
                1,
                lamports,
                invoke_context,
                transaction_context,
                instruction_context,
            );
            result
        }
        SystemInstruction::TransferWithSeed {
            lamports,
            from_seed,
            from_owner,
        } => {
            instruction_context.check_number_of_instruction_accounts(3)?;
            let result = transfer_with_seed(
                0,
                1,
                &from_seed,
                &from_owner,
                2,
                lamports,
                invoke_context,
                transaction_context,
                instruction_context,
            );
            result
        }
        SystemInstruction::AdvanceNonceAccount => {
            panic!("not supported")
        }
        SystemInstruction::WithdrawNonceAccount(_lamports) => {
            panic!("not supported")
        }
        SystemInstruction::InitializeNonceAccount(_authorized) => {
            panic!("not supported")
        }
        SystemInstruction::AuthorizeNonceAccount(_nonce_authority) => {
            panic!("not supported")
        }
        SystemInstruction::UpgradeNonceAccount => {
            panic!("not supported")
        }
        SystemInstruction::Allocate { space } => {
            instruction_context.check_number_of_instruction_accounts(1)?;
            let mut account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let address = Address::create(
                transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(0)?,
                )?,
                None,
                invoke_context,
            )?;
            let result = allocate(&mut account, &address, space, &signers, invoke_context);
            result
        }
        SystemInstruction::AllocateWithSeed {
            base,
            seed,
            space,
            owner,
        } => {
            instruction_context.check_number_of_instruction_accounts(1)?;
            let mut account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let address = Address::create(
                transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(0)?,
                )?,
                Some((&base, &seed, &owner)),
                invoke_context,
            )?;
            let result = allocate_and_assign(
                &mut account,
                &address,
                space,
                &owner,
                &signers,
                invoke_context,
            );
            result
        }
        SystemInstruction::AssignWithSeed { base, seed, owner } => {
            instruction_context.check_number_of_instruction_accounts(1)?;
            let mut account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let address = Address::create(
                transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(0)?,
                )?,
                Some((&base, &seed, &owner)),
                invoke_context,
            )?;
            let result = assign(&mut account, &address, &owner, &signers, invoke_context);
            result
        }
    }
});

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SystemAccountKind {
    System,
    // Nonce,
}

pub fn get_system_account_kind(account: &AccountSharedData) -> Option<SystemAccountKind> {
    if system_program::check_id(account.owner()) {
        if account.data().is_empty() {
            Some(SystemAccountKind::System)
        // } else if account.data().len() == nonce::state::State::size() {
        //     let nonce_versions: Versions = account.state().ok()?;
        //     match nonce_versions.state() {
        //         nonce::state::State::Uninitialized => None,
        //         nonce::state::State::Initialized(_) => Some(SystemAccountKind::Nonce),
        //     }
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::{pubkey::Pubkey, system_processor::Address};

    impl From<Pubkey> for Address {
        fn from(address: Pubkey) -> Self {
            Self {
                address,
                base: None,
            }
        }
    }
}
