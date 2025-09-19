use fluentbase_sdk::{Address, Bytes};
use fluentbase_sdk_testing::{try_print_utf8_error, EvmTestingContext};
use fluentbase_svm::account::{AccountSharedData, WritableAccount};
use fluentbase_svm::pubkey::Pubkey;
use fluentbase_svm_common::common::{evm_balance_from_lamports, pubkey_from_evm_address};
use fluentbase_types::PRECOMPILE_SVM_RUNTIME;
use revm::context::result::ExecutionResult;
use std::fs::File;
use std::io::Read;
use std::time::Instant;

pub fn call_with_sig(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Result<Vec<u8>, u32> {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    match &result {
        ExecutionResult::Revert {
            gas_used: _,
            output,
        } => {
            let output_vec = output.to_vec();
            try_print_utf8_error(&output_vec);
            const ERROR_OFFSET: usize = 32;
            if output_vec.len() <= ERROR_OFFSET {
                return Err(u32::MAX);
            }
            let error_data: Result<[u8; size_of::<u32>()], _> =
                output_vec[ERROR_OFFSET..].try_into();
            let Ok(error_data) = error_data else {
                return Err(u32::MAX);
            };
            let error_code = u32::from_be_bytes(error_data);
            Err(error_code)
        }
        ExecutionResult::Success { output, .. } => Ok(output.data().to_vec()),
        _ => {
            panic!("expected revert, got: {:?}", &result)
        }
    }
}

pub fn svm_deploy(
    ctx: &mut EvmTestingContext,
    deployer_address: &Address,
    program_bytes: &[u8],
    seed1: &[u8],
    payer_initial_lamports: u64,
) -> (Pubkey, Pubkey, Pubkey, Address) {
    ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);

    // setup initial accounts

    let pk_deployer1 = pubkey_from_evm_address::<true>(deployer_address);
    ctx.add_balance(
        deployer_address.clone(),
        evm_balance_from_lamports(payer_initial_lamports),
    );

    // deploy and get exec contract

    let measure = Instant::now();
    let program_bytes_vec = program_bytes.to_vec();
    let (contract_address, _gas) =
        ctx.deploy_evm_tx_with_gas(deployer_address.clone(), program_bytes_vec.into());
    println!("deploy took: {:.2?}", measure.elapsed());

    let pk_contract = pubkey_from_evm_address::<true>(&contract_address);

    let seeds = &[seed1, pk_deployer1.as_ref()];
    let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_contract);

    (pk_deployer1, pk_contract, pk_new, contract_address)
}

pub fn load_program_account_from_elf_file(loader_id: &Pubkey, path: &str) -> AccountSharedData {
    let mut file = File::open(path).expect("file open failed");
    let mut elf = Vec::new();
    file.read_to_end(&mut elf).unwrap();
    let mut program_account = AccountSharedData::new(0, 0, loader_id);
    program_account.set_data(elf);
    program_account.set_executable(true);
    program_account
}
