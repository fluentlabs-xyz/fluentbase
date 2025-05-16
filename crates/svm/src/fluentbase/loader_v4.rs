use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    common::{calculate_max_chunk_size, lamports_from_evm_balance, pubkey_from_address},
    fluentbase::{
        common::{process_svm_result, BatchMessage, MemStorage},
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
use fluentbase_sdk::{
    debug_log,
    BlockContextReader,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
};
use solana_bincode::{deserialize, serialize};

pub fn deploy<SDK: SharedAPI>(mut sdk: SDK) {
    debug_log!("loader_v4: deploy started");
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
    let pk_payer = pubkey_from_address(contract_caller.clone()); // must exist // caller
    let pk_exec = pubkey_from_address(contract_address.clone()); // may not exist // contract_address
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
    // match &result {
    //     Err(e) => {
    //         debug_log!("deploy: result error: {:?}", e)
    //     }
    //     _ => {}
    // }
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
        storage_read_account_data(&mem_storage, &pk_payer).expect("no payer account"); // caller
    let payer_balance_after = payer_account_data.lamports();
    assert_eq!(
        payer_balance_before, payer_balance_after,
        "payer account balance shouldn't change"
    );
    let exec_account_data =
        storage_read_account_data(&mem_storage, &pk_exec).expect("no exec account");
    assert_eq!(exec_account_data.lamports(), 0, "exec account balance != 0");

    // debug_log!("after deploy: payer_account_data {:x?}", payer_account_data);
    // debug_log!("after deploy: exec_account_data {:x?}", exec_account_data);

    let preimage = serialize(&exec_account_data).expect("failed to serialize exec account data");
    let preimage: Bytes = preimage.into();
    let _ = write_protected_preimage(&mut sdk, preimage);
}

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let input = sdk.input();
    let preimage = read_protected_preimage(&sdk);
    let contract_address = sdk.context().contract_address();

    let mut mem_storage = MemStorage::new();
    let loader_id = loader_v4::id();

    let pk_exec = pubkey_from_address(contract_address);
    let mut exec_account_data: AccountSharedData =
        deserialize(preimage.as_ref()).expect("preimage doesnt contain account data");
    exec_account_data.set_lamports(lamports_from_evm_balance(
        sdk.balance(&contract_address).data,
    ));
    let exec_account_balance_before = exec_account_data.lamports();
    // debug_log!("before main: exec_account_data {:x?}", exec_account_data);

    storage_write_account_data(&mut mem_storage, &pk_exec, &exec_account_data)
        .expect("failed to write exec account");

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

    let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut Some(&mut mem_storage));
    // match &result {
    //     Err(e) => {
    //         debug_log!("main: result error: {:?}", e)
    //     }
    //     _ => {}
    // }
    let (result_accounts, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error '{}' result_accounts.len '{}'",
            exit_code,
            result_accounts.len()
        );
    }
    let mut exec_account_data =
        storage_read_account_data(&mem_storage, &pk_exec).expect("no exec account");
    let exec_account_balance_after = exec_account_data.lamports();
    assert_eq!(
        exec_account_balance_before, exec_account_balance_after,
        "exec account balance shouldn't change"
    );
    // debug_log!("after main: exec_account_data {:x?}", exec_account_data);
    // debug_log!(
    //     "after main: result_accounts.len {:x?}",
    //     result_accounts.len()
    // );
    // for (num, acc) in result_accounts.iter().enumerate() {
    //     debug_log!(
    //         "after main: result_account {}: pk {:x?} account data {:?}",
    //         num,
    //         &acc.0.to_bytes(),
    //         &acc.1
    //     );
    // }

    let out = Bytes::new();
    sdk.write(out.as_ref());
}
