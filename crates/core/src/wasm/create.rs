use crate::{debug_log, helpers::wasm2rwasm};
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    types::{WasmCreateMethodInput, WasmCreateMethodOutput},
    Account,
    AccountStatus,
    NativeAPI,
    SovereignAPI,
};
use fluentbase_types::{Bytes, ExitCode, Fuel, STATE_DEPLOY};
use revm_primitives::WASM_MAX_CODE_SIZE;

pub fn _wasm_create<SDK: SovereignAPI>(
    sdk: &mut SDK,
    input: WasmCreateMethodInput,
) -> WasmCreateMethodOutput {
    debug_log!(sdk, "_wasm_create start");

    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"

    // check write protection
    if input.is_static {
        debug_log!(
            sdk,
            "_wasm_create return: Err: exit_code: {}",
            ExitCode::WriteProtection
        );
        return WasmCreateMethodOutput::from_exit_code(ExitCode::WriteProtection);
    }

    // code length can't exceed max constructor limit
    if input.bytecode.len() > WASM_MAX_CODE_SIZE {
        debug_log!(
            sdk,
            "_wasm_create return: Err: exit_code: {}",
            ExitCode::ContractSizeLimit
        );
        return WasmCreateMethodOutput::from_exit_code(ExitCode::ContractSizeLimit);
    }

    let source_code_hash = sdk.native_sdk().keccak256(input.bytecode.as_ref());

    // load deployer and contract accounts
    let (mut deployer_account, _) = sdk.account(&input.caller);

    // create an account
    let (mut contract_account, checkpoint) = match Account::create_account_checkpoint(
        sdk,
        &mut deployer_account,
        input.value,
        input.salt.map(|salt| (salt, source_code_hash)),
    ) {
        Ok(result) => result,
        Err(exit_code) => {
            return WasmCreateMethodOutput::from_exit_code(exit_code);
        }
    };
    if !input.value.is_zero() {
        debug_log!(
            sdk,
            "ecm(_wasm_create): transfer from={} to={} value={}",
            contract_account.address,
            contract_account.address,
            hex::encode(input.value.to_be_bytes::<32>())
        );
    }

    debug_log!(
        sdk,
        "ecl(_wasm_create): creating account={} balance={}",
        contract_account.address,
        hex::encode(contract_account.balance.to_be_bytes::<32>())
    );

    // translate WASM to rWASM
    let rwasm_bytecode = match wasm2rwasm(&input.bytecode) {
        Ok(result) => result,
        Err(exit_code) => {
            sdk.rollback(checkpoint);
            debug_log!(sdk, "_wasm_create return: panic: exit_code: {}", exit_code);
            return WasmCreateMethodOutput::from_exit_code(exit_code);
        }
    };
    // let exit_code = LowLevelSDK::wasm_to_rwasm(
    //     input.bytecode.as_ptr(),
    //     input.bytecode.len() as u32,
    //     core::ptr::null_mut(),
    //     0,
    // );
    // if exit_code != ExitCode::Ok.into_i32() {
    //     debug_log(&format!(
    //         "_wasm_create return: panic: exit_code: {}",
    //         exit_code
    //     ));
    //     panic!("wasm create failed, exit code: {}", exit_code);
    // }
    // let rwasm_bytecode_len = LowLevelSDK::sys_output_size();
    // let mut rwasm_bytecode = vec![0u8; rwasm_bytecode_len as usize];
    // LowLevelSDK::sys_read_output(rwasm_bytecode.as_mut_ptr(), 0, rwasm_bytecode_len);

    // write deployer to the trie
    sdk.write_account(deployer_account, AccountStatus::Modified);

    // write contract to the trie
    contract_account.update_bytecode(sdk, input.bytecode, None, rwasm_bytecode.into(), None);

    let mut fuel = Fuel::from(input.gas_limit);
    let (_, exit_code) = sdk.context_call(&contract_account.address, &mut fuel, &[], STATE_DEPLOY);
    // if call is not success set deployed address to zero
    if exit_code != ExitCode::Ok {
        sdk.rollback(checkpoint);
        debug_log!(sdk, "_wasm_create return: Err: ExitCode::TransactError");
        return WasmCreateMethodOutput::from_exit_code(ExitCode::from(exit_code));
    }

    debug_log!(
        sdk,
        "_wasm_create return: Ok: contract_account.address {}",
        contract_account.address
    );

    // commit all changes made
    sdk.commit();

    WasmCreateMethodOutput {
        output: Bytes::new(),
        address: Some(contract_account.address),
        exit_code: exit_code.into_i32(),
        gas: fuel.into(),
        gas_refund: 0,
    }
}
