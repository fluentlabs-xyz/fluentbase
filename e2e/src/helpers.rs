use fluentbase_sdk::{Address, Bytes, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME};
// use fluentbase_svm::{
//     account::AccountSharedData,
//     account_info::AccountInfo,
//     helpers::{storage_read_account_data, storage_write_account_data},
//     pubkey::Pubkey,
//     solana_program::instruction::AccountMeta,
// };
use fluentbase_testing::{try_print_utf8_error, EvmTestingContext};
use revm::context::result::ExecutionResult;
// use solana_program_pack::Pack;

pub fn call_with_sig(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Result<Vec<u8>, (u32, Vec<u8>)> {
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
                return Err((u32::MAX, output_vec));
            }
            let error_data: Result<[u8; size_of::<u32>()], _> =
                output_vec[ERROR_OFFSET..].try_into();
            let Ok(error_data) = error_data else {
                return Err((u32::MAX, output_vec));
            };
            let error_code = u32::from_be_bytes(error_data);
            Err((error_code, output_vec))
        }
        ExecutionResult::Success { output, .. } => Ok(output.data().to_vec()),
        _ => {
            panic!("expected revert, got: {:?}", &result)
        }
    }
}

#[cfg(feature = "svm")]
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

#[cfg(feature = "svm")]
pub fn load_program_account_from_elf_file(loader_id: &Pubkey, path: &str) -> AccountSharedData {
    let mut file = File::open(path).expect("file open failed");
    let mut elf = Vec::new();
    file.read_to_end(&mut elf).unwrap();
    let mut program_account = AccountSharedData::new(0, 0, loader_id);
    program_account.set_data(elf);
    program_account.set_executable(true);
    program_account
}

// pub fn with_svm_account_mut(
//     ctx: &mut EvmTestingContext,
//     pk: &Pubkey,
//     f: impl FnOnce(&mut fluentbase_svm::account::Account),
// ) {
//     ctx.commit_db_to_sdk();
//     let account_data =
//         storage_read_account_data(&ctx.sdk, &pk, Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME)).unwrap();
//     let mut account: fluentbase_svm::account::Account = account_data.into();
//     f(&mut account);
//     let account_data: AccountSharedData = account.into();
//     storage_write_account_data(
//         &mut ctx.sdk,
//         &pk,
//         &account_data,
//         Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
//     )
//     .unwrap();
//     ctx.commit_sdk_to_db();
// }
// pub fn with_svm_account_info_mut(
//     ctx: &mut EvmTestingContext,
//     pk: &Pubkey,
//     f: impl FnOnce(&mut AccountInfo),
// ) {
//     with_svm_account_mut(ctx, pk, |mut account| {
//         let account_meta = AccountMeta::default();
//         let mut account_info = account_info_from_meta_and_account(&account_meta, &mut account);
//         f(&mut account_info);
//     });
// }
// pub fn with_svm_account_state_mut(
//     ctx: &mut EvmTestingContext,
//     pk: &Pubkey,
//     f: impl FnOnce(&mut Account),
// ) {
//     with_svm_account_info_mut(ctx, pk, |account_info| {
//         let mut account1_state = Account::unpack_unchecked(&account_info.data.borrow()).unwrap();
//         f(&mut account1_state);
//         Account::pack(account1_state, &mut account_info.data.borrow_mut()).unwrap();
//     });
// }
