use crate::{as_usize_saturated, evm::i256::i256_cmp, gas, pop_top, EVM};
use core::cmp::Ordering;
use fluentbase_sdk::{SharedAPI, U256};

pub fn lt<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = U256::from(op1 < *op2);
}

pub fn gt<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = U256::from(op1 > *op2);
}

pub fn slt<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = U256::from(i256_cmp(&op1, op2) == Ordering::Less);
}

pub fn sgt<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = U256::from(i256_cmp(&op1, op2) == Ordering::Greater);
}

pub fn eq<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = U256::from(op1 == *op2);
}

pub fn iszero<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1);
    *op1 = U256::from(op1.is_zero());
}

pub fn bitand<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = op1 & *op2;
}

pub fn bitor<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = op1 | *op2;
}

pub fn bitxor<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = op1 ^ *op2;
}

pub fn not<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1);
    *op1 = !*op1;
}

pub fn byte<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    let o1 = as_usize_saturated!(op1);
    *op2 = if o1 < 32 {
        // `31 - o1` because `byte` returns LE, while we want BE
        U256::from(op2.byte(31 - o1))
    } else {
        U256::ZERO
    };
}

/// EIP-145: Bitwise shifting instructions in EVM
pub fn shl<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    let shift = as_usize_saturated!(op1);
    *op2 = if shift < 256 {
        *op2 << shift
    } else {
        U256::ZERO
    }
}

/// EIP-145: Bitwise shifting instructions in EVM
pub fn shr<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    let shift = as_usize_saturated!(op1);
    *op2 = if shift < 256 {
        *op2 >> shift
    } else {
        U256::ZERO
    }
}

/// EIP-145: Bitwise shifting instructions in EVM
pub fn sar<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    let shift = as_usize_saturated!(op1);
    *op2 = if shift < 256 {
        op2.arithmetic_shr(shift)
    } else if op2.bit(255) {
        U256::MAX
    } else {
        U256::ZERO
    };
}

#[cfg(test)]
mod tests {
    use crate::{
        bytecode::AnalyzedBytecode,
        evm::{
            bitwise::{byte, sar, shl, shr},
            EVM,
        },
        pop,
        push,
    };
    use fluentbase_sdk::{uint, U256};
    use fluentbase_sdk_testing::HostTestingContext;
    use url::Host;

    #[test]
    fn test_shift_left() {
        let mut sdk = HostTestingContext::default();
        let mut evm = EVM::new(&mut sdk, AnalyzedBytecode::default(), &[], u64::MAX);

        struct TestCase {
            value: U256,
            shift: U256,
            expected: U256,
        }

        uint! {
            let test_cases = [
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0x00_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0x01_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000002_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0xff_U256,
                    expected: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0x0100_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0x0101_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x00_U256,
                    expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x01_U256,
                    expected: 0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0xff_U256,
                    expected: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x0100_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                    shift: 0x01_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x01_U256,
                    expected: 0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe_U256,
                },
            ];
        }

        for test in test_cases {
            push!(evm, test.value);
            push!(evm, test.shift);
            shl(&mut evm);
            pop!(evm, res);
            assert_eq!(res, test.expected);
        }
    }

    #[test]
    fn test_logical_shift_right() {
        let mut sdk = HostTestingContext::default();
        let mut evm = EVM::new(&mut sdk, AnalyzedBytecode::default(), &[], u64::MAX);

        struct TestCase {
            value: U256,
            shift: U256,
            expected: U256,
        }

        uint! {
            let test_cases = [
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0x00_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                    shift: 0x01_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                    shift: 0x01_U256,
                    expected: 0x4000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                    shift: 0xff_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                },
                TestCase {
                    value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                    shift: 0x0100_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                    shift: 0x0101_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x00_U256,
                    expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x01_U256,
                    expected: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0xff_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    shift: 0x0100_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
                TestCase {
                    value: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                    shift: 0x01_U256,
                    expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                },
            ];
        }

        for test in test_cases {
            push!(evm, test.value);
            push!(evm, test.shift);
            shr(&mut evm);
            pop!(evm, res);
            assert_eq!(res, test.expected);
        }
    }

    #[test]
    fn test_arithmetic_shift_right() {
        let mut sdk = HostTestingContext::default();
        let mut evm = EVM::new(&mut sdk, AnalyzedBytecode::default(), &[], u64::MAX);

        struct TestCase {
            value: U256,
            shift: U256,
            expected: U256,
        }

        uint! {
        let test_cases = [
            TestCase {
                value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                shift: 0x00_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
            },
            TestCase {
                value: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
                shift: 0x01_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
            },
            TestCase {
                value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                shift: 0x01_U256,
                expected: 0xc000000000000000000000000000000000000000000000000000000000000000_U256,
            },
            TestCase {
                value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                shift: 0xff_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                shift: 0x0100_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256,
                shift: 0x0101_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0x00_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0x01_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0xff_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0x0100_U256,
                expected: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
            },
            TestCase {
                value: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
                shift: 0x01_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
            },
            TestCase {
                value: 0x4000000000000000000000000000000000000000000000000000000000000000_U256,
                shift: 0xfe_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
            },
            TestCase {
                value: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0xf8_U256,
                expected: 0x000000000000000000000000000000000000000000000000000000000000007f_U256,
            },
            TestCase {
                value: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0xfe_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000001_U256,
            },
            TestCase {
                value: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0xff_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
            },
            TestCase {
                value: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                shift: 0x0100_U256,
                expected: 0x0000000000000000000000000000000000000000000000000000000000000000_U256,
            },
        ];
            }

        for test in test_cases {
            push!(evm, test.value);
            push!(evm, test.shift);
            sar(&mut evm);
            pop!(evm, res);
            assert_eq!(res, test.expected);
        }
    }

    #[test]
    fn test_byte() {
        struct TestCase {
            input: U256,
            index: usize,
            expected: U256,
        }

        let mut sdk = HostTestingContext::default();
        let mut evm = EVM::new(&mut sdk, AnalyzedBytecode::default(), &[], u64::MAX);

        let input_value = U256::from(0x1234567890abcdef1234567890abcdef_u128);
        let test_cases = (0..32)
            .map(|i| {
                let byte_pos = 31 - i;

                let shift_amount = U256::from(byte_pos * 8);
                let byte_value = (input_value >> shift_amount) & U256::from(0xFF);
                TestCase {
                    input: input_value,
                    index: i,
                    expected: byte_value,
                }
            })
            .collect::<Vec<_>>();

        for test in test_cases.iter() {
            push!(evm, test.input);
            push!(evm, U256::from(test.index));
            byte(&mut evm);
            pop!(evm, res);
            assert_eq!(res, test.expected, "Failed at index: {}", test.index);
        }
    }
}
