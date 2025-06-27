use fluentbase_genesis::{devnet_genesis_from_file, UPDATE_GENESIS_AUTH, UPDATE_GENESIS_PREFIX};
use fluentbase_sdk::Bytes;
use fluentbase_sdk_testing::EvmTestingContext;
use revm::bytecode::{Bytecode, RWASM_MAGIC_BYTES};

#[test]
fn test_update_account_code_by_auth() {
    let mut ctx = EvmTestingContext::default();
    let genesis = devnet_genesis_from_file();

    let (update_target, account) = genesis.alloc.iter().next().unwrap();

    let code = ctx.get_code(*update_target);

    assert_eq!(&code.unwrap().bytes(), account.code.as_ref().unwrap());

    let new_bytecode = Bytecode::Rwasm(Bytes::from_iter(
        RWASM_MAGIC_BYTES.iter().chain(&[0xab; 100]),
    ));

    let result = ctx.call_evm_tx(
        UPDATE_GENESIS_AUTH,
        *update_target,
        Bytes::from_iter(
            UPDATE_GENESIS_PREFIX
                .iter()
                .chain(new_bytecode.bytecode().iter()),
        ),
        None,
        None,
    );

    assert!(result.is_success());

    let new_code = ctx.get_code(*update_target);

    assert_eq!(new_code.unwrap(), &new_bytecode);
}
