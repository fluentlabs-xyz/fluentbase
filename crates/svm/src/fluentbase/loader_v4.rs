use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    common::{
        evm_address_from_pubkey, evm_balance_from_lamports, lamports_from_evm_balance,
        pubkey_from_evm_address,
    },
    fluentbase::{
        common::{extract_account_data_or_default, flush_not_system_accounts, process_svm_result},
        helpers::exec_encoded_svm_batch_message,
        loader_common::{read_contract_data, write_contract_data},
        mem_storage::MemStorage,
    },
    helpers::storage_write_account_data,
    loaders::bpf_loader_v4::get_state_mut,
    native_loader,
    native_loader::create_loadable_account_with_fields2,
    solana_program::{
        loader_v4,
        loader_v4::{LoaderV4State, LoaderV4Status},
    },
    system_program,
    types::BalanceHistorySnapshot,
};
use alloc::vec::Vec;
pub use deploy_entry_simplified as deploy_entry;
use fluentbase_sdk::{debug_log_ext, Bytes, ContextReader, SharedAPI, U256};
use fluentbase_types::default;
use hashbrown::HashMap;
use solana_clock::Epoch;
use solana_pubkey::Pubkey;
use solana_rbpf::{
    aligned_memory::{is_memory_aligned, AlignedMemory},
    ebpf::HOST_ALIGN,
    elf_parser::Elf64,
};

pub fn deploy_entry_simplified<SDK: SharedAPI>(mut sdk: SDK) {
    let elf_program_slice = sdk.input();
    let elf_program_bytes: Bytes = elf_program_slice.into();
    let ctx = sdk.context();
    let contract_address = ctx.contract_address();
    // let contract_caller = ctx.contract_caller();
    // let pk_caller = pubkey_from_evm_address(&contract_caller);

    drop(ctx);

    #[cfg(not(feature = "do-not-validate-elf-on-deploy"))]
    {
        let aligned;
        let bytes = if is_memory_aligned(elf_program_slice.as_ptr() as usize, HOST_ALIGN) {
            elf_program_slice
        } else {
            aligned = AlignedMemory::<{ HOST_ALIGN }>::from_slice(elf_program_slice);
            aligned.as_slice()
        };
        Elf64::parse(bytes).expect("invalid elf executable");
    }

    let pk_contract = pubkey_from_evm_address(&contract_address); // may not exist

    // let mut contract_data = pk_caller.to_bytes().to_vec();
    // contract_data.extend_from_slice(elf_program_bytes.as_ref());
    write_contract_data(&mut sdk, &pk_contract, elf_program_bytes)
        .expect("failed to save contract data");
}

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let input = sdk.input();

    let loader_id = loader_v4::id();

    let contract_caller = sdk.context().contract_caller();
    let contract_address = sdk.context().contract_address();

    let mut mem_storage = MemStorage::new();
    let loader_v4 = loader_v4::id();

    let pk_caller = pubkey_from_evm_address(&contract_caller);
    let pk_contract = pubkey_from_evm_address(&contract_address);

    debug_log_ext!(
        "pk_caller {:x?} lamports_balance (before) {}",
        &pk_caller.to_bytes(),
        sdk.metadata_storage_read(&U256::from_le_bytes(pk_caller.to_bytes()))
            .expect("failed to get lamports balance")
            .data
    );
    sdk.metadata_storage_write(&U256::from_le_bytes(pk_caller.to_bytes()), U256::from(12))
        .expect("failed to set lamports balance");
    debug_log_ext!(
        "pk_caller {:x?} lamports_balance (after) {}",
        &pk_caller.to_bytes(),
        sdk.metadata_storage_read(&U256::from_le_bytes(pk_caller.to_bytes()))
            .expect("failed to get lamports balance")
            .data
    );

    let caller_account_balance = lamports_from_evm_balance(
        sdk.balance(&contract_caller)
            .expect("balance for caller must exist")
            .data,
    );
    let mut caller_account_data =
        extract_account_data_or_default(&sdk, &pk_caller).expect("caller must exist");
    caller_account_data.set_lamports(caller_account_balance);

    let contract_data =
        read_contract_data(&sdk, &pk_contract).expect("failed to read contract executable");
    let elf_program_bytes = &contract_data.data;
    let contract_balance = lamports_from_evm_balance(
        sdk.balance(&contract_address)
            .expect("contract balance must exist")
            .data,
    );
    let mut contract_account_data = AccountSharedData::new(
        contract_balance,
        LoaderV4State::program_data_offset().saturating_add(elf_program_bytes.len()),
        &loader_id,
    );
    contract_account_data.set_rent_epoch(Epoch::MAX);
    let state = get_state_mut(contract_account_data.data_as_mut_slice())
        .expect("contract account has not enough data len");
    // state.slot = block_number;
    // state.authority_address_or_next_version = pk_authority;
    state.status = LoaderV4Status::Deployed;
    contract_account_data.data_as_mut_slice()[LoaderV4State::program_data_offset()..]
        .copy_from_slice(elf_program_bytes);

    let exec_account_balance_before = contract_balance;

    storage_write_account_data(&mut mem_storage, &pk_contract, &contract_account_data)
        .expect("failed to write contract account");

    storage_write_account_data(&mut mem_storage, &pk_caller, &caller_account_data)
        .expect("failed to write caller account");

    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_with_fields2("system_program_id", &native_loader::id()),
    )
    .expect("failed to write system_program");
    storage_write_account_data(
        &mut mem_storage,
        &loader_v4,
        &create_loadable_account_with_fields2("loader_v4_id", &native_loader::id()),
    )
    .expect("failed to write loader_v4");

    let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut Some(&mut mem_storage));
    let (result_accounts, balance_changes): (
        HashMap<Pubkey, AccountSharedData>,
        HashMap<Pubkey, BalanceHistorySnapshot<u64>>,
    ) = match process_svm_result(result) {
        Ok((result_accounts, balance_changes)) => {
            if result_accounts.len() > 0 {
                let mut api: Option<&mut SDK> = None;
                flush_not_system_accounts(&mut sdk, &mut api, &result_accounts)
                    .expect("failed to save result accounts");
            }
            (result_accounts, balance_changes)
        }
        Err(err_str) => {
            debug_log_ext!("exec err: {}", err_str);
            panic!("failed to execute encoded svm batch message: {}", err_str);
        }
    };
    let exec_account_data = result_accounts.get(&pk_contract).expect("no exec account");
    let exec_account_balance_after = exec_account_data.lamports();
    assert_eq!(
        exec_account_balance_before, exec_account_balance_after,
        "exec account balance shouldn't change"
    );
    // TODO figure out balance changes and apply them to evm
    // TODO need optimal balance sync logic
    // TODO to make this work - need implementations for accounts based on OwnableAccount

    // reorder balance changes so we have balance transfers like 'from->to'
    let mut balance_decreases: Vec<(&Pubkey, u64)> = default!();
    let mut balance_increases: Vec<(&Pubkey, u64)> = default!();
    for (pk, snapshot) in &balance_changes {
        let descriptor = snapshot.get_descriptor();
        if descriptor.amount <= 0 {
            continue;
        }
        debug_log_ext!("descriptor.amount={}", descriptor.amount);
        if descriptor.direction.is_decrease() {
            balance_decreases.push((pk, descriptor.amount));
        } else if descriptor.direction.is_increase() {
            balance_increases.push((pk, descriptor.amount));
        }
    }

    if !balance_decreases.is_empty() || !balance_increases.is_empty() {
        let mut from_iter = balance_decreases.iter_mut();
        let mut to_iter = balance_increases.iter_mut();
        let mut from = from_iter.next();
        let mut to = to_iter.next();
        while from.is_some() && to.is_some() {
            let from_value = from.as_deref_mut().unwrap();
            let to_value = to.as_deref_mut().unwrap();

            let amount = core::cmp::min(from_value.1, to_value.1);
            let address_from = evm_address_from_pubkey::<true>(from_value.0).unwrap();
            let address_to = evm_address_from_pubkey::<true>(to_value.0).unwrap();

            if contract_caller == address_from {
                sdk.call(address_to, evm_balance_from_lamports(amount), &[], None)
                    .expect("failed to transfer balance");
                debug_log_ext!("from {} to {} sent {}", contract_caller, address_to, amount);
                from_value.1 -= amount;
                to_value.1 -= amount;
                if from_value.1 <= 0 {
                    from = from_iter.next();
                }
                if to_value.1 <= 0 {
                    to = to_iter.next();
                }
            } else {
                panic!("balance transfers from non-caller are not supported");
            }
        }
        assert!(
            from.is_none() && to.is_none(),
            "some balances left unsettled: from {} to {}",
            from.is_some(),
            to.is_some()
        );
    }

    let out = Bytes::new();
    sdk.write(out.as_ref());
}

/*pub fn deploy_entry_original<SDK: SharedAPI>(mut sdk: SDK) {
    use crate::{
        common::calculate_max_chunk_size,
        fluentbase::{common::BatchMessage, helpers::exec_svm_batch_message},
        solana_program::message::Message,
    };
    use alloc::{vec, vec::Vec};

    let mut mem_storage = MemStorage::new();

    let elf_program_bytes: Bytes = sdk.input().into();
    let program_len = elf_program_bytes.len();
    let loader_id = loader_v4::id();

    let ctx = sdk.context();
    let contract_caller = ctx.contract_caller();
    let contract_address = ctx.contract_address();

    drop(ctx);

    let pk_payer = pubkey_from_evm_address(&contract_caller); // must exist // caller
    let pk_contract = pubkey_from_evm_address(&contract_address); // may not exist // contract_address
    let pk_authority = pk_payer.clone(); // must exist // caller

    let contract_caller_balance = sdk.balance(&contract_caller);
    let payer_balance_before = lamports_from_evm_balance(contract_caller_balance.data);
    let payer_account_data = AccountSharedData::new(payer_balance_before, 0, &system_program::id());

    storage_write_account_data(
        &mut mem_storage,
        &system_program::id(),
        &create_loadable_account_for_test("system_program_id", &native_loader::id()),
    )
    .expect("failed to write system_program into mem storage");
    storage_write_account_data(
        &mut mem_storage,
        &loader_id,
        &create_loadable_account_for_test("loader_v4_id", &native_loader::id()),
    )
    .expect("failed to write loader_v4 into mem storage");
    storage_write_account_data(&mut mem_storage, &pk_payer, &payer_account_data).unwrap();

    let mut batch_message = BatchMessage::new(None);

    // TODO need specific value?
    let balance_to_transfer = 0;
    let instructions = loader_v4::create_buffer(
        &pk_payer,
        &pk_contract,
        balance_to_transfer,
        &pk_authority,
        program_len as u32,
        &pk_payer,
    );
    let message = Message::new(&instructions, Some(&pk_payer));
    batch_message.append_one(message);

    let create_msg = |offset: u32, bytes: Vec<u8>| {
        let instruction = loader_v4::write(&pk_contract, &pk_authority, offset, bytes);
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

    let instruction = loader_v4::deploy(&pk_contract, &pk_authority);
    let message = Message::new(&[instruction], Some(&pk_payer));
    batch_message.append_one(message);

    let result = exec_svm_batch_message(&mut sdk, batch_message, true, &mut Some(&mut mem_storage));
    let (result_accounts, _balance_changes) = match process_svm_result(result) {
        Ok(v) => v,
        Err(err_str) => {
            panic!("failed to execute svm batch message: {}", err_str);
        }
    };

    let payer_account_data = result_accounts
        .get(&pk_payer)
        .expect("payer account doesn't exist"); // caller
    let payer_balance_after = payer_account_data.lamports();
    assert_eq!(
        payer_balance_before, payer_balance_after,
        "payer_balance_before != payer_balance_after"
    );
    let contract_account_data = result_accounts
        .get(&pk_contract)
        .expect("contract account must exist");
    assert_eq!(
        contract_account_data.lamports(),
        0,
        "contract account balance != 0"
    );

    let contract_account_data =
        serialize(&contract_account_data).expect("failed to serialize contract account data");
    let contract_account_data: Bytes = contract_account_data.into();
    write_contract_data(&mut sdk, &pk_contract, contract_account_data)
        .expect("failed to save contract");
    // TODO figure out balance changes and apply them to evm
}*/
