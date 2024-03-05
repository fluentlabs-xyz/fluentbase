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
        init_bytecode_offset: u32,
        init_bytecode_length: u32,
        salt32_offset: u32,
        deployed_contract_address20_offset: u32,
        is_create2: u32,
    ) -> Result<i32, Trap> {
        let value32 = caller.read_memory(value32_offset, 32).to_vec();
        let init_bytecode = caller
            .read_memory(init_bytecode_offset, init_bytecode_length)
            .to_vec();
        let salt32 = if is_create2 != 0 {
            Some(caller.read_memory(salt32_offset, 32).to_vec())
        } else {
            None
        };
        let exit_code = match Self::fn_impl(
            caller.data_mut(),
            &value32,
            &init_bytecode,
            salt32.as_ref().map(|v| v.as_slice()),
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
        init_bytecode: &[u8],
        salt32: Option<&[u8]>,
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
        let dc_address = if let Some(salt32) = salt32 {
            let init_code_hash = B256::from_slice(keccak_hash::keccak(init_bytecode).as_bytes());
            context
                .address
                .create2(B256::from_slice(salt32), init_code_hash)
        } else {
            context.address.create(sender_account.nonce)
        };
        // transfer funds to a new account
        if !value.is_zero() {
            account_db
                .borrow_mut()
                .transfer(&context.address, &dc_address, &value);
        }
        // init shared runtime
        let import_linker = Runtime::<()>::new_shared_linker();
        let bytecode = Bytes::copy_from_slice(init_bytecode);
        let mut ctx = RuntimeContext::new(bytecode);
        ctx.with_state(STATE_DEPLOY)
            .with_caller(context.address)
            .with_address(dc_address)
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
        let dc_account = Account {
            balance: value,
            code_hash: B256::from_slice(code_hash.as_bytes()),
            code: Some(Bytes::copy_from_slice(output)),
            ..Default::default()
        };
        account_db
            .borrow_mut()
            .update_account(&dc_address, &dc_account);
        Ok(dc_address)
    }
}
