pub use crate::gas::Gas;
use core::ops::Range;
use fluentbase_sdk::evm::{Address, Bytes, B256};
use fluentbase_types::{ExitCode, U256};
use revm_primitives::{Bytecode, CreateScheme, Env, TransactTo, TxEnv};
use std::boxed::Box;

pub type InstructionResult = ExitCode;

#[derive(Default, Debug)]
pub struct Interpreter {
    pub contract: Box<Contract>,
    pub gas_limit: u64,
    pub is_static: bool,
    pub gas: Gas,
    pub program_counter: usize,
    pub current_opcode: u8,
}

impl Interpreter {
    pub fn new(contract: Box<Contract>, gas_limit: u64, is_static: bool) -> Self {
        Self {
            contract,
            gas_limit,
            is_static,
            ..Default::default()
        }
    }

    pub fn insert_call_outcome(
        &mut self,
        _shared_memory: &mut SharedMemory,
        _call_outcome: CallOutcome,
    ) {
    }

    pub fn insert_create_outcome(&mut self, _create_outcome: CreateOutcome) {}

    pub fn program_counter(&self) -> usize {
        self.program_counter
    }

    pub fn current_opcode(&self) -> u8 {
        self.current_opcode
    }

    pub fn gas(&self) -> &Gas {
        &self.gas
    }
}

#[macro_export]
macro_rules! return_ok {
    () => {
        fluentbase_types::ExitCode::Ok
    };
}
#[macro_export]
macro_rules! return_revert {
    () => {
        fluentbase_types::ExitCode::Panic
    };
}

/// EVM contract information.
#[derive(Clone, Debug, Default)]
pub struct Contract {
    /// Contracts data
    pub input: Bytes,
    /// Bytecode contains contract code, size of original code, analysis with gas block and jump table.
    /// Note that current code is extended with push padding and STOP at end.
    pub bytecode: Bytecode,
    /// Bytecode hash.
    pub hash: B256,
    /// Contract address
    pub address: Address,
    /// Caller of the EVM.
    pub caller: Address,
    /// Value send to contract.
    pub value: U256,
}

impl Contract {
    /// Instantiates a new contract by analyzing the given bytecode.
    #[inline]
    pub fn new(
        input: Bytes,
        bytecode: Bytecode,
        hash: B256,
        address: Address,
        caller: Address,
        value: U256,
    ) -> Self {
        Self {
            input,
            bytecode,
            hash,
            address,
            caller,
            value,
        }
    }

    /// Creates a new contract from the given [`Env`].
    #[inline]
    pub fn new_env(env: &Env, bytecode: Bytecode, hash: B256) -> Self {
        let contract_address = match env.tx.transact_to {
            TransactTo::Call(caller) => caller,
            TransactTo::Create(..) => Address::ZERO,
        };
        Self::new(
            env.tx.data.clone(),
            bytecode,
            hash,
            contract_address,
            env.tx.caller,
            env.tx.value,
        )
    }

    /// Creates a new contract from the given [`CallContext`].
    #[inline]
    pub fn new_with_context(
        input: Bytes,
        bytecode: Bytecode,
        hash: B256,
        call_context: &CallContext,
    ) -> Self {
        Self::new(
            input,
            bytecode,
            hash,
            call_context.address,
            call_context.caller,
            call_context.apparent_value,
        )
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub struct OpCode(u8);

impl OpCode {
    pub fn new(value: u8) -> Option<Self> {
        Some(Self(value))
    }

    pub fn to_string(&self) -> String {
        format!("opcode_{}", self.0).to_string()
    }
}

#[derive(Clone, Default, Debug)]
pub struct SharedMemory;

impl SharedMemory {
    #[deprecated(note = "will be removed")]
    pub fn len(&self) -> usize {
        0
    }

    #[deprecated(note = "will be removed")]
    pub fn slice(&self, _start: usize, _end: usize) -> &[u8] {
        &[]
    }
}

#[derive(Clone, Default, Debug)]
pub struct Stack;

impl Stack {
    #[deprecated(note = "will be removed")]
    pub fn len(&self) -> usize {
        0
    }

    #[deprecated(note = "will be removed")]
    pub fn peek(&self, _index: usize) -> Result<U256, ExitCode> {
        Ok(U256::ZERO)
    }
}

#[allow(non_camel_case_types)]
pub(crate) enum BytecodeType {
    EVM,
    WASM,
}

impl BytecodeType {
    pub(crate) fn from_slice(input: &[u8]) -> Self {
        if input.len() >= 4 && input[0..4] == [0x00, 0x61, 0x73, 0x6d] {
            Self::WASM
        } else {
            Self::EVM
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterResult {
    /// The result of the instruction execution.
    pub result: ExitCode,
    /// The output of the instruction execution.
    pub output: Bytes,
    /// The gas usage information.
    pub gas: Gas,
}

/// Represents the result of an `sstore` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct SStoreResult {
    /// Value of the storage when it is first read
    pub(crate) original_value: U256,
    /// Current value of the storage
    pub(crate) present_value: U256,
    /// New value that is set
    pub(crate) new_value: U256,
    /// Is storage slot loaded from database
    pub(crate) is_cold: bool,
}

/// Result of a call that resulted in a self destruct.
#[derive(Default, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct SelfDestructResult {
    pub(crate) had_value: bool,
    pub(crate) target_exists: bool,
    pub(crate) is_cold: bool,
    pub(crate) previously_destroyed: bool,
}

/// Transfer from source to target, with given value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transfer {
    /// The source address.
    pub source: Address,
    /// The target address.
    pub target: Address,
    /// The transfer value.
    pub value: U256,
}

/// Call schemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallScheme {
    /// `CALL`.
    Call,
    /// `CALLCODE`
    CallCode,
    /// `DELEGATECALL`
    DelegateCall,
    /// `STATICCALL`
    StaticCall,
}

/// Context of a runtime call.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallContext {
    /// Execution address.
    pub address: Address,
    /// Caller address of the EVM.
    pub caller: Address,
    /// The address the contract code was loaded from, if any.
    pub code_address: Address,
    /// Apparent value of the EVM.
    pub apparent_value: U256,
    /// The scheme used for the call.
    pub scheme: CallScheme,
}

impl Default for CallContext {
    fn default() -> Self {
        CallContext {
            address: Address::default(),
            caller: Address::default(),
            code_address: Address::default(),
            apparent_value: U256::default(),
            scheme: CallScheme::Call,
        }
    }
}

/// Inputs for a call.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallInputs {
    /// The target of the call.
    pub contract: Address,
    /// The transfer, if any, in this call.
    pub transfer: Transfer,
    /// The call data of the call.
    pub input: Bytes,
    /// The gas limit of the call.
    pub gas_limit: u64,
    /// The context of the call.
    pub context: CallContext,
    /// Whether this is a static call.
    pub is_static: bool,
    /// The return memory offset where the output of the call is written.
    pub return_memory_offset: Range<usize>,
}

/// Inputs for a create call.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CreateInputs {
    /// Caller address of the EVM.
    pub caller: Address,
    /// The creation scheme.
    pub scheme: CreateScheme,
    /// The value to transfer.
    pub value: U256,
    /// The init code of the contract.
    pub init_code: Bytes,
    /// The gas limit of the call.
    pub gas_limit: u64,
}

impl CallInputs {
    /// Creates new call inputs.
    pub fn new(tx_env: &TxEnv, gas_limit: u64) -> Option<Self> {
        let TransactTo::Call(address) = tx_env.transact_to else {
            return None;
        };

        Some(CallInputs {
            contract: address,
            transfer: Transfer {
                source: tx_env.caller,
                target: address,
                value: tx_env.value,
            },
            input: tx_env.data.clone(),
            gas_limit,
            context: CallContext {
                caller: tx_env.caller,
                address,
                code_address: address,
                apparent_value: tx_env.value,
                scheme: CallScheme::Call,
            },
            is_static: false,
            return_memory_offset: 0..0,
        })
    }

    /// Returns boxed call inputs.
    pub fn new_boxed(tx_env: &TxEnv, gas_limit: u64) -> Option<Box<Self>> {
        Self::new(tx_env, gas_limit).map(Box::new)
    }
}

impl CreateInputs {
    /// Creates new create inputs.
    pub fn new(tx_env: &TxEnv, gas_limit: u64) -> Option<Self> {
        let TransactTo::Create(scheme) = tx_env.transact_to else {
            return None;
        };

        Some(CreateInputs {
            caller: tx_env.caller,
            scheme,
            value: tx_env.value,
            init_code: tx_env.data.clone(),
            gas_limit,
        })
    }

    /// Returns boxed create inputs.
    pub fn new_boxed(tx_env: &TxEnv, gas_limit: u64) -> Option<Box<Self>> {
        Self::new(tx_env, gas_limit).map(Box::new)
    }

    /// Returns the address that this create call will create.
    pub fn created_address(&self, nonce: u64) -> Address {
        match self.scheme {
            CreateScheme::Create => self.caller.create(nonce),
            CreateScheme::Create2 { salt } => self
                .caller
                .create2_from_code(salt.to_be_bytes(), &self.init_code),
        }
    }
}

/// Represents the outcome of a create operation in an interpreter.
///
/// This struct holds the result of the operation along with an optional address.
/// It provides methods to determine the next action based on the result of the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOutcome {
    // The result of the interpreter operation.
    pub result: InterpreterResult,
    // An optional address associated with the create operation.
    pub address: Option<Address>,
}

impl CreateOutcome {
    /// Constructs a new `CreateOutcome`.
    ///
    /// # Arguments
    ///
    /// * `result` - An `InterpreterResult` representing the result of the interpreter operation.
    /// * `address` - An optional `Address` associated with the create operation.
    ///
    /// # Returns
    ///
    /// A new `CreateOutcome` instance.
    pub fn new(result: InterpreterResult, address: Option<Address>) -> Self {
        Self { result, address }
    }

    /// Retrieves a reference to the `InstructionResult` from the `InterpreterResult`.
    ///
    /// This method provides access to the `InstructionResult` which represents the
    /// outcome of the instruction execution. It encapsulates the result information
    /// such as whether the instruction was executed successfully, resulted in a revert,
    /// or encountered a fatal error.
    ///
    /// # Returns
    ///
    /// A reference to the `InstructionResult`.
    pub fn instruction_result(&self) -> &ExitCode {
        &self.result.result
    }

    /// Retrieves a reference to the output bytes from the `InterpreterResult`.
    ///
    /// This method returns the output of the interpreted operation. The output is
    /// typically used when the operation successfully completes and returns data.
    ///
    /// # Returns
    ///
    /// A reference to the output `Bytes`.
    pub fn output(&self) -> &Bytes {
        &self.result.output
    }

    /// Retrieves a reference to the `Gas` details from the `InterpreterResult`.
    ///
    /// This method provides access to the gas details of the operation, which includes
    /// information about gas used, remaining, and refunded. It is essential for
    /// understanding the gas consumption of the operation.
    ///
    /// # Returns
    ///
    /// A reference to the `Gas` details.
    pub fn gas(&self) -> &Gas {
        &self.result.gas
    }
}

/// Represents the outcome of a call operation in a virtual machine.
///
/// This struct encapsulates the result of executing an instruction by an interpreter, including
/// the result itself, gas usage information, and the memory offset where output data is stored.
///
/// # Fields
///
/// * `result` - The result of the interpreter's execution, including output data and gas usage.
/// * `memory_offset` - The range in memory where the output data is located.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallOutcome {
    pub result: InterpreterResult,
    pub memory_offset: Range<usize>,
}

impl CallOutcome {
    /// Constructs a new `CallOutcome`.
    ///
    /// Creates an instance of `CallOutcome` with the given interpreter result and memory offset.
    ///
    /// # Arguments
    ///
    /// * `result` - The result of the interpreter's execution.
    /// * `memory_offset` - The range in memory indicating where the output data is stored.
    pub fn new(result: InterpreterResult, memory_offset: Range<usize>) -> Self {
        Self {
            result,
            memory_offset,
        }
    }

    /// Returns a reference to the instruction result.
    ///
    /// Provides access to the result of the executed instruction.
    ///
    /// # Returns
    ///
    /// A reference to the `InstructionResult`.
    pub fn instruction_result(&self) -> &ExitCode {
        &self.result.result
    }

    /// Returns the gas usage information.
    ///
    /// Provides access to the gas usage details of the executed instruction.
    ///
    /// # Returns
    ///
    /// An instance of `Gas` representing the gas usage.
    pub fn gas(&self) -> Gas {
        self.result.gas
    }

    /// Returns a reference to the output data.
    ///
    /// Provides access to the output data generated by the executed instruction.
    ///
    /// # Returns
    ///
    /// A reference to the output data as `Bytes`.
    pub fn output(&self) -> &Bytes {
        &self.result.output
    }

    /// Returns the start position of the memory offset.
    ///
    /// Provides the starting index of the memory range where the output data is stored.
    ///
    /// # Returns
    ///
    /// The starting index of the memory offset as `usize`.
    pub fn memory_start(&self) -> usize {
        self.memory_offset.start
    }

    /// Returns the length of the memory range.
    ///
    /// Provides the length of the memory range where the output data is stored.
    ///
    /// # Returns
    ///
    /// The length of the memory range as `usize`.
    pub fn memory_length(&self) -> usize {
        self.memory_offset.len()
    }
}
