use crate::{
    debug_log,
    helpers::{exec_evm_bytecode, exit_code_from_evm_error},
};
use fluentbase_sdk::{
    types::{EvmCreateMethodInput, EvmCreateMethodOutput},
    Account,
    ContextReader,
    SharedAPI,
    SovereignAPI,
};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    analysis::to_analysed,
    gas,
    primitives::{Bytecode, Bytes},
    return_ok,
    Contract,
    InstructionResult,
    MAX_CODE_SIZE,
};
use revm_primitives::MAX_INITCODE_SIZE;

pub fn _evm_create<CTX: ContextReader, SDK: SovereignAPI>(
    ctx: &CTX,
    sdk: &SDK,
    input: EvmCreateMethodInput,
) -> EvmCreateMethodOutput {
    debug_log!(
        sdk,
        "ecl(_evm_create): start. gas_limit {}",
        input.gas_limit
    );

    // check write protection
    let is_static = ctx.contract_is_static();
    if is_static {
        debug_log!(
            sdk,
            "ecl(_evm_create): return: Err: exit_code: {}",
            ExitCode::WriteProtection
        );
        return EvmCreateMethodOutput::from_exit_code(ExitCode::WriteProtection)
            .with_gas(input.gas_limit, 0);
    }

    // load deployer and contract accounts
    let caller_address = ctx.contract_caller();
    let (mut caller_account, _) = sdk.account(&caller_address);

    // call depth check
    if input.depth > 1024 {
        return EvmCreateMethodOutput::from_exit_code(ExitCode::CallDepthOverflow)
            .with_gas(input.gas_limit, 0);
    }

    // check init max code size for EIP-3860
    if input.bytecode.len() > MAX_INITCODE_SIZE {
        return EvmCreateMethodOutput::from_exit_code(ExitCode::ContractSizeLimit)
            .with_gas(input.gas_limit, 0);
    }

    // calc source code hash
    let mut source_code_hash = SDK::keccak256(input.bytecode.as_ref());

    // create an account
    let salt_hash = input.salt.map(|salt| (salt, source_code_hash));
    let (contract_account, checkpoint) = match Account::create_account_checkpoint(
        sdk,
        &mut caller_account,
        input.value,
        salt_hash,
    ) {
        Ok(result) => result,
        Err(err) => {
            return EvmCreateMethodOutput::from_exit_code(err).with_gas(input.gas_limit, 0);
        }
    };
    if !input.value.is_zero() {
        debug_log!(
            sdk,
            "ecm(_evm_create): transfer from={} to={} value={}",
            caller_account.address,
            contract_account.address,
            hex::encode(input.value.to_be_bytes::<32>())
        )
    }

    debug_log!(
        sdk,
        "ecl(_evm_create): creating account={} balance={}",
        contract_account.address,
        hex::encode(contract_account.balance.to_be_bytes::<32>())
    );

    let analyzed_bytecode = to_analysed(Bytecode::new_raw(input.bytecode.into()));

    let contract = Contract {
        input: Bytes::new(),
        bytecode: analyzed_bytecode,
        hash: Some(source_code_hash),
        target_address: contract_account.address,
        caller: caller_address,
        call_value: input.value,
    };

    let mut result = exec_evm_bytecode(ctx, sdk, contract, input.gas_limit, is_static, input.depth);

    if !matches!(result.result, return_ok!()) {
        sdk.rollback(checkpoint);
        debug_log!(sdk, "ecl(_evm_create): return: Err: {:?}", result.result);
        return EvmCreateMethodOutput::from_exit_code(exit_code_from_evm_error(result.result))
            .with_output(result.output)
            .with_gas(result.gas.remaining(), result.gas.refunded());
    }
    if !result.output.is_empty() && result.output.first() == Some(&0xEF) {
        sdk.rollback(checkpoint);
        debug_log!(sdk, "ecl(_evm_create): return: Err: {:?}", result.result);
        return EvmCreateMethodOutput::from_exit_code(ExitCode::CreateContractStartingWithEF)
            .with_output(result.output)
            .with_gas(result.gas.remaining(), result.gas.refunded());
    }
    if result.output.len() > MAX_CODE_SIZE {
        sdk.rollback(checkpoint);
        debug_log!(sdk, "ecl(_evm_create): return: Err: {:?}", result.result);
        return EvmCreateMethodOutput::from_exit_code(ExitCode::ContractSizeLimit)
            .with_output(result.output)
            .with_gas(result.gas.remaining(), result.gas.refunded());
    }

    // record gas for each created byte
    let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
    if !result.gas.record_cost(gas_for_code) {
        sdk.rollback(checkpoint);
        return EvmCreateMethodOutput::from_exit_code(ExitCode::OutOfGas)
            .with_output(result.output)
            .with_gas(result.gas.remaining(), result.gas.refunded());
    }

    // write callee changes to a database (lets keep rWASM part empty for now since universal loader
    // is not ready yet)
    let (mut contract_account, _) = sdk.account(&contract_account.address);
    contract_account.update_bytecode(sdk, &result.output, None, &Bytes::new(), None);

    debug_log!(
        sdk,
        "ecl(_evm_create): return: Ok: callee_account.address: {}",
        contract_account.address
    );

    // commit all changes made
    sdk.commit();

    return EvmCreateMethodOutput::from_exit_code(ExitCode::Ok)
        .with_output(result.output)
        .with_gas(result.gas.remaining(), result.gas.refunded())
        .with_address(contract_account.address);
}
