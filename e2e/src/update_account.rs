use fluentbase_genesis::devnet_genesis_from_file;
use fluentbase_sdk::{address, Address, Bytes};
use fluentbase_sdk_testing::EvmTestingContext;
use revm::bytecode::{Bytecode, RWASM_MAGIC_BYTES};

#[test]
fn test_update_account_code_by_auth() {
    let mut ctx = EvmTestingContext::default();
    let genesis = devnet_genesis_from_file();

    let (update_target, account) = genesis.alloc.iter().next().unwrap();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;

    let code = ctx.get_code(*update_target);

    assert_eq!(&code.unwrap().bytes(), account.code.as_ref().unwrap());
    let prefix = b"UPDATE_DEVNET";
    let new_bytecode = Bytecode::Rwasm(Bytes::from_iter(
        RWASM_MAGIC_BYTES.iter().chain(&[0xab; 100]),
    ));

    const UPDATE_GENESIS_AUTH: Address = address!("0xa7bf6a9168fe8a111307b7c94b8883fe02b30934");
    let result = ctx.call_evm_tx(
        UPDATE_GENESIS_AUTH,
        *update_target,
        Bytes::from_iter(prefix.into_iter().chain(new_bytecode.bytecode().iter())),
        None,
        None,
    );

    assert!(result.is_success());

    let new_code = ctx.get_code(*update_target);

    assert_eq!(new_code.unwrap(), &new_bytecode);
}
