use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    bincode,
    common::{calculate_max_chunk_size, lamports_from_evm_balance, pubkey_from_address},
    fluentbase::{
        common::{process_svm_result, BatchMessage, MemStorage},
        helpers::{exec_encoded_svm_batch_message, exec_svm_batch_message},
        loader_common::{read_protected_preimage, write_protected_preimage},
    },
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    native_loader::create_loadable_account_for_test,
    solana_program::{
        bpf_loader_upgradeable,
        bpf_loader_upgradeable::{create_buffer, UpgradeableLoaderState},
        message::Message,
        pubkey::Pubkey,
        system_program,
    },
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    debug_log,
    BlockContextReader,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
    TxContextReader,
};

pub fn deploy(mut sdk: impl SharedAPI) {
    debug_log!("");
    debug_log!("block_number: {:?}", sdk.context().block_number());
    let mut mem_storage = MemStorage::new();
    // input must be solana elf bytes
    let elf_program_bytes: Bytes = sdk.input().into();
    let program_len = elf_program_bytes.len();

    let contract_caller = sdk.context().contract_caller();
    let contract_address = sdk.context().contract_address();

    // TODO generate inter-dependant pubkey
    let pk_payer = pubkey_from_address(contract_caller.clone()); // must exist // caller
    let pk_exec = pubkey_from_address(contract_address.clone()); // may not exist // contract_address
    let pk_buffer = Pubkey::new_unique(); // must not exist
    let pk_authority = pubkey_from_address(contract_caller.clone()); // must exist // caller

    let contract_caller_balance = sdk.balance(&contract_caller);
    let mut payer_account_data = AccountSharedData::new(
        lamports_from_evm_balance(contract_caller_balance.data),
        0,
        &system_program::id(),
    );
    debug_log!("contract_caller_balance {:?}", contract_caller_balance);
    debug_log!(
        "payer_account_data.lamports {:?}",
        payer_account_data.lamports()
    );

    debug_log!("contract_caller {}", sdk.context().contract_caller());
    debug_log!("block_coinbase {}", sdk.context().block_coinbase());
    debug_log!("contract_address {}", sdk.context().contract_address());
    debug_log!("tx_origin {}", sdk.context().tx_origin());
    debug_log!(
        "contract_bytecode_address {}",
        sdk.context().contract_bytecode_address()
    );

    // setup storage
    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_for_test("system_program_id", &native_loader::id()),
    )
    .unwrap();
    storage_write_account_data(
        &mut mem_storage,
        &bpf_loader_upgradeable::id(),
        &create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader::id()),
    )
    .unwrap();
    storage_write_account_data(&mut mem_storage, &pk_payer, &payer_account_data).unwrap();
    debug_log!("");

    let mut batch_message = BatchMessage::new(None);

    let instructions = create_buffer(&pk_payer, &pk_buffer, &pk_authority, 0, program_len).unwrap();
    let message = Message::new(&instructions, Some(&pk_payer));
    batch_message.append_one(message);

    let create_msg = |offset: u32, bytes: Vec<u8>| {
        let instruction = bpf_loader_upgradeable::write(&pk_buffer, &pk_authority, offset, bytes);
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

    let instructions = bpf_loader_upgradeable::deploy_with_max_program_len(
        &pk_payer,
        &pk_exec,
        &pk_buffer,
        &pk_authority,
        // TODO compute
        0,
        program_len,
    )
    .unwrap();
    let message = Message::new(&instructions, Some(&pk_payer));
    batch_message.append_one(message);

    let result = exec_svm_batch_message(&mut sdk, batch_message, true, &mut Some(&mut mem_storage));
    match &result {
        Err(e) => {
            debug_log!("deploy: result error: {:?}", e)
        }
        _ => {}
    }
    let (result_accounts, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error exit_code '{}' result_accounts.len '{}'",
            exit_code,
            result_accounts.len()
        );
    }

    let (pk_programdata, _) =
        Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable::id());

    // TODO save updated accounts (from result_accounts): payer, exec, program_data
    let payer_account_data =
        storage_read_account_data(&mem_storage, &pk_payer).expect("no payer account"); // caller
    debug_log!(
        "deploy: payer_account_data.lamports {}",
        payer_account_data.lamports()
    );
    let exec_account_data =
        storage_read_account_data(&mem_storage, &pk_exec).expect("no exec account"); // contract itself
    let programdata_account_data =
        storage_read_account_data(&mem_storage, &pk_programdata).expect("no programdata account");
    debug_log!(
        "deploy: pk_exec {:x?} exec_account_data {} {:x?}",
        &pk_exec.as_ref(),
        exec_account_data.data().len(),
        &exec_account_data
    );
    debug_log!(
        "deploy: pk_programdata {:x?} programdata_account_data {} {:x?}",
        &pk_programdata.as_ref(),
        programdata_account_data.data().len(),
        &programdata_account_data
    );
    let preimage: Bytes = serialize(&programdata_account_data).unwrap().into();
    debug_log!("deploy: preimage.len: {}", preimage.len());
    let _ = write_protected_preimage(&mut sdk, preimage);
}

pub fn main(mut sdk: impl SharedAPI) {
    debug_log!("loader_upgradable: main started");
    debug_log!("main: block_number: {:?}", sdk.context().block_number());
    let input = sdk.input();
    let preimage = read_protected_preimage(&sdk);

    let mut mem_storage = MemStorage::new();

    let pk_exec = pubkey_from_address(sdk.context().contract_address());
    let (pk_programdata, _) =
        Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable::id());
    let programdata_account_data: AccountSharedData =
        deserialize(preimage.as_ref()).expect("preimage must contain account shared data");
    let state = UpgradeableLoaderState::Program {
        programdata_address: pk_programdata,
    };
    let exec_account_data = AccountSharedData::create(
        lamports_from_evm_balance(sdk.self_balance().data),
        serialize(&state).unwrap(),
        bpf_loader_upgradeable::id(),
        true,
        Default::default(),
    );
    debug_log!(
        "main: exec_account_data {} {:x?}",
        exec_account_data.data().len(),
        &exec_account_data
    );
    debug_log!(
        "main: programdata_account_data {} {:x?}",
        programdata_account_data.data().len(),
        &programdata_account_data
    );

    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_for_test("system_program_id", &native_loader::id()),
    )
    .unwrap();
    storage_write_account_data(
        &mut mem_storage,
        &bpf_loader_upgradeable::id(),
        &create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader::id()),
    )
    .unwrap();

    storage_write_account_data(&mut mem_storage, &pk_exec, &exec_account_data)
        .expect("failed to write exec account");
    storage_write_account_data(&mut mem_storage, &pk_programdata, &programdata_account_data)
        .expect("failed to write programdata account");

    debug_log!("main: pk_exec {:x?}", &pk_exec.as_ref());
    debug_log!("main: pk_programdata {:x?}", &pk_programdata.as_ref());

    debug_log!("main: contract_caller {}", sdk.context().contract_caller());
    debug_log!("main: block_coinbase {}", sdk.context().block_coinbase());
    debug_log!(
        "main: contract_address {}",
        sdk.context().contract_address()
    );
    debug_log!("main1: tx_origin {}", sdk.context().tx_origin());
    debug_log!(
        "main: contract_bytecode_address {}",
        sdk.context().contract_bytecode_address()
    );

    let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut Some(&mut mem_storage));
    debug_log!(
        "input.len {} input '{:?}' result: {:?}",
        preimage.len(),
        preimage,
        &result
    );
    match &result {
        Err(e) => {
            debug_log!("main: result error: {:?}", e)
        }
        _ => {}
    }
    let (result_accounts, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error '{}' result_accounts.len '{}'",
            exit_code,
            result_accounts.len()
        );
    }

    let out = Bytes::new();
    sdk.write(out.as_ref());
}
