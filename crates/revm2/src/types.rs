use core::ops::Range;
use fluentbase_sdk::evm::{Address, Bytes};
use fluentbase_types::{ExitCode, U256};
use revm_primitives::alloy_primitives::private::serde;
use revm_primitives::{CreateScheme, TransactTo, TxEnv};

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
pub struct SStoreResult {
    /// Value of the storage when it is first read
    pub original_value: U256,
    /// Current value of the storage
    pub present_value: U256,
    /// New value that is set
    pub new_value: U256,
    /// Is storage slot loaded from database
    pub is_cold: bool,
}

/// Result of a call that resulted in a self destruct.
#[derive(Default, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SelfDestructResult {
    pub had_value: bool,
    pub target_exists: bool,
    pub is_cold: bool,
    pub previously_destroyed: bool,
}

/// Represents the state of gas during execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Gas {
    /// The initial gas limit.
    limit: u64,
    /// The total used gas.
    all_used_gas: u64,
    /// Used gas without memory expansion.
    used: u64,
    /// Used gas for memory expansion.
    memory: u64,
    /// Refunded gas. This is used only at the end of execution.
    refunded: i64,
}

impl Gas {
    /// Creates a new `Gas` struct with the given gas limit.
    #[inline]
    pub const fn new(limit: u64) -> Self {
        Self {
            limit,
            used: 0,
            memory: 0,
            refunded: 0,
            all_used_gas: 0,
        }
    }

    /// Returns the gas limit.
    #[inline]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// Returns the amount of gas that was used.
    #[inline]
    pub const fn memory(&self) -> u64 {
        self.memory
    }

    /// Returns the amount of gas that was refunded.
    #[inline]
    pub const fn refunded(&self) -> i64 {
        self.refunded
    }

    /// Returns all the gas used in the execution.
    #[inline]
    pub const fn spend(&self) -> u64 {
        self.all_used_gas
    }

    /// Returns the amount of gas remaining.
    #[inline]
    pub const fn remaining(&self) -> u64 {
        self.limit - self.all_used_gas
    }

    /// Erases a gas cost from the totals.
    #[inline]
    pub fn erase_cost(&mut self, returned: u64) {
        self.used -= returned;
        self.all_used_gas -= returned;
    }

    /// Records a refund value.
    ///
    /// `refund` can be negative but `self.refunded` should always be positive
    /// at the end of transact.
    #[inline]
    pub fn record_refund(&mut self, refund: i64) {
        self.refunded += refund;
    }

    /// Set a refund value
    pub fn set_refund(&mut self, refund: i64) {
        self.refunded = refund;
    }

    /// Records an explicit cost.
    ///
    /// Returns `false` if the gas limit is exceeded.
    ///
    /// This function is called on every instruction in the interpreter if the feature
    /// `no_gas_measuring` is not enabled.
    #[inline(always)]
    pub fn record_cost(&mut self, cost: u64) -> bool {
        let all_used_gas = self.all_used_gas.saturating_add(cost);
        if self.limit < all_used_gas {
            return false;
        }

        self.used += cost;
        self.all_used_gas = all_used_gas;
        true
    }

    /// used in memory_resize! macro to record gas used for memory expansion.
    #[inline]
    pub fn record_memory(&mut self, gas_memory: u64) -> bool {
        if gas_memory > self.memory {
            let all_used_gas = self.used.saturating_add(gas_memory);
            if self.limit < all_used_gas {
                return false;
            }
            self.memory = gas_memory;
            self.all_used_gas = all_used_gas;
        }
        true
    }
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
    /// The create scheme.
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
