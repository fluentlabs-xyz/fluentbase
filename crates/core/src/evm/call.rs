use crate::{
    debug_log,
    fluentbase_sdk::LowLevelAPI,
    helpers::{exec_evm_bytecode, exit_code_from_evm_error},
};
use fluentbase_sdk::{AccountManager, ContextReader, EvmCallMethodInput, EvmCallMethodOutput};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    analysis::to_analysed,
    primitives::Bytecode,
    return_ok,
    Contract,
    InstructionResult,
};

pub fn _evm_call<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    debug_log!("ecl(_evm_call): start. gas_limit {}", input.gas_limit);

    // call depth check
    if input.depth > 1024 {
        return EvmCallMethodOutput::from_exit_code(ExitCode::CallDepthOverflow)
            .with_gas(input.gas_limit, 0);
    }

    // read caller and callee
    let (mut caller_account, _) = am.account(cr.contract_caller());
    let (mut callee_account, _) = am.account(cr.contract_address());

    // create new checkpoint position in the journal
    let checkpoint = am.checkpoint();

    // transfer funds from caller to callee
    if !input.value.is_zero() {
        debug_log!(
            "ecm(_evm_call): transfer from={} to={} value={}",
            caller_account.address,
            callee_account.address,
            hex::encode(input.value.to_be_bytes::<32>())
        );
    }

    if caller_account.address != callee_account.address {
        // do transfer from caller to callee
        match am.transfer(&mut caller_account, &mut callee_account, input.value) {
            Ok(_) => {}
            Err(exit_code) => {
                return EvmCallMethodOutput::from_exit_code(exit_code).with_gas(input.gas_limit, 0);
            }
        }
        // write current account state before doing nested calls
        am.write_account(&caller_account);
        am.write_account(&callee_account);
    } else {
        // what if self-transfer amount exceeds our balance?
        if input.value > caller_account.balance {
            return EvmCallMethodOutput::from_exit_code(ExitCode::InsufficientBalance)
                .with_gas(input.gas_limit, 0);
        }
        // write only one account's state since caller equals callee
        am.write_account(&caller_account);
    }

    // check is it precompile
    if let Some(result) = am.precompile(&input.callee, &input.input, input.gas_limit) {
        if ExitCode::from(result.exit_code).is_ok() {
            am.commit();
        } else {
            am.rollback(checkpoint);
        }
        return result;
    }

    // take right bytecode depending on context params
    let (source_hash, source_bytecode) = if input.callee != callee_account.address {
        let (code_account, _) = am.account(input.callee);
        (
            code_account.source_code_hash,
            am.preimage(&code_account.source_code_hash),
        )
    } else {
        (
            callee_account.source_code_hash,
            am.preimage(&callee_account.source_code_hash),
        )
    };
    // load bytecode and convert it to analysed (we can safely unwrap here)
    let bytecode = to_analysed(Bytecode::new_raw(source_bytecode));

    // if bytecode is empty then commit result and return empty buffer
    if bytecode.is_empty() {
        am.commit();
        debug_log!("ecl(_evm_call): empty bytecode exit_code=Ok");
        return EvmCallMethodOutput::from_exit_code(ExitCode::Ok).with_gas(input.gas_limit, 0);
    }

    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.input,
        hash: Some(source_hash),
        bytecode,
        // we don't take contract callee, because callee refers to address with bytecode
        target_address: cr.contract_address(),
        call_value: cr.contract_value(),
        caller: caller_account.address,
    };
    let result = exec_evm_bytecode(
        cr,
        am,
        contract,
        input.gas_limit,
        cr.contract_is_static(),
        input.depth,
    );

    if matches!(result.result, return_ok!()) {
        am.commit();
    } else {
        am.rollback(checkpoint);
    }

    let exit_code = exit_code_from_evm_error(result.result);

    debug_log!(
        "ecl(_evm_call): return exit_code={} gas_remaining={} gas_refund={}",
        exit_code,
        result.gas.remaining(),
        result.gas.refunded()
    );
    EvmCallMethodOutput {
        output: result.output,
        exit_code: exit_code.into_i32(),
        gas_remaining: result.gas.remaining(),
        gas_refund: result.gas.refunded(),
    }
}
