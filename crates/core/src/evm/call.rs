use crate::{
    debug_log,
    helpers::{exec_evm_bytecode, exit_code_from_evm_error},
};
use fluentbase_sdk::{
    types::{EvmCallMethodInput, EvmCallMethodOutput},
    AccountStatus,
    ContextReader,
    SovereignAPI,
};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    analysis::to_analysed,
    primitives::Bytecode,
    return_ok,
    Contract,
    InstructionResult,
};

pub fn _evm_call<CTX: ContextReader, SDK: SovereignAPI>(
    ctx: &CTX,
    sdk: &SDK,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    debug_log!(sdk, "ecl(_evm_call): start. gas_limit {}", input.gas_limit);

    // call depth check
    if input.depth > 1024 {
        return EvmCallMethodOutput::from_exit_code(ExitCode::CallDepthOverflow)
            .with_gas(input.gas_limit, 0);
    }

    // read caller and callee
    let (mut caller_account, _) = sdk.account(&ctx.contract_caller());
    let (mut callee_account, _) = sdk.account(&ctx.contract_address());

    // create a new checkpoint position in the journal
    let checkpoint = sdk.checkpoint();

    // transfer funds from caller to callee
    if !input.value.is_zero() {
        debug_log!(
            sdk,
            "ecm(_evm_call): transfer from={} to={} value={}",
            caller_account.address,
            callee_account.address,
            hex::encode(input.value.to_be_bytes::<32>())
        );
    }

    if caller_account.address != callee_account.address {
        // do transfer from caller to callee
        match sdk.transfer(&mut caller_account, &mut callee_account, input.value) {
            Ok(_) => {}
            Err(exit_code) => {
                return EvmCallMethodOutput::from_exit_code(exit_code).with_gas(input.gas_limit, 0);
            }
        }
        // write current account state before doing nested calls
        sdk.write_account(&caller_account, AccountStatus::Modified);
        sdk.write_account(&callee_account, AccountStatus::Modified);
    } else {
        // what if self-transfer amount exceeds our balance?
        if input.value > caller_account.balance {
            let res = EvmCallMethodOutput::from_exit_code(ExitCode::InsufficientBalance)
                .with_gas(input.gas_limit, 0);
            return res;
        }
        // write only one account's state since caller equals callee
        sdk.write_account(&caller_account, AccountStatus::Modified);
    }

    // check is it precompile
    if let Some(result) = sdk.precompile(&input.callee, &input.input, input.gas_limit) {
        let result = EvmCallMethodOutput {
            output: result.0,
            exit_code: result.1.into_i32(),
            gas_remaining: result.2,
            gas_refund: result.3,
        };
        if ExitCode::from(result.exit_code).is_ok() {
            sdk.commit();
        } else {
            sdk.rollback(checkpoint);
        }
        return result;
    }

    // take right bytecode depending on context params
    let (source_hash, source_bytecode) = if input.callee != callee_account.address {
        let (code_account, _) = sdk.account(&input.callee);
        (
            code_account.source_code_hash,
            sdk.preimage(&code_account.source_code_hash),
        )
    } else {
        (
            callee_account.source_code_hash,
            sdk.preimage(&callee_account.source_code_hash),
        )
    };
    debug_log!(
        sdk,
        "ecl(_evm_call): source_bytecode: {}",
        hex::encode(&source_bytecode)
    );
    // load bytecode and convert it to analysed (we can safely unwrap here)
    let bytecode = to_analysed(Bytecode::new_raw(source_bytecode));

    // if bytecode is empty, then commit result and return empty buffer
    if bytecode.is_empty() {
        sdk.commit();
        debug_log!(sdk, "ecl(_evm_call): empty bytecode exit_code=Ok");
        return EvmCallMethodOutput::from_exit_code(ExitCode::Ok).with_gas(input.gas_limit, 0);
    }

    // initiate contract instance and pass it to interpreter for and EVM transition
    let contract = Contract {
        input: input.input,
        hash: Some(source_hash),
        bytecode,
        // we don't take contract callee, because callee refers to address with bytecode
        target_address: ctx.contract_address(),
        call_value: ctx.contract_value(),
        caller: caller_account.address,
    };
    let result = exec_evm_bytecode(
        ctx,
        sdk,
        contract,
        input.gas_limit,
        ctx.contract_is_static(),
        input.depth,
    );

    if matches!(result.result, return_ok!()) {
        sdk.commit();
    } else {
        sdk.rollback(checkpoint);
    }

    let exit_code = exit_code_from_evm_error(result.result);

    debug_log!(
        sdk,
        "ecl(_evm_call): return exit_code={} gas_remaining={} spent={} gas_refund={}",
        exit_code,
        result.gas.remaining(),
        result.gas.spent(),
        result.gas.refunded()
    );
    EvmCallMethodOutput {
        output: result.output,
        exit_code: exit_code.into_i32(),
        gas_remaining: result.gas.remaining(),
        gas_refund: result.gas.refunded(),
    }
}
