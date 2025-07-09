use crate::memory::num_words;
use fluentbase_sdk::U256;

pub const BASE: u64 = 2;
pub const VERY_LOW: u64 = 3;

pub const LOW: u64 = 5;
pub const MID: u64 = 8;
pub const HIGH: u64 = 10;
pub const JUMPDEST: u64 = 1;
pub const CREATE: u64 = 32000;
pub const EXP: u64 = 10;
pub const MEMORY: u64 = 3;
pub const KECCAK256: u64 = 30;
pub const KECCAK256WORD: u64 = 6;
pub const COPY: u64 = 3;
pub const BLOCKHASH: u64 = 20;
pub const CODEDEPOSIT: u64 = 200;

// berlin eip2929 constants
pub const COLD_ACCOUNT_ACCESS_COST: u64 = 2600;
pub const WARM_STORAGE_READ_COST: u64 = 100;

/// EIP-3860 : Limit and meter initcode
pub const INITCODE_WORD_COST: u64 = 2;

/// Represents the state of gas during execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Gas {
    /// The initial gas limit. This is constant throughout execution.
    limit: u64,
    /// The remaining gas.
    remaining: u64,
    /// Refunded gas. This is used only at the end of execution.
    refunded: i64,
}

impl Gas {
    /// Creates a new `Gas` struct with the given gas limit.
    #[inline]
    pub const fn new(limit: u64) -> Self {
        Self {
            limit,
            remaining: limit,
            refunded: 0,
        }
    }

    /// Returns the total amount of gas that was refunded.
    #[inline]
    pub const fn refunded(&self) -> i64 {
        self.refunded
    }

    /// Returns the amount of gas remaining.
    #[inline]
    pub const fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Records a refund value.
    ///
    /// `refund` can be negative but `self.refunded` should always be positive
    /// at the end of transact.
    #[inline]
    pub fn record_refund(&mut self, refund: i64) {
        self.refunded += refund;
    }

    /// Records an explicit cost.
    ///
    /// Returns `false` if the gas limit is exceeded.
    #[inline]
    #[must_use = "prefer using `gas!` instead to return an out-of-gas error on failure"]
    pub fn record_cost(&mut self, cost: u64) -> bool {
        // #[cfg(target_arch = "wasm32")]
        // unsafe {
        //     let message = alloc::format!("record cost: {}, remaining={}", cost, self.remaining);
        //     #[link(wasm_import_module = "fluentbase_v1preview")]
        //     extern "C" {
        //         fn _debug_log(msg_ptr: *const u8, msg_len: u32);
        //     }
        //     _debug_log(message.as_ptr(), message.len() as u32);
        // }
        let (remaining, overflow) = self.remaining().overflowing_sub(cost);
        let success = !overflow;
        if success {
            self.remaining = remaining;
        }
        success
    }
}

/// `const` Option `?`.
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return None,
        }
    };
}

/// `CREATE2` opcode cost calculation.
#[inline]
pub const fn create2_cost(len: u64) -> Option<u64> {
    CREATE.checked_add(tri!(cost_per_word(len, KECCAK256WORD)))
}

#[inline]
const fn log2floor(value: U256) -> u64 {
    let mut l: u64 = 256;
    let mut i = 3;
    loop {
        if value.as_limbs()[i] == 0u64 {
            l -= 64;
        } else {
            l -= value.as_limbs()[i].leading_zeros() as u64;
            if l == 0 {
                return l;
            } else {
                return l - 1;
            }
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    l
}

/// `EXP` opcode cost calculation.
#[inline]
pub fn exp_cost(power: U256) -> Option<u64> {
    if power.is_zero() {
        Some(EXP)
    } else {
        // EIP-160: EXP cost increase
        let gas_byte = U256::from(50);
        let gas = U256::from(EXP)
            .checked_add(gas_byte.checked_mul(U256::from(log2floor(power) / 8 + 1))?)?;

        u64::try_from(gas).ok()
    }
}

/// `*COPY` opcodes cost calculation.
#[inline]
pub const fn verylowcopy_cost(len: u64) -> Option<u64> {
    VERY_LOW.checked_add(tri!(cost_per_word(len, COPY)))
}

/// `EXTCODECOPY` opcode cost calculation.
#[inline]
pub const fn extcodecopy_cost(len: u64, is_cold: bool) -> Option<u64> {
    let base_gas = warm_cold_cost(is_cold);
    base_gas.checked_add(tri!(cost_per_word(len, COPY)))
}

/// `KECCAK256` opcode cost calculation.
#[inline]
pub const fn keccak256_cost(len: u64) -> Option<u64> {
    KECCAK256.checked_add(tri!(cost_per_word(len, KECCAK256WORD)))
}

/// Calculate the cost of buffer per word.
#[inline]
pub const fn cost_per_word(len: u64, multiple: u64) -> Option<u64> {
    multiple.checked_mul(num_words(len))
}

/// EIP-3860: Limit and meter initcode
///
/// Apply extra gas cost of 2 for every 32-byte chunk of initcode.
///
/// This cannot overflow as the initcode length is assumed to be checked.
#[inline]
pub const fn initcode_cost(len: u64) -> u64 {
    let Some(cost) = cost_per_word(len, INITCODE_WORD_COST) else {
        panic!("initcode cost overflow")
    };
    cost
}

/// Berlin warm and cold storage access cost for account access.
#[inline]
pub const fn warm_cold_cost(is_cold: bool) -> u64 {
    if is_cold {
        COLD_ACCOUNT_ACCESS_COST
    } else {
        WARM_STORAGE_READ_COST
    }
}

/// Memory expansion cost calculation for a given memory length.
#[inline]
pub const fn memory_gas_for_len(len: usize) -> u64 {
    memory_gas(num_words(len as u64))
}

/// Memory expansion cost calculation for a given number of words.
#[inline]
pub const fn memory_gas(num_words: u64) -> u64 {
    MEMORY
        .saturating_mul(num_words)
        .saturating_add(num_words.saturating_mul(num_words) / 512)
}
