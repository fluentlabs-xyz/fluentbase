use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    common::{calculate_max_chunk_size, lamports_from_evm_balance, pubkey_from_address},
    error::SvmError,
    fluentbase::{
        common::{extract_account_data_or_default, process_svm_result, BatchMessage, MemStorage},
        helpers_v2::{exec_encoded_svm_batch_message, exec_svm_batch_message},
        loader_common::{read_protected_preimage, write_protected_preimage},
    },
    helpers::{storage_read_account_data, storage_write_account_data, SyscallError},
    native_loader,
    native_loader::{create_loadable_account_for_test, create_loadable_account_with_fields},
    solana_program::{loader_v4, message::Message},
    system_program,
};
use alloc::{format, vec, vec::Vec};
use bincode::error::DecodeError;
use core::any::type_name;
use fluentbase_sdk::{
    debug_log,
    BlockContextReader,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
    U256,
};
use fluentbase_types::SyscallResult;
use solana_bincode::{deserialize, serialize};
use solana_clock::INITIAL_RENT_EPOCH;
use solana_pubkey::Pubkey;
use solana_rbpf::memory_region::{AccessType, MemoryMapping};

fn translate_slice_inner<'a, T>(vm_addr: u64, len: u64) -> Result<&'a mut [T], SvmError> {
    if len == 0 {
        return Ok(&mut []);
    }
    let type_name = type_name::<T>();
    let size_of_t = size_of::<T>();
    debug_log!(
        "translate_slice_inner 1: len {} item type '{}' size_of_t {}",
        len,
        type_name,
        size_of_t,
    );

    let total_size = len.saturating_mul(size_of_t as u64);
    if isize::try_from(total_size).is_err() {
        return Err(SyscallError::InvalidLength.into());
    }

    debug_log!(
        "translate_slice_inner 2: vm_addr {} total_size {}",
        vm_addr,
        total_size
    );

    Ok(unsafe { core::slice::from_raw_parts_mut(vm_addr as *mut T, len as usize) })
}

#[test]
fn slice_test() {
    let seed1 = b"my_seed";
    let seed2 = Pubkey::new_unique();
    let seeds_binding = [seed1.as_slice(), seed2.as_ref()];
    let seeds = seeds_binding.as_slice();
    let seeds_byte_len = size_of::<&[&[u8]]>();
    debug_log!("seeds_byte_len: {}", seeds_byte_len);
    let seeds_addr = &seeds as *const &[&[u8]];
    // let slice = translate_slice_inner::<&[u8]>(seeds_addr, seeds.len() as u64).unwrap();
    // debug_log!("slice: {:x?}", slice);
    let word_size = size_of::<usize>();
    let seeds_fat_ptr_header =
        unsafe { core::slice::from_raw_parts(seeds_addr as *const u8, seeds_byte_len) };
    let raw_ptr = usize::from_le_bytes(seeds_fat_ptr_header[..word_size].try_into().unwrap());
    let raw_len = usize::from_le_bytes(seeds_fat_ptr_header[word_size..].try_into().unwrap());
    debug_log!(
        "seeds_slice (raw_ptr:{} raw_len:{}): {:x?}",
        raw_ptr,
        raw_len,
        seeds_fat_ptr_header
    );
    let seeds_slice1 = unsafe { core::slice::from_raw_parts(raw_ptr as *const u8, raw_len) };
    debug_log!("seeds_slice1 {:x?}", seeds_slice1);
}

pub fn deploy_entry<SDK: SharedAPI>(mut sdk: SDK) {
    // let seed1 = b"my_seed";
    // let seed2 = Pubkey::new_unique();
    // let seeds = &[seed1.as_slice(), seed2.as_ref()];
    // let seeds_byte_len = size_of::<&[&[u8]]>();
    // debug_log!("seeds_byte_len: {}", seeds_byte_len);
    // let seeds_addr = seeds.as_ptr() as usize;
    // let slice = translate_slice_inner::<&[u8]>(seeds_addr, seeds.len() as u64).unwrap();
    // debug_log!("slice: {:x?}", slice);
    // let seeds_slice =
    //     unsafe { core::slice::from_raw_parts(seeds_addr as *const u8, seeds_byte_len) };
    // debug_log!("seeds_slice: {:x?}", seeds_slice);
    //
    // return;

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
        storage_read_account_data(&mem_storage, &pk_payer).expect("payer account must exist"); // caller
    let payer_balance_after = payer_account_data.lamports();
    assert_eq!(
        payer_balance_before, payer_balance_after,
        "payer account balance shouldn't change"
    );
    let exec_account_data =
        storage_read_account_data(&mem_storage, &pk_exec).expect("exec account must exist");
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

    let contract_caller = sdk.context().contract_caller();
    let contract_address = sdk.context().contract_address();

    let mut mem_storage = MemStorage::new();
    let loader_id = loader_v4::id();

    let pk_caller = pubkey_from_address(&contract_caller);
    debug_log!("pk_caller: {}", pk_caller);
    let pk_contract = pubkey_from_address(&contract_address);
    debug_log!("pk_contract: {}", pk_contract);

    let caller_lamports = lamports_from_evm_balance(
        sdk.balance(&contract_caller)
            .expect("balance for caller must exist")
            .data,
    );
    debug_log!("caller_lamports (from evm balance) {}", caller_lamports);
    let mut caller_account_data =
        extract_account_data_or_default(&sdk, &pk_caller).expect("caller must exist");
    debug_log!(
        "caller_lamports (from storage) {}",
        caller_account_data.lamports()
    );
    caller_account_data.set_lamports(caller_lamports);

    let mut contract_account_data: Result<AccountSharedData, DecodeError> =
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
    debug_log!("contract_lamports {}", contract_lamports);
    contract_account_data.set_lamports(contract_lamports);
    let exec_account_balance_before = contract_account_data.lamports();
    // debug_log!("before main: exec_account_data {:x?}", exec_account_data);

    storage_write_account_data(&mut mem_storage, &pk_contract, &contract_account_data)
        .expect("failed to write contract account");

    storage_write_account_data(&mut mem_storage, &pk_caller, &caller_account_data)
        .expect("failed to write caller account");

    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_for_test("system_program_id", &native_loader::id()), // TODO replace with create_loadable_account_with_fields
    )
    .unwrap();
    storage_write_account_data(
        &mut mem_storage,
        &loader_id,
        &create_loadable_account_for_test("loader_v4_id", &native_loader::id()), // TODO replace with create_loadable_account_with_fields
    )
    .unwrap();

    debug_log!(
        "main: exec_encoded_svm_batch_message: input.len: {} input {:x?}",
        input.len(),
        input
    );
    let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut Some(&mut mem_storage));
    match &result {
        Err(e) => {
            debug_log!("main: result error: {:?}", e)
        }
        _ => {}
    }
    let (result_accounts, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "main: svm_exec error '{}' result_accounts.len '{}'",
            exit_code,
            result_accounts.len()
        );
    }
    let mut exec_account_data =
        storage_read_account_data(&mem_storage, &pk_contract).expect("no exec account");
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
