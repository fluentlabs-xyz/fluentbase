use crate::{
    types::{Address, U256},
    Runtime,
    RuntimeContext,
};
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::{ExitCode, STATE_MAIN};

pub struct RwasmTransact;

impl RwasmTransact {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        address20_offset: u32,
        value32_offset: u32,
        input_offset: u32,
        input_length: u32,
        return_offset: u32,
        return_length: u32,
        fuel: u32,
        is_delegate: u32,
        is_static: u32,
    ) -> Result<i32, Trap> {
        let address = caller.read_memory(address20_offset, 20).to_vec();
        let value = caller.read_memory(value32_offset, 32).to_vec();
        let input = caller.read_memory(input_offset, input_length).to_vec();
        let exit_code = match Self::fn_impl(
            caller.data_mut(),
            &address,
            &value,
            &input,
            return_length,
            fuel,
            is_delegate != 0,
            is_static != 0,
        ) {
            Ok(return_data) => {
                caller.write_memory(return_offset, &return_data);
                0
            }
            Err(err) => err.into_i32(),
        };
        Ok(exit_code)
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        address: &[u8],
        value: &[u8],
        input: &[u8],
        return_length: u32,
        fuel: u32,
        is_delegate: bool,
        is_static: bool,
    ) -> Result<Vec<u8>, ExitCode> {
        let value = U256::from_be_slice(value);
        // reject static with value not zero
        if is_delegate && !value.is_zero() {
            return Err(ExitCode::NotSupportedCall);
        } else if context.is_static && !value.is_zero() {
            return Err(ExitCode::WriteProtection);
        }
        let address = Address::from_slice(address);
        // get bytecode
        let account_db = context.account_db.clone().unwrap();
        let account = account_db
            .borrow_mut()
            .get_account(&address)
            .unwrap_or_default();
        let code = account.code.unwrap_or_default();
        // transfer funds
        if !is_delegate {
            account_db
                .borrow_mut()
                .transfer(&context.address, &address, &value);
        }
        // init shared runtime
        let import_linker = Runtime::<()>::new_shared_linker();
        let mut ctx = RuntimeContext::new(code)
            .with_input(input.to_vec())
            .with_is_static(is_static)
            .with_state(STATE_MAIN)
            .with_fuel_limit(fuel)
            .with_address(address)
            .with_account_db(account_db)
            .with_is_shared(true);
        if is_delegate {
            ctx = ctx.with_caller(context.caller);
        } else {
            ctx = ctx.with_caller(context.address);
        }
        let execution_result = Runtime::<()>::run_with_context(ctx, &import_linker)
            .map_err(|_| ExitCode::TransactError)?;
        let output = execution_result.data().output();
        if output.len() > return_length as usize {
            return Err(ExitCode::OutputOverflow);
        }
        Ok(output.clone())
    }
}
