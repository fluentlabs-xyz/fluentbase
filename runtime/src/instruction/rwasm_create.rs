use crate::{
    types::{Address, Bytes, B256, U256},
    Runtime,
    RuntimeContext,
};
use fluentbase_types::{Account, ExitCode, STATE_DEPLOY};
use rwasm::{common::Trap, Caller};

pub struct RwasmCreate;

impl RwasmCreate {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        value32_offset: u32,
        input_bytecode_offset: u32,
        input_bytecode_length: u32,
        salt32_offset: u32,
        deployed_contract_address20_offset: u32,
        is_create2: u32,
    ) -> Result<i32, Trap> {
        let value32 = caller.read_memory(value32_offset, 32).to_vec();
        let input_bytecode = caller
            .read_memory(input_bytecode_offset, input_bytecode_length)
            .to_vec();
        let salt32: Vec<u8> = if is_create2 != 0 {
            vec![]
        } else {
            caller.read_memory(salt32_offset, 32).to_vec()
        };
        let exit_code = match Self::fn_impl(
            caller.data_mut(),
            &value32,
            &input_bytecode,
            &salt32,
            is_create2 != 0,
        ) {
            Ok(deployed_contract_address20) => {
                caller.write_memory(
                    deployed_contract_address20_offset,
                    deployed_contract_address20.as_slice(),
                );
                0
            }
            Err(err) => err.into_i32(),
        };
        Ok(exit_code)
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        value: &[u8],
        input_bytecode: &[u8],
        salt32: &[u8],
        is_create2: bool,
    ) -> Result<Address, ExitCode> {
        // reject static with value not zero
        if context.is_static {
            return Err(ExitCode::WriteProtection);
        }
        let value = U256::from_be_slice(value);
        // get bytecode
        let account_db = context.account_db.clone().unwrap();
        let sender_account = account_db
            .borrow_mut()
            .get_account(&context.address)
            .unwrap_or_default();
        let deployed_contract_address = if is_create2 {
            let init_code_hash = B256::from_slice(keccak_hash::keccak(input_bytecode).as_bytes());
            context
                .address
                .create2(B256::from_slice(salt32), init_code_hash)
        } else {
            context.address.create(sender_account.nonce)
        };
        // transfer funds to a new account
        account_db
            .borrow_mut()
            .transfer(&context.address, &deployed_contract_address, &value);
        // init shared runtime
        let import_linker = Runtime::<()>::new_shared_linker();
        let bytecode = Bytes::copy_from_slice(input_bytecode);
        let ctx = RuntimeContext::new(bytecode)
            .with_state(STATE_DEPLOY)
            .with_caller(context.address)
            .with_address(deployed_contract_address)
            .with_account_db(account_db.clone())
            .with_is_shared(true);
        let execution_result = Runtime::<()>::run_with_context(ctx, &import_linker)
            .map_err(|_| ExitCode::CreateError)?;
        if execution_result.data().exit_code != 0 {
            return Err(ExitCode::CreateError);
        }
        let output = execution_result.data().output();
        let code_hash = if output.len() <= 0 {
            keccak_hash::KECCAK_EMPTY
        } else {
            keccak_hash::keccak(output)
        };
        let contract_account = Account {
            balance: value,
            code_hash: B256::from_slice(code_hash.as_bytes()),
            code: Some(Bytes::copy_from_slice(output)),
            ..Default::default()
        };
        account_db
            .borrow_mut()
            .update_account(&deployed_contract_address, &contract_account);
        Ok(deployed_contract_address)
    }
}
