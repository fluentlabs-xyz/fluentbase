#[macro_export]
macro_rules! debug_unreachable {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            unreachable!($($t)*);
        } else {
            unsafe { core::hint::unreachable_unchecked() };
        }
    };
}

#[macro_export]
macro_rules! assume {
    ($e:expr $(,)?) => {
        if !$e {
            debug_unreachable!(stringify!($e));
        }
    };

    ($e:expr, $($t:tt)+) => {
        if !$e {
            debug_unreachable!($($t)+);
        }
    };
}

macro_rules! gas {
    ($translator:expr, $gas:expr) => {
        if crate::translator::USE_GAS {
            $translator
                .result_instruction_set_mut()
                .op_consume_fuel($gas);
        }
    };
}

macro_rules! gas_or_fail {
    ($translator:expr, $gas:expr) => {
        if crate::translator::USE_GAS {
            match $gas {
                Some(gas_used) => gas!($translator, gas_used),
                None => {
                    $translator.instruction_result =
                        crate::translator::instruction_result::InstructionResult::OutOfGas;
                    return;
                }
            }
        }
    };
}

macro_rules! pop {
    ($translator:expr, $x1:ident) => {
        if $translator.stack.len() < 1 {
            $translator.instruction_result =
                crate::translator::instruction_result::InstructionResult::StackUnderflow;
            return;
        }
        // Safety: Length is checked above.
        let $x1 = unsafe { $translator.stack.pop_unsafe() };
    };
    ($translator:expr, $x1:ident, $x2:ident) => {
        if $translator.stack.len() < 2 {
            $translator.instruction_result =
                crate::translator::instruction_result::InstructionResult::StackUnderflow;
            return;
        }
        // Safety: Length is checked above.
        let ($x1, $x2) = unsafe { $translator.stack.pop2_unsafe() };
    };
    ($translator:expr, $x1:ident, $x2:ident, $x3:ident) => {
        if $translator.stack.len() < 3 {
            $translator.instruction_result =
                crate::translator::instruction_result::InstructionResult::StackUnderflow;
            return;
        }
        // Safety: Length is checked above.
        let ($x1, $x2, $x3) = unsafe { $translator.stack.pop3_unsafe() };
    };

    ($translator:expr, $x1:ident, $x2:ident, $x3:ident, $x4:ident) => {
        if $translator.stack.len() < 4 {
            $translator.instruction_result =
                crate::translator::instruction_result::InstructionResult::StackUnderflow;
            return;
        }
        // Safety: Length is checked above.
        let ($x1, $x2, $x3, $x4) = unsafe { $translator.stack.pop4_unsafe() };
    };
}

macro_rules! pop_address {
    ($interp:expr, $x1:ident) => {
        if $interp.stack.len() < 1 {
            $interp.instruction_result = InstructionResult::StackUnderflow;
            return;
        }
        // Safety: Length is checked above.
        let $x1 =
            fluentbase_sdk::evm::Address::from_word(fluentbase_sdk::evm::B256::from(unsafe {
                $interp.stack.pop_unsafe()
            }));
    };
    ($interp:expr, $x1:ident, $x2:ident) => {
        if $interp.stack.len() < 2 {
            $interp.instruction_result = InstructionResult::StackUnderflow;
            return;
        }
        // Safety: Length is checked above.
        let $x1 =
            fluentbase_sdk::evm::Address::from_word(fluentbase_sdk::evm::B256::from(unsafe {
                $interp.stack.pop_unsafe()
            }));
        let $x2 =
            fluentbase_sdk::evm::Address::from_word(fluentbase_sdk::evm::B256::from(unsafe {
                $interp.stack.pop_unsafe()
            }));
    };
}

macro_rules! as_usize_or_fail {
    ($translator:expr, $v:expr) => {
        as_usize_or_fail!(
            $translator,
            $v,
            crate::translator::instruction_result::InstructionResult::InvalidOperandOOG
        )
    };

    ($translator:expr, $v:expr, $reason:expr) => {{
        let x = $v.as_limbs();
        if x[1] != 0 || x[2] != 0 || x[3] != 0 {
            $translator.instruction_result = $reason;
            return;
        }
        x[0] as usize
    }};
}

macro_rules! return_with_reason {
    ($translator:expr, $reason:expr) => {{
        $translator.instruction_result = $reason;
        return;
    }};
}
