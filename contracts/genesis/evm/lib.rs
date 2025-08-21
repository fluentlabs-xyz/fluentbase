#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;
extern crate core;

use fluentbase_sdk::{
    bytes::Buf, entrypoint, keccak256, Address, Bytes, ContextReader, ExitCode, Log, SharedAPI,
    B256, EVM_MAX_CODE_SIZE, KECCAK_EMPTY, U256, WASM_MAX_CODE_SIZE,
};
use revm_bytecode::Bytecode;
use revm_context_interface::journaled_state::AccountLoad;
use revm_interpreter::interpreter::{EthInterpreter, ExtBytecode};
use revm_interpreter::{
    gas, instruction_table, Host, InputsImpl, Instruction, Interpreter, InterpreterResult,
    SStoreResult, SelfDestructResult, SharedMemory, StateLoad,
};
use revm_primitives::hardfork::SpecId;
use revm_primitives::{StorageKey, StorageValue, BLOCK_HASH_HISTORY};

/// Indicates whether analyzed EVM bytecode should be cached.
/// Set to `false` to disable caching, which may result in repeated analysis and potentially slower
/// performance but ensures the latest results are always used.
pub const CACHE_ANALYZED_EVM_BYTECODE: bool = false;

/// Commits EVM bytecode to persistent storage and updates the corresponding code hash.
///
/// This function performs the following operations:
/// 1. Write the provided bytecode (`evm_bytecode`) to preimage storage using the SDK, which returns
///    a hash of the preimage.
/// 2. Takes the resulting code hash and writes it to a predefined storage slot identified by
///    `EVM_CODE_HASH_SLOT`.
///
/// # Arguments
/// - `sdk`: A mutable reference to the SDK instance implementing the `SharedAPI` trait, which
///   provides the methods required for interactions with storage.
/// - `evm_bytecode`: A `Bytes` object containing the EVM bytecode to be stored.
pub(crate) fn commit_evm_bytecode<const CACHE_ANALYZED: bool, SDK: SharedAPI>(
    sdk: &mut SDK,
    evm_bytecode: Bytes,
) {
    let contract_address = sdk.context().contract_address();
    let evm_code_hash = keccak256(evm_bytecode.as_ref());
    // write an EVM code hash & bytecode into metadata at offset 0
    let mut metadata = evm_code_hash.as_slice().to_vec();
    metadata.extend(evm_bytecode);
    sdk.metadata_write(&contract_address, 0, metadata.into())
        .unwrap();
}

/// Loads the EVM bytecode associated with the contract using the provided SDK.
///
/// This function retrieves the EVM bytecode for a contract from the state storage
/// using a delegated storage mechanism. The process involves fetching the contract's
/// bytecode address, locating the storage slot for its EVM code hash, and verifying
/// if the bytecode exists (i.e., it is not empty). If valid bytecode is found, it is loaded
/// and returned as a `Bytecode` object.
///
/// # Arguments
/// - `sdk`: A reference to an implementation of the `SharedAPI` trait that provides access to
///   storage, context, and pre-image retrieval methods required for handling contract data.
///
/// # Returns
/// An `Option<Bytecode>`.
/// - `Some(Bytecode)`: If a valid bytecode exists and is successfully retrieved.
/// - `None`: If the bytecode is empty or not present in the storage.
pub(crate) fn load_evm_bytecode<const CACHE_ANALYZED: bool, SDK: SharedAPI>(
    sdk: &SDK,
) -> Option<AnalyzedBytecode> {
    // we use bytecode address because contract can be called using DELEGATECALL
    let bytecode_address = sdk.context().contract_bytecode_address();
    // read metadata size, if it's zero, then an account is not assigned to the EVM runtime
    let (metadata_size, _, _, _) = sdk.metadata_size(&bytecode_address).unwrap();
    if metadata_size == 0 {
        return None;
    }
    let mut metadata = sdk
        .metadata_copy(&bytecode_address, 0, metadata_size)
        .unwrap();
    assert!(
        metadata.len() >= 32,
        "can't load EVM bytecode: metadata is too small"
    );
    // load EVM bytecode hash and exit if the code hash is empty
    let evm_code_hash = B256::from_slice(&metadata[0..32]);
    // TODO(dmitry123): "do we want to have this optimized during the creation of the frame?"
    let is_empty_bytecode = evm_code_hash == B256::ZERO || evm_code_hash == KECCAK_EMPTY;
    if is_empty_bytecode {
        return None;
    }
    // skip the first 32 bytes (code hash)
    metadata.advance(32);
    let analyzed_bytecode = AnalyzedBytecode::new(&metadata[..], evm_code_hash.into());
    Some(analyzed_bytecode)
}

/// Handles non-OK results from the EVM interpreter.
///
/// This function performs the following:
/// 1. Synchronizes the remaining and refunded gas with the `SharedAPI` instance.
/// 2. Write the output of the interpreter result to the `SharedAPI` instance.
/// 3. If the result is a revert, immediately exits with a panic exit code.
/// 4. Determine the final exit code based on the `InstructionResult` value and exits accordingly.
///
/// ## Parameters:
/// - `sdk`: A mutable instance of a type implementing the `SharedAPI` trait to interface with the
///   environment.
/// - `result`: The `InterpreterResult` containing the gas usage, output, and result status.
///
/// ## Exit Codes:
/// - `ExitCode::Ok` (0) for successful instructions.
/// - `ExitCode::Panic` (-1) for revert cases.
/// - `ExitCode::Err` (-2) for any other error conditions.
///
/// By interpreting and mapping results appropriately, this function ensures
/// the correct handling and propagation of results from the EVM context.
fn handle_not_ok_result<SDK: SharedAPI>(mut sdk: SDK, result: InterpreterResult) {
    let (consumed_diff, refund_diff) = result.chargeable_fuel_and_refund();
    sdk.charge_fuel_manually(consumed_diff, refund_diff);
    sdk.write(result.output.as_ref());
    sdk.native_exit(if result.is_revert() {
        ExitCode::Panic
    } else {
        ExitCode::Err
    });
}

struct HostAdapter<SDK: SharedAPI> {
    sdk: SDK,
}

impl<SDK: SharedAPI> Host for HostAdapter<SDK> {
    fn basefee(&self) -> U256 {
        self.sdk.context().block_base_fee()
    }

    fn blob_gasprice(&self) -> U256 {
        U256::ZERO
    }

    fn gas_limit(&self) -> U256 {
        self.sdk.context().tx_gas_limit().into()
    }

    fn difficulty(&self) -> U256 {
        self.sdk.context().block_difficulty()
    }

    fn prevrandao(&self) -> Option<U256> {
        Some(self.sdk.context().block_prev_randao().into())
    }

    fn block_number(&self) -> U256 {
        self.sdk.context().block_number().into()
    }

    fn timestamp(&self) -> U256 {
        self.sdk.context().block_timestamp().into()
    }

    fn beneficiary(&self) -> Address {
        self.sdk.context().block_coinbase()
    }

    fn chain_id(&self) -> U256 {
        self.sdk.context().block_chain_id().into()
    }

    fn effective_gas_price(&self) -> U256 {
        self.sdk.context().tx_gas_price()
    }

    fn caller(&self) -> Address {
        self.sdk.context().contract_caller()
    }

    fn blob_hash(&self, _number: usize) -> Option<U256> {
        None
    }

    fn max_initcode_size(&self) -> usize {
        WASM_MAX_CODE_SIZE
    }

    fn block_hash(&mut self, number: u64) -> Option<B256> {
        let hash = self.sdk.block_hash(number).unwrap();
        Some(hash)
    }

    fn selfdestruct(
        &mut self,
        address: Address,
        target: Address,
    ) -> Option<StateLoad<SelfDestructResult>> {
        self.sdk.destroy_account(target);
    }

    fn log(&mut self, log: Log) {
        self.sdk.emit_log(log.topics(), log.data.data.as_ref());
    }

    fn sstore(
        &mut self,
        address: Address,
        key: StorageKey,
        value: StorageValue,
    ) -> Option<StateLoad<SStoreResult>> {
        todo!()
    }

    fn sload(&mut self, address: Address, key: StorageKey) -> Option<StateLoad<StorageValue>> {
        todo!()
    }

    fn tstore(&mut self, address: Address, key: StorageKey, value: StorageValue) {
        todo!()
    }

    fn tload(&mut self, address: Address, key: StorageKey) -> StorageValue {
        todo!()
    }

    fn balance(&mut self, address: Address) -> Option<StateLoad<U256>> {
        self.sdk.balance(&address);
    }

    fn load_account_delegated(&mut self, address: Address) -> Option<StateLoad<AccountLoad>> {
        todo!()
    }

    fn load_account_code(&mut self, address: Address) -> Option<StateLoad<Bytes>> {
        todo!()
    }

    fn load_account_code_hash(&mut self, address: Address) -> Option<StateLoad<B256>> {
        todo!()
    }
}

/// Deploys an EVM smart contract using the provided bytecode input.
///
/// This function handles the deployment process for EVM-compatible smart contracts,
/// including executing the contract bytecode, ensuring compliance with EVM specifications,
/// and committing the deployed bytecode if the deployment is successful.
///
/// # Steps:
/// 1. **Fetch Input and Context**:
///    - Retrieves the input bytecode for contract deployment using the SDK.
///    - Obtains the gas limit for the deployment from the context.
///
/// 2. **Execute EVM Bytecode**:
///    - Executes the provided bytecode via the `exec_evm_bytecode` function.
///    - If the execution fails, the non-success result is processed by `handle_not_ok_result` and
///      the function terminates early.
///
/// 3. **EIP-3541 (Disallow Code Starting with 0xEF)**:
///    - Checks if the executed contract output begins with the byte `0xEF` (non-standard prefix).
///    - If so, exits with the error code `InstructionResult::CreateContractStartingWithEF`.
///
/// 4. **EIP-170 (Code Size Limit)**:
///    - Verifies if the length of the generated bytecode exceeds 24KB, specified by
///      `MAX_CODE_SIZE`.
///    - Exits with `InstructionResult::CreateContractSizeLimit` error if the limit is exceeded.
///
/// 5. **Gas Cost for Code Deposit**:
///    - Calculates the gas cost for storing the deployed bytecode based on `CODEDEPOSIT` (a
///      predefined gas constant) and the bytecode size.
///    - If the cost cannot be recorded (due to insufficient gas), charge the maximum fuel and exits
///      accordingly.
///
/// 6. **Synchronize Gas Information**:
///    - Updates the EVM gas state (remaining and refunded gas) in the SDK to keep it synchronized
///      with the deployment process.
///
/// 7. **Commit Bytecode**:
///    - Saves the deployed contract bytecode to persistent storage using `commit_evm_bytecode`.
///
/// This function ensures compatibility with fundamental Ethereum standards and handles
/// gas calculations, runtime checks, and storage updates as part of the deployment flow.
///
/// # Parameters
/// - `sdk`: A mutable reference to the SDK instance that implements the `SharedAPI` trait.
///
/// # Errors
/// The function can exit under various error conditions:
/// - Non-successful EVM bytecode execution.
/// - Code starting with 0xEF (EIP-3541 violation).
/// - Code exceeding the size limit (EIP-170 violation).
/// - Insufficient gas for code deposit.
///
/// # Gas Mechanics
/// - Gas is deducted during the bytecode execution and additional deployment steps.
/// - Compatibility with EVM gas mechanisms is maintained to ensure Ethereum-like behavior.
pub fn deploy<SDK: SharedAPI>(mut sdk: SDK) {
    let input: Bytes = sdk.input().into();

    let is_static = sdk.context().contract_is_static();
    let gas_limit = sdk.context().contract_gas_limit();

    let shared_memory = SharedMemory::new();

    let bytecode = Bytecode::new_raw_checked(input)
        .unwrap_or_else(|| unreachable!("can't decode evm init bytecode"));
    match bytecode {
        Bytecode::LegacyAnalyzed(_) => {}
        _ => unreachable!("not supported evm init bytecode"),
    }
    let bytecode = ExtBytecode::new_with_hash(bytecode, B256::ZERO);

    let inputs = InputsImpl {
        target_address: Default::default(),
        bytecode_address: None,
        caller_address: Default::default(),
        input: Default::default(),
        call_value: Default::default(),
        account_owner: None,
    };

    let mut interpreter = Interpreter::new(
        shared_memory,
        bytecode,
        inputs,
        is_static,
        SpecId::PRAGUE,
        gas_limit,
    );
    const INSTRUCTION_TABLE: [Instruction<EthInterpreter, HostAdapter<SDK>>; 256] =
        instruction_table();
    interpreter.run_plain();

    let mut result = EVM::new(&mut sdk, analyzed_bytecode, &[], gas_limit).exec();
    if !result.is_ok() {
        return handle_not_ok_result(sdk, result);
    }

    // EIP-3541 and EIP-170 checks
    if result.output.first() == Some(&0xEF) {
        sdk.native_exit(ExitCode::PrecompileError);
    } else if result.output.len() > EVM_MAX_CODE_SIZE {
        sdk.native_exit(ExitCode::PrecompileError);
    }
    let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
    if !result.gas.record_cost(gas_for_code) {
        sdk.native_exit(ExitCode::OutOfFuel);
    }

    let (consumed_diff, refund_diff) = result.chargeable_fuel_and_refund();
    sdk.charge_fuel_manually(consumed_diff, refund_diff);

    // we intentionally don't charge gas for these opcodes
    // to keep full compatibility with an EVM deployment process
    commit_evm_bytecode::<{ CACHE_ANALYZED_EVM_BYTECODE }, SDK>(&mut sdk, result.output);
}

/// The main entry point function of the application that processes EVM-based contract bytecode.
///
/// This function interacts with an environment (`SharedAPI`) to execute EVM bytecode
/// with input data under a specified gas limit.
/// The results are then processed, handled, and written back to the environment.
///
/// ### Key Steps:
/// 1. Load the EVM bytecode for the specific contract using `load_evm_bytecode`.
///    - If the bytecode is not available (e.g., invalid or absent), the function terminates early.
/// 2. Retrieve the input data provided by the environment via `sdk.input()`.
/// 3. Fetch the gas limit for the contract execution from the environment's `contract_gas_limit`.
/// 4. Execute EVM bytecode with `exec_evm_bytecode`, passing:
///    - Loaded bytecode
///    - Input data
///    - Gas limit
/// 5. Check the result of the execution:
///    - If unsuccessful, handle the failure gracefully using `handle_not_ok_result`.
///    - If successful, sync gas usage (`remaining` and `refunded` gas) via `sdk.sync_evm_gas`, and
///      write the execution output back with `sdk.write`.
///
/// ### Parameters:
/// - `sdk`: An instance implementing `SharedAPI` to provide runtime functionality, such as
///   input/output handling, gas synchronization, and context details.
///
/// ### Detailed Behavior:
/// This function ensures that gas usage and execution outputs are synchronously managed
/// between the SDK environment and the virtual machine.
/// The error-handling mechanism ensures
/// that non-successful results terminate the function with appropriate actions, such as panic
/// (`revert`) or error logging.
///
/// ### Assumptions:
/// - The SDK instance conforms to the `SharedAPI` interface.
/// - Bytecode is preloaded and valid for the specific context where the function is executed.
pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let Some(evm_bytecode) = load_evm_bytecode::<{ CACHE_ANALYZED_EVM_BYTECODE }, SDK>(&sdk) else {
        return;
    };

    let input: Bytes = sdk.input().into();
    let gas_limit = sdk.context().contract_gas_limit();

    let result = EVM::new(&mut sdk, evm_bytecode, &input[..], gas_limit).exec();
    if !result.is_ok() {
        return handle_not_ok_result(sdk, result);
    }

    let (consumed_diff, refund_diff) = result.chargeable_fuel_and_refund();
    sdk.charge_fuel_manually(consumed_diff, refund_diff);

    sdk.write(result.output.as_ref());
}

entrypoint!(main_entry, deploy);

#[cfg(test)]
mod tests {
    use crate::{deploy, main_entry};
    use core::str::from_utf8;
    use fluentbase_sdk::{hex, Address, ContractContextV1, U256};
    use fluentbase_sdk_testing::HostTestingContext;

    #[ignore]
    #[test]
    fn test_deploy_greeting() {
        const CONTRACT_ADDRESS: Address = Address::new([
            189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111, 164, 142,
            177,
        ]);
        let mut sdk = HostTestingContext::default()
            .with_contract_context(ContractContextV1 {
                address: CONTRACT_ADDRESS,
                bytecode_address: CONTRACT_ADDRESS,
                caller: Address::ZERO,
                is_static: false,
                value: U256::ZERO,
                gas_limit: 1_000_000,
            })
            .with_gas_limit(1_000_000);
        // deploy
        {
            sdk = sdk.with_input(hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033"));
            deploy(sdk.clone());
        }
        // main
        {
            let sdk = sdk.with_input(hex!("45773e4e"));
            main_entry(sdk.clone());
            let bytes = &sdk.take_output()[64..75];
            assert_eq!("Hello World", from_utf8(bytes.as_ref()).unwrap());
        }
    }
}
