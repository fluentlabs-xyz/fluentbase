use crate::{
    account::{ReadableAccount, WritableAccount},
    context::{IndexOfAccount, InstructionAccount, InvokeContext},
    precompiles::is_precompile,
    solana_program::{message::SanitizedMessage, sysvar::instructions},
};
use alloc::vec::Vec;
use fluentbase_sdk::SharedAPI;
use hashbrown::HashMap;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use solana_transaction_error::TransactionError;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct MessageProcessor {}

impl MessageProcessor {
    pub fn process_message<'a, SDK: SharedAPI>(
        message: &SanitizedMessage,
        program_indices: &[Vec<IndexOfAccount>],
        invoke_context: &mut InvokeContext<'_, SDK>,
    ) -> Result<(), TransactionError> {
        debug_assert_eq!(program_indices.len(), message.instructions().len());
        for (instruction_index, ((program_id, instruction), program_indices)) in message
            .program_instructions_iter()
            .zip(program_indices.iter())
            .enumerate()
        {
            let is_precompile = is_precompile(program_id, |id| {
                invoke_context.environment_config.feature_set.is_active(id)
            });

            // Fixup the special instructions key if present
            // before the account pre-values are taken care of
            if let Some(account_index) = invoke_context
                .transaction_context
                .find_index_of_account(&instructions::id())
            {
                let mut mut_account_ref = invoke_context
                    .transaction_context
                    .get_account_at_index(account_index)
                    .map_err(|_| TransactionError::InvalidAccountIndex)?
                    .borrow_mut();
                instructions::store_current_index(
                    mut_account_ref.data_as_mut_slice(),
                    instruction_index as u16,
                );
            }

            let mut instruction_accounts = Vec::with_capacity(instruction.accounts.len());
            for (instruction_account_index, index_in_transaction) in
                instruction.accounts.iter().enumerate()
            {
                let index_in_callee = instruction
                    .accounts
                    .get(0..instruction_account_index)
                    .ok_or(TransactionError::InvalidAccountIndex)?
                    .iter()
                    .position(|account_index| account_index == index_in_transaction)
                    .unwrap_or(instruction_account_index)
                    as IndexOfAccount;
                let index_in_transaction = *index_in_transaction as usize;
                let instruction_account = InstructionAccount {
                    index_in_transaction: index_in_transaction as IndexOfAccount,
                    index_in_caller: index_in_transaction as IndexOfAccount,
                    index_in_callee,
                    is_signer: message.is_signer(index_in_transaction),
                    is_writable: message.is_writable(index_in_transaction),
                };
                instruction_accounts.push(instruction_account);
            }

            for instruction_account in instruction_accounts.iter() {
                let instruction_account_key = invoke_context
                    .transaction_context
                    .get_key_of_account_at_index(instruction_account.index_in_transaction)
                    .expect("instruction account key must always exist");
                let account_data = invoke_context
                    .transaction_context
                    .get_account_at_index(instruction_account.index_in_transaction)
                    .expect("instruction account must always exist");
            }

            let result = if is_precompile {
                invoke_context
                    .transaction_context
                    .get_next_instruction_context()
                    .map(|instruction_context| {
                        instruction_context.configure(
                            program_indices,
                            &instruction_accounts,
                            &instruction.data,
                        );
                    })
                    .and_then(|_| {
                        invoke_context.transaction_context.push()?;
                        invoke_context.transaction_context.pop()
                    })
            } else {
                let result = invoke_context.process_instruction(
                    &instruction.data,
                    &instruction_accounts,
                    program_indices,
                );
                result
            };

            result
                .map_err(|err| TransactionError::InstructionError(instruction_index as u8, err))?;
        }
        Ok(())
    }
}
