use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    common::{calculate_max_chunk_size, lamports_from_evm_balance, pubkey_from_address},
    fluentbase::{
        common::{
            extract_account_data_or_default,
            flush_accounts,
            process_svm_result,
            BatchMessage,
            MemStorage,
        },
        helpers_v2::{exec_encoded_svm_batch_message, exec_svm_batch_message},
        loader_common::{read_protected_preimage, write_protected_preimage},
    },
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    native_loader::create_loadable_account_for_test,
    solana_program::{loader_v4, message::Message},
    system_program,
};
use alloc::{vec, vec::Vec};
use bincode::error::DecodeError;
use fluentbase_sdk::{Bytes, ContextReader, ExitCode, SharedAPI};
use solana_bincode::{deserialize, serialize};

pub fn deploy_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let mut mem_storage = MemStorage::new();

    let elf_program_bytes: Bytes = sdk.input().into();
    let program_len = elf_program_bytes.len();
    let loader_id = loader_v4::id();

    let ctx = sdk.context();
    let contract_caller = ctx.contract_caller();
    let contract_address = ctx.contract_address();
    ctx.contract_value();

    drop(ctx);

    // TODO generate inter-dependant pubkey
    let pk_payer = pubkey_from_address(&contract_caller); // must exist // caller
    let pk_exec = pubkey_from_address(&contract_address); // may not exist // contract_address
    let pk_authority = pk_payer.clone(); // must exist // caller

    let contract_caller_balance = sdk.balance(&contract_caller);
    let payer_balance_before = lamports_from_evm_balance(contract_caller_balance.data);
    let payer_account_data = AccountSharedData::new(payer_balance_before, 0, &system_program::id());

    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_for_test("system_program_id", &native_loader::id()),
    )
    .unwrap();
    storage_write_account_data(
        &mut mem_storage,
        &loader_id,
        &create_loadable_account_for_test("loader_v4_id", &native_loader::id()),
    )
    .unwrap();
    storage_write_account_data(&mut mem_storage, &pk_payer, &payer_account_data).unwrap();

    let mut batch_message = BatchMessage::new(None);

    // TODO do we need this?
    let balance_to_transfer = 0;
    let instructions = loader_v4::create_buffer(
        &pk_payer,
        &pk_exec,
        balance_to_transfer,
        &pk_authority,
        program_len as u32,
        &pk_payer,
    );
    let message = Message::new(&instructions, Some(&pk_payer));
    batch_message.append_one(message);

    let create_msg = |offset: u32, bytes: Vec<u8>| {
        let instruction = loader_v4::write(&pk_exec, &pk_authority, offset, bytes);
        let instructions = vec![instruction];
        Message::new(&instructions, Some(&pk_payer))
    };
    let mut write_messages = vec![];
    let chunk_size = calculate_max_chunk_size(&create_msg);
    for (chunk, i) in elf_program_bytes.chunks(chunk_size).zip(0..) {
        let offset = i * chunk_size;
        let msg = create_msg(offset as u32, chunk.to_vec());
        write_messages.push(msg);
    }
    batch_message.append_many(write_messages);

    let instruction = loader_v4::deploy(&pk_exec, &pk_authority);
    let message = Message::new(&[instruction], Some(&pk_payer));
    batch_message.append_one(message);

    let result = exec_svm_batch_message(&mut sdk, batch_message, true, &mut Some(&mut mem_storage));
    let (result_accounts, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error exit_code '{}' result_accounts.len '{}'",
            exit_code,
            result_accounts.len()
        );
    }

    // TODO save updated accounts (from result_accounts): payer, exec, program_data
    let payer_account_data =
        storage_read_account_data(&mem_storage, &pk_payer).expect("payer account must exist"); // caller
    let payer_balance_after = payer_account_data.lamports();
    assert_eq!(
        payer_balance_before, payer_balance_after,
        "payer account balance shouldn't change"
    );
    let exec_account_data =
        storage_read_account_data(&mem_storage, &pk_exec).expect("exec account must exist");
    assert_eq!(exec_account_data.lamports(), 0, "exec account balance != 0");

    let preimage = serialize(&exec_account_data).expect("failed to serialize exec account data");
    let preimage: Bytes = preimage.into();
    let _ = write_protected_preimage(&mut sdk, preimage);
}

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let input = sdk.input();
    let preimage = read_protected_preimage(&sdk);

    let contract_caller = sdk.context().contract_caller();
    let contract_address = sdk.context().contract_address();

    let mut mem_storage = MemStorage::new();
    let loader_v4 = loader_v4::id();

    let pk_caller = pubkey_from_address(&contract_caller);
    let pk_contract = pubkey_from_address(&contract_address);

    let caller_lamports = lamports_from_evm_balance(
        sdk.balance(&contract_caller)
            .expect("balance for caller must exist")
            .data,
    );
    let mut caller_account_data =
        extract_account_data_or_default(&sdk, &pk_caller).expect("caller must exist");
    caller_account_data.set_lamports(caller_lamports);

    let contract_account_data: Result<AccountSharedData, DecodeError> =
        deserialize(preimage.as_ref());
    let mut contract_account_data = match contract_account_data {
        Ok(v) => v,
        Err(e) => {
            panic!(
                "main: preimage doesnt contain account data ({}): {}",
                preimage.len(),
                e
            );
        }
    };
    let contract_lamports = lamports_from_evm_balance(
        sdk.balance(&contract_address)
            .expect("contract balance must exist")
            .data,
    );
    contract_account_data.set_lamports(contract_lamports);
    let exec_account_balance_before = contract_account_data.lamports();

    storage_write_account_data(&mut mem_storage, &pk_contract, &contract_account_data)
        .expect("failed to write contract account");

    storage_write_account_data(&mut mem_storage, &pk_caller, &caller_account_data)
        .expect("failed to write caller account");

    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_for_test("system_program_id", &native_loader::id()), // TODO replace with create_loadable_account_with_fields
    )
    .expect("failed to write system_program");
    storage_write_account_data(
        &mut mem_storage,
        &loader_v4,
        &create_loadable_account_for_test("loader_v4_id", &native_loader::id()), // TODO replace with create_loadable_account_with_fields
    )
    .expect("failed to write loader_v4");

    let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut Some(&mut mem_storage));
    match &result {
        Err(_e) => {}
        Ok(accounts) => {
            if accounts.len() > 0 {
                let mut sapi: Option<&mut SDK> = None;
                flush_accounts(&mut sdk, &mut sapi, accounts).expect("failed to flush accounts");
            }
        }
    }
    let (result_accounts, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "main: svm_exec error '{}' result_accounts.len '{}'",
            exit_code,
            result_accounts.len()
        );
    }
    let exec_account_data =
        storage_read_account_data(&mem_storage, &pk_contract).expect("no exec account");
    let exec_account_balance_after = exec_account_data.lamports();
    assert_eq!(
        exec_account_balance_before, exec_account_balance_after,
        "exec account balance shouldn't change"
    );

    let out = Bytes::new();
    sdk.write(out.as_ref());
}
