use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolEvent};
use bytes::BytesMut;
use fluentbase_codec::SolidityABI;
use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_sdk::{
    address, bytes, compile_rwasm_maybe_system, crypto::crypto_keccak256, Address, Bytes, B256,
    DEFAULT_UPDATE_GENESIS_AUTH, PRECOMPILE_EVM_RUNTIME, PRECOMPILE_RIPEMD160,
    PRECOMPILE_RUNTIME_UPGRADE, PRECOMPILE_WEBAUTHN_VERIFIER, U256, UPDATE_GENESIS_PREFIX,
};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;
use revm::context::result::ExecutionResult;

sol! {
    event RuntimeUpgraded(
        address indexed target_address,
        bytes32 indexed genesis_hash,
        string genesis_version,
        bytes32 code_hash
    );

    function upgradeTo(
        address target_address,
        uint256 genesis_hash,
        string genesis_version,
        bytes wasm_bytecode
    );

    function upgradeEvmTo(
        address target_address,
        uint256 genesis_hash,
        string genesis_version,
        bytes evm_bytecode
    );

    function planUpgrade(
        uint256 genesis_hash,
        string genesis_version,
        address[] target_addresses,
        bytes32[] wasm_code_hashes,
        address updater
    );

    function upgradeToPlanned(address target_address, bytes wasm_bytecode);
}

#[test]
fn test_upgrade_solidity_contract_preserves_storage() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER_ADDRESS: Address = address!("0x7777777777777777777777777777777777777777");

    // Runtime stores the first ABI argument in slot zero.
    let (contract_address, _) = ctx.deploy_evm_tx_with_gas(
        DEPLOYER_ADDRESS,
        hex!("6007600c60003960076000f360043560005500").into(),
    );
    let old_code = ctx.get_code(contract_address).unwrap().original_bytes();
    let stored_value = B256::from([0x42; 32]);
    let mut store_input = vec![0u8; 4];
    store_input.extend_from_slice(stored_value.as_slice());
    assert!(ctx
        .call_evm_tx(
            DEPLOYER_ADDRESS,
            contract_address,
            store_input.into(),
            None,
            None,
        )
        .is_success());

    // New deployed runtime returns slot zero.
    let new_runtime = bytes!("60005460005260206000f3");
    let mut upgrade_args = BytesMut::new();
    SolidityABI::<(Address, B256, String, Bytes)>::encode_function_args(
        &(
            contract_address,
            B256::ZERO,
            "v1.0.1".to_string(),
            new_runtime.clone(),
        ),
        &mut upgrade_args,
    )
    .unwrap();
    let selector = crypto_keccak256(b"upgradeEvmTo(address,uint256,string,bytes)");
    let mut upgrade_input = selector[..4].to_vec();
    upgrade_input.extend_from_slice(&upgrade_args);

    let result = ctx.call_evm_tx(
        DEFAULT_UPDATE_GENESIS_AUTH,
        PRECOMPILE_RUNTIME_UPGRADE,
        upgrade_input.into(),
        None,
        None,
    );
    assert!(result.is_success(), "{result:?}");
    let upgraded_code = ctx.get_code(contract_address).unwrap();
    assert_ne!(upgraded_code.original_bytes(), old_code);
    let account = ctx.db.cache.accounts.get(&contract_address).unwrap();
    assert_eq!(
        account.storage.get(&U256::ZERO),
        Some(&U256::from_be_slice(stored_value.as_slice()))
    );

    let result = ctx.call_evm_tx(DEPLOYER_ADDRESS, contract_address, Bytes::new(), None, None);
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.output().unwrap(), stored_value.as_slice());

    let mut recompile_args = BytesMut::new();
    SolidityABI::<(Address,)>::encode_function_args(&(contract_address,), &mut recompile_args)
        .unwrap();
    let selector = crypto_keccak256(b"recompile(address)");
    let mut recompile_input = selector[..4].to_vec();
    recompile_input.extend_from_slice(&recompile_args);
    let result = ctx.call_evm_tx(
        DEFAULT_UPDATE_GENESIS_AUTH,
        PRECOMPILE_RUNTIME_UPGRADE,
        recompile_input.into(),
        None,
        None,
    );
    assert!(!result.is_success());
}

#[test]
#[should_panic(
    expected = "Encountered unexpected internal return flag: FatalExternalError with instruction result: InterpreterResult { result: FatalExternalError, output: 0x, gas: Gas { tracker: GasTracker { gas_limit: 3000000, remaining: 0, reservoir: 0, state_gas_spent: 0, refunded: 0 }, memory: MemoryGas { words_num: 0, expansion_cost: 0 } } }"
)]
fn test_update_account_code_by_auth() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // ----------------------------------------------------
    // 1. Deploy some random EVM bytecode
    // ----------------------------------------------------

    const DEPLOYER_ADDRESS: Address = address!("0x7777777777777777777777777777777777777777");
    let (contract_address, _) = ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033").into());

    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        bytes!("45773e4e"),
        None,
        None,
    );
    assert!(result.is_success());

    let account = GENESIS_CONTRACTS_BY_ADDRESS
        .get(&PRECOMPILE_EVM_RUNTIME)
        .unwrap();
    let code = ctx.get_code(PRECOMPILE_EVM_RUNTIME).unwrap();
    assert_eq!(&code.original_bytes(), account.rwasm_bytecode.as_ref());

    // ----------------------------------------------------
    // 2. Deploy Wasm bytecode using V2
    // ----------------------------------------------------

    let wasm_module: Bytes = wat::parse_str(
        r#"
(module
  (memory (export "memory") 1)
  (func (export "main") (param i32 i32) (result i32)
    unreachable
  )
  (func (export "deploy")
    unreachable
  )
)
    "#,
    )
    .unwrap()
    .into();

    let mut upgrade_input = BytesMut::new();
    SolidityABI::<(Address, B256, String, Bytes)>::encode_function_args(
        &(
            PRECOMPILE_EVM_RUNTIME,
            B256::ZERO,
            "v1.0.1".to_string(),
            wasm_module.clone(),
        ),
        &mut upgrade_input,
    )
    .unwrap();
    let upgrade_input = upgrade_input.freeze();
    let mut bytes_input = vec![];
    bytes_input.extend_from_slice(&UPDATE_GENESIS_PREFIX);
    bytes_input.extend_from_slice(&upgrade_input);

    let result = ctx.call_evm_tx(
        DEFAULT_UPDATE_GENESIS_AUTH,
        PRECOMPILE_RUNTIME_UPGRADE,
        bytes_input.into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());

    let new_code = ctx.get_code(PRECOMPILE_EVM_RUNTIME).unwrap();
    let rwasm_bytecode_should_be =
        compile_rwasm_maybe_system(&PRECOMPILE_EVM_RUNTIME, &wasm_module)
            .unwrap()
            .rwasm_module
            .serialize();
    assert_eq!(
        new_code.original_bytes().as_ref(),
        &rwasm_bytecode_should_be
    );

    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        bytes!("45773e4e"),
        None,
        None,
    );
    assert!(result.is_halt());
}

#[test]
fn test_cant_upgrade_from_incorrect_address() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // ----------------------------------------------------
    // 1. Deploy some random EVM bytecode
    // ----------------------------------------------------

    const DEPLOYER_ADDRESS: Address = address!("0x7777777777777777777777777777777777777777");
    let (contract_address, _) = ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033").into());

    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        bytes!("45773e4e"),
        None,
        None,
    );
    assert!(result.is_success());

    let account = GENESIS_CONTRACTS_BY_ADDRESS
        .get(&PRECOMPILE_EVM_RUNTIME)
        .unwrap();
    let code = ctx.get_code(PRECOMPILE_EVM_RUNTIME).unwrap();
    assert_eq!(&code.original_bytes(), account.rwasm_bytecode.as_ref());

    // ----------------------------------------------------
    // 2. Deploy Wasm bytecode using V2
    // ----------------------------------------------------

    let wasm_module: Bytes = wat::parse_str(
        r#"
(module
  (memory (export "memory") 1)
  (func (export "main") (param i32 i32) (result i32)
    unreachable
  )
  (func (export "deploy")
    unreachable
  )
)
    "#,
    )
    .unwrap()
    .into();

    let mut upgrade_input = BytesMut::new();
    SolidityABI::<(Address, B256, String, Bytes)>::encode_function_args(
        &(
            PRECOMPILE_EVM_RUNTIME,
            B256::ZERO,
            "v1.0.1".to_string(),
            wasm_module.clone(),
        ),
        &mut upgrade_input,
    )
    .unwrap();
    let upgrade_input = upgrade_input.freeze();
    let mut bytes_input = vec![];
    bytes_input.extend_from_slice(&UPDATE_GENESIS_PREFIX);
    bytes_input.extend_from_slice(&upgrade_input);

    let result = ctx.call_evm_tx(
        address!("0x1111111111111111111111111111111111111111"),
        PRECOMPILE_RUNTIME_UPGRADE,
        upgrade_input.into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(!result.is_success());
}

/// A canonical EVM precompile that Fluent implements as rWasm in state. EVM-facing code queries for
/// these addresses are masked to empty by design, which is why upgrade events must not source their
/// artifact hash from `EXTCODEHASH`.
const CANONICAL_PRECOMPILE_TARGET: Address = PRECOMPILE_RIPEMD160;
/// A Fluent system address, outside the canonical precompile range and therefore unmasked.
const FLUENT_TARGET: Address = PRECOMPILE_WEBAUTHN_VERIFIER;
const UPDATER_ADDRESS: Address = address!("0x8888888888888888888888888888888888888888");

/// A minimal WASM runtime, valid enough to compile but small enough to keep upgrade tests cheap.
fn upgrade_wasm_module() -> Bytes {
    wat::parse_str(
        r#"
(module
  (memory (export "memory") 1)
  (func (export "main") (param i32 i32) (result i32)
    unreachable
  )
  (func (export "deploy")
    unreachable
  )
)
    "#,
    )
    .unwrap()
    .into()
}

/// The rWasm bytes an upgrade of `target_address` installs, compiled here rather than read back out
/// of state, so the expected hash is derived independently of the contract under test.
fn installed_rwasm_bytecode(target_address: Address, wasm_module: &Bytes) -> Vec<u8> {
    compile_rwasm_maybe_system(&target_address, wasm_module)
        .unwrap()
        .rwasm_module
        .serialize()
}

/// Decodes the single `RuntimeUpgraded` log of a successful upgrade with a standard Solidity log
/// decoder (`alloy-sol-types`), not with the encoder the contract itself uses.
fn decode_runtime_upgraded(logs: &[revm::primitives::Log]) -> RuntimeUpgraded {
    let log = logs
        .iter()
        .find(|log| log.topics().first() == Some(&RuntimeUpgraded::SIGNATURE_HASH))
        .expect("no RuntimeUpgraded event emitted");
    RuntimeUpgraded::decode_raw_log(log.topics().iter().copied(), &log.data.data)
        .expect("failed to decode RuntimeUpgraded")
}

/// Reads `EXTCODEHASH` the way an ordinary EVM caller would — through deployed EVM bytecode — so
/// the precompile masking rules apply.
fn evm_ext_code_hash(ctx: &mut EvmTestingContext, address: Address) -> B256 {
    const DEPLOYER_ADDRESS: Address = address!("0x9999999999999999999999999999999999999999");
    // PUSH20 <address>; EXTCODEHASH; PUSH0; MSTORE; PUSH1 0x20; PUSH0; RETURN
    let mut runtime = vec![0x73u8];
    runtime.extend_from_slice(address.as_slice());
    runtime.extend_from_slice(&[0x3f, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3]);
    let runtime_len = runtime.len() as u8;
    // Copy the runtime tail (12 bytes in) out of the init code and return it.
    let mut init_bytecode = vec![
        0x60,
        runtime_len,
        0x60,
        0x0c,
        0x60,
        0x00,
        0x39,
        0x60,
        runtime_len,
        0x60,
        0x00,
        0xf3,
    ];
    init_bytecode.extend_from_slice(&runtime);

    let (probe_address, _) = ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, init_bytecode.into());
    let result = ctx.call_evm_tx(DEPLOYER_ADDRESS, probe_address, Bytes::new(), None, None);
    assert!(result.is_success(), "{result:?}");
    B256::from_slice(result.output().unwrap())
}

/// Upgrades `target_address` with `upgradeTo` and returns the emitted event plus the artifact hash
/// computed independently from the submitted WASM.
fn upgrade_wasm_runtime(
    ctx: &mut EvmTestingContext,
    target_address: Address,
) -> (RuntimeUpgraded, B256) {
    let wasm_module = upgrade_wasm_module();
    let input = upgradeToCall {
        target_address,
        genesis_hash: U256::ZERO,
        genesis_version: "v1.0.1".to_string(),
        wasm_bytecode: wasm_module.clone(),
    }
    .abi_encode();

    let result = ctx.call_evm_tx(
        DEFAULT_UPDATE_GENESIS_AUTH,
        PRECOMPILE_RUNTIME_UPGRADE,
        input.into(),
        None,
        None,
    );
    assert!(result.is_success(), "{result:?}");

    let installed = installed_rwasm_bytecode(target_address, &wasm_module);
    assert_eq!(
        ctx.get_code(target_address)
            .unwrap()
            .original_bytes()
            .as_ref(),
        &installed,
        "installed bytecode differs from the compiled artifact"
    );

    (
        decode_runtime_upgraded(result.logs()),
        crypto_keccak256(&installed),
    )
}

#[test]
fn test_wasm_upgrade_emits_installed_code_hash_for_canonical_precompile() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let (event, expected_code_hash) = upgrade_wasm_runtime(&mut ctx, CANONICAL_PRECOMPILE_TARGET);

    assert_eq!(event.target_address, CANONICAL_PRECOMPILE_TARGET);
    assert_ne!(
        event.code_hash,
        B256::ZERO,
        "canonical precompile upgrade emitted a zero artifact hash"
    );
    assert_eq!(event.code_hash, expected_code_hash);

    // The masking that broke the event must stay in place for ordinary EVM callers.
    assert_eq!(
        evm_ext_code_hash(&mut ctx, CANONICAL_PRECOMPILE_TARGET),
        B256::ZERO,
        "EXTCODEHASH stopped masking a canonical precompile"
    );
}

#[test]
fn test_wasm_upgrade_emits_installed_code_hash_for_fluent_target() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let (event, expected_code_hash) = upgrade_wasm_runtime(&mut ctx, FLUENT_TARGET);

    assert_eq!(event.target_address, FLUENT_TARGET);
    assert_ne!(event.code_hash, B256::ZERO);
    assert_eq!(event.code_hash, expected_code_hash);
    // Unmasked targets keep reporting the same hash through the public EVM query.
    assert_eq!(
        evm_ext_code_hash(&mut ctx, FLUENT_TARGET),
        expected_code_hash
    );
}

#[test]
fn test_planned_upgrade_emits_installed_code_hash_for_canonical_precompile() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let target_address = CANONICAL_PRECOMPILE_TARGET;
    let wasm_module = upgrade_wasm_module();

    let plan_input = planUpgradeCall {
        genesis_hash: U256::ZERO,
        genesis_version: "v1.0.1".to_string(),
        target_addresses: vec![target_address],
        wasm_code_hashes: vec![crypto_keccak256(wasm_module.as_ref())],
        updater: UPDATER_ADDRESS,
    }
    .abi_encode();
    let result = ctx.call_evm_tx(
        DEFAULT_UPDATE_GENESIS_AUTH,
        PRECOMPILE_RUNTIME_UPGRADE,
        plan_input.into(),
        None,
        None,
    );
    assert!(result.is_success(), "{result:?}");

    let upgrade_input = upgradeToPlannedCall {
        target_address,
        wasm_bytecode: wasm_module.clone(),
    }
    .abi_encode();
    let result = ctx.call_evm_tx(
        UPDATER_ADDRESS,
        PRECOMPILE_RUNTIME_UPGRADE,
        upgrade_input.into(),
        None,
        None,
    );
    assert!(result.is_success(), "{result:?}");

    let expected_code_hash =
        crypto_keccak256(&installed_rwasm_bytecode(target_address, &wasm_module));
    let event = decode_runtime_upgraded(result.logs());
    assert_eq!(event.target_address, target_address);
    assert_ne!(
        event.code_hash,
        B256::ZERO,
        "planned upgrade emitted a zero artifact hash"
    );
    assert_eq!(event.code_hash, expected_code_hash);
}

#[test]
fn test_evm_upgrade_emits_installed_code_hash_for_canonical_precompile() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let target_address = CANONICAL_PRECOMPILE_TARGET;
    let evm_runtime = bytes!("60005460005260206000f3");

    let input = upgradeEvmToCall {
        target_address,
        genesis_hash: U256::ZERO,
        genesis_version: "v1.0.1".to_string(),
        evm_bytecode: evm_runtime.clone(),
    }
    .abi_encode();
    let result = ctx.call_evm_tx(
        DEFAULT_UPDATE_GENESIS_AUTH,
        PRECOMPILE_RUNTIME_UPGRADE,
        input.into(),
        None,
        None,
    );
    assert!(result.is_success(), "{result:?}");

    let event = decode_runtime_upgraded(result.logs());
    assert_eq!(event.target_address, target_address);
    assert_ne!(
        event.code_hash,
        B256::ZERO,
        "canonical precompile EVM upgrade emitted a zero artifact hash"
    );
    assert_eq!(event.code_hash, crypto_keccak256(evm_runtime.as_ref()));

    assert_eq!(
        evm_ext_code_hash(&mut ctx, target_address),
        B256::ZERO,
        "EXTCODEHASH stopped masking a canonical precompile"
    );
}
