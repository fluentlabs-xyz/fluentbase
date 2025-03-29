#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    debug_log,
    func_entrypoint,
    BlockContextReader,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
    TxContextReader,
};
use solana_ee_core::{
    account::ReadableAccount,
    common::calculate_max_chunk_size,
    fluentbase_helpers::{
        exec_encoded_svm_batch_message,
        exec_svm_batch_message,
        process_svm_result,
        BatchMessage,
    },
    helpers::sdk_storage_read_account_data,
    solana_program::{
        bpf_loader_upgradeable,
        bpf_loader_upgradeable::create_buffer,
        message::Message,
        pubkey::Pubkey,
        system_program,
    },
};

func_entrypoint!(main, deploy);

pub fn deploy(mut sdk: impl SharedAPI) {
    // input must be solana elf bytes
    let elf_program_bytes: Bytes = sdk.input().into();
    let program_len = elf_program_bytes.len();
    // TODO form deploy messages and exec them
    // TODO form this Pubkeys:
    //  pk_payer - caller
    //  pk_exec - (becomes executable account key)
    //  pk_buffer - generate some random
    //  pk_authority - (who can modify the program)
    // debug_log!("elf_program_bytes.len {}", program_len);
    // panic!("elf_program_bytes.len {}", program_len);

    // TODO generate inter-dependant pubkey
    let pk_payer = Pubkey::new_unique(); // must exist // caller
    let pk_exec = Pubkey::new_unique(); // may not exist // contract_address
    let pk_buffer = Pubkey::new_unique(); // must not exist
    let pk_authority = Pubkey::new_unique(); // must exist // caller

    debug_log!("deploy.contract_caller {}", sdk.context().contract_caller());
    debug_log!("deploy.block_coinbase {}", sdk.context().block_coinbase());
    debug_log!(
        "deploy.contract_address {}",
        sdk.context().contract_address()
    );
    debug_log!("deploy.tx_origin {}", sdk.context().tx_origin());
    debug_log!(
        "deploy.contract_bytecode_address {}",
        sdk.context().contract_bytecode_address()
    );

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
        10,
        program_len,
    )
    .unwrap();
    let message = Message::new(&instructions, Some(&pk_payer));
    batch_message.append_one(message);

    let result = exec_svm_batch_message(&mut sdk, batch_message);
    let (output, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error exit_code '{}' output '{:?}'",
            exit_code,
            output.as_ref()
        );
    }

    let out = output.as_ref();
    sdk.write(out);
}

pub fn main(mut sdk: impl SharedAPI) {
    let input = sdk.input();

    let pk_system_program = system_program::id();
    let system_program_account = sdk_storage_read_account_data(&sdk, &pk_system_program);
    match system_program_account {
        Ok(v) => {
            if v.lamports() <= 0 {
                panic!("not enough lamports");
            }
        }
        Err(_) => {
            panic!("cannot get system program account");
        }
    }

    let result = exec_encoded_svm_batch_message(&mut sdk, input);
    debug_log!(
        "input.len {} input '{:?}' result: {:?}",
        input.len(),
        input,
        &result
    );
    let (output, exit_code) = process_svm_result(result);
    if exit_code != ExitCode::Ok.into_i32() {
        panic!(
            "svm_exec error '{}' output '{:?}'",
            exit_code,
            output.as_ref()
        );
    }

    let out = output.as_ref();
    sdk.write(out);
}

#[cfg(test)]
mod tests {
    use crate::main;
    use fluentbase_sdk::testing::TestingContext;
    use solana_ee_core::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        bincode,
        common::calculate_max_chunk_size,
        fluentbase_helpers::BatchMessage,
        helpers::sdk_storage_write_account_data,
        native_loader,
        native_loader::create_loadable_account_for_test,
        solana_program::{
            bpf_loader_upgradeable,
            bpf_loader_upgradeable::create_buffer,
            message::Message,
            pubkey::Pubkey,
            rent::Rent,
            system_program,
            sysvar,
        },
    };
    use std::{fs::File, io::Read};

    pub fn load_program_account_from_elf_file(loader_id: &Pubkey, path: &str) -> AccountSharedData {
        let mut file = File::open(path).expect("file open failed");
        let mut elf = Vec::new();
        file.read_to_end(&mut elf).unwrap();
        let rent = Rent::default();
        let minimum_balance = rent.minimum_balance(elf.len());
        let mut program_account = AccountSharedData::new(minimum_balance, 0, loader_id);
        program_account.set_data(elf);
        program_account.set_executable(true);
        program_account
    }

    #[test]
    fn test_svm_deploy_exec() {
        let mut sdk = TestingContext::default();

        // setup

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let bpf_loader_upgradeable_id = bpf_loader_upgradeable::id();
        let sysvar_clock_id = sysvar::clock::id();
        let sysvar_rent_id = sysvar::rent::id();

        let pk_payer = Pubkey::new_unique();
        let account_payer = AccountSharedData::new(100, 0, &system_program_id);

        let pk_buffer = Pubkey::new_unique();

        let pk_exec = Pubkey::from([8; 32]);

        let pk_authority = Pubkey::from([9; 32]);
        let pk_authority_account = AccountSharedData::new(100, 0, &system_program_id);

        let (pk_program_data, _) =
            Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable_id);
        let pk_program_data_account = AccountSharedData::new(0, 0, &system_program_id);

        let account_with_program = load_program_account_from_elf_file(
            &bpf_loader_upgradeable_id,
            "../../../solana-ee/crates/examples/hello-world/assets/solana_ee_hello_world.so",
        );

        let program_len = account_with_program.data().len();

        sdk_storage_write_account_data(&mut sdk, &pk_payer, &account_payer).unwrap();
        sdk_storage_write_account_data(&mut sdk, &pk_program_data, &pk_program_data_account)
            .unwrap();
        sdk_storage_write_account_data(
            &mut sdk,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        sdk_storage_write_account_data(
            &mut sdk,
            &bpf_loader_upgradeable_id,
            &create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader_id),
        )
        .unwrap();
        sdk_storage_write_account_data(
            &mut sdk,
            &sysvar_clock_id,
            &create_loadable_account_for_test("sysvar_clock_id", &system_program_id),
        )
        .unwrap();
        sdk_storage_write_account_data(
            &mut sdk,
            &sysvar_rent_id,
            &create_loadable_account_for_test("sysvar_rent_id", &system_program_id),
        )
        .unwrap();

        // init buffer, fill buffer, deploy

        let mut batch_message = BatchMessage::new(None);

        let instructions =
            create_buffer(&pk_payer, &pk_buffer, &pk_authority, 0, program_len).unwrap();
        let message = Message::new(&instructions, Some(&pk_payer));
        batch_message.append_one(message);

        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction =
                bpf_loader_upgradeable::write(&pk_buffer, &pk_authority, offset, bytes);
            let instructions = vec![instruction];
            Message::new(&instructions, Some(&pk_payer))
        };
        let mut write_messages = vec![];
        let chunk_size = calculate_max_chunk_size(&create_msg);
        for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
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
            10,
            account_with_program.data().len(),
        )
        .unwrap();
        let message = Message::new(&instructions, Some(&pk_payer));
        batch_message.append_one(message);

        let input = bincode::serialize(&batch_message).unwrap();
        println!("input.len {}", input.len());

        sdk = sdk.with_input(input);

        main(sdk);
    }
}
