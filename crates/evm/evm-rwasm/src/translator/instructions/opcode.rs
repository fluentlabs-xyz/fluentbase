//! EVM opcode definitions and utilities.

use super::*;
use crate::translator::{host::Host, translator::Translator};
use alloc::{boxed::Box, sync::Arc};
use core::fmt;

/// EVM opcode function signature.
pub type Instruction<H> = fn(&mut Translator<'_>, &mut H);

/// Instruction table is list of instruction function pointers mapped to
/// 256 EVM opcodes.
pub type InstructionTable<H> = [Instruction<H>; 256];

/// Arc over plain instruction table
pub type InstructionTableArc<H> = Arc<InstructionTable<H>>;

/// EVM opcode function signature.
pub type BoxedInstruction<'a, H> = Box<dyn Fn(&mut Translator<'_>, &mut H) + 'a>;

/// A table of instructions.
pub type BoxedInstructionTable<'a, H> = [BoxedInstruction<'a, H>; 256];

/// Arc over instruction table
pub type BoxedInstructionTableArc<'a, H> = Arc<BoxedInstructionTable<'a, H>>;

/// Instruction set that contains plain instruction table that contains simple `fn` function
/// pointer. and Boxed `Fn` variant that contains `Box<dyn Fn()>` function pointer that can be used
/// with closured.
///
/// Note that `Plain` variant gives us 10-20% faster Translator execution.
///
/// Boxed variant can be used to wrap plain function pointer with closure.
#[derive(Clone)]
pub enum InstructionTables<'a, H> {
    Plain(InstructionTableArc<H>),
    Boxed(BoxedInstructionTableArc<'a, H>),
}

macro_rules! opcodes {
    ($($val:literal => $name:ident => $f:expr),* $(,)?) => {
        // Constants for each opcode. This also takes care of duplicate names.
        $(
            #[doc = concat!("The `", stringify!($val), "` (\"", stringify!($name),"\") opcode.")]
            pub const $name: u8 = $val;
        )*

        /// Maps each opcode to its name.
        pub const OPCODE_JUMPMAP: [Option<&'static str>; 256] = {
            let mut map = [None; 256];
            let mut prev: u8 = 0;
            $(
                let val: u8 = $val;
                assert!(val == 0 || val > prev, "opcodes must be sorted in ascending order");
                prev = val;
                map[$val] = Some(stringify!($name));
            )*
            let _ = prev;
            map
        };

        /// Returns the instruction function for the given opcode
        pub fn instruction<H: Host>(opcode: u8) -> Instruction<H> {
            match opcode {
                $($name => $f,)*
                _ => control::not_found,
            }
        }
    };
}

/// Make instruction table.
pub fn make_instruction_table<H: Host>() -> InstructionTable<H> {
    core::array::from_fn(|i| {
        debug_assert!(i <= u8::MAX as usize);
        instruction::<H>(i as u8)
    })
}

/// Make boxed instruction table that calls `outer` closure for every instruction.
pub fn make_boxed_instruction_table<'a, H, SPEC, FN>(
    table: InstructionTable<H>,
    outer: FN,
) -> BoxedInstructionTable<'a, H>
where
    H: Host + 'a,
    FN: Fn(Instruction<H>) -> BoxedInstruction<'a, H>,
{
    core::array::from_fn(|i| outer(table[i]))
}

// When adding new opcodes:
// 1. add the opcode to the list below; make sure it's sorted by opcode value
// 2. add its gas info in the `opcode_gas_info` function below
// 3. implement the opcode in the corresponding module; the function signature must be the exact
//    same as the others
opcodes! {
    0x00 => STOP => control::stop, // done

    0x01 => ADD        => arithmetic::wrapped_add, // done
    0x02 => MUL        => arithmetic::wrapping_mul, // done
    0x03 => SUB        => arithmetic::wrapping_sub, // done
    0x04 => DIV        => arithmetic::div, // done
    0x05 => SDIV       => arithmetic::sdiv, // done
    0x06 => MOD        => arithmetic::rem, // done
    0x07 => SMOD       => arithmetic::smod, // done
    0x08 => ADDMOD     => arithmetic::addmod, // done
    0x09 => MULMOD     => arithmetic::mulmod, // done
    0x0A => EXP        => arithmetic::exp::<H>, // done
    0x0B => SIGNEXTEND => arithmetic::signextend, // done
    // 0x0C
    // 0x0D
    // 0x0E
    // 0x0F
    0x10 => LT     => bitwise::lt, // done
    0x11 => GT     => bitwise::gt, // done
    0x12 => SLT    => bitwise::slt, // done
    0x13 => SGT    => bitwise::sgt, // done
    0x14 => EQ     => bitwise::eq, // done
    0x15 => ISZERO => bitwise::iszero, // done
    0x16 => AND    => bitwise::bitand, // done
    0x17 => OR     => bitwise::bitor, // done
    0x18 => XOR    => bitwise::bitxor, // done
    0x19 => NOT    => bitwise::not, // done
    0x1A => BYTE   => bitwise::byte, // done
    0x1B => SHL    => bitwise::shl::<H>, // done
    0x1C => SHR    => bitwise::shr::<H>, // done
    0x1D => SAR    => bitwise::sar::<H>, // done
    // 0x1E
    // 0x1F
    0x20 => KECCAK256 => system::keccak256, // done
    // 0x21
    // 0x22
    // 0x23
    // 0x24
    // 0x25
    // 0x26
    // 0x27
    // 0x28
    // 0x29
    // 0x2A
    // 0x2B
    // 0x2C
    // 0x2D
    // 0x2E
    // 0x2F
    0x30 => ADDRESS   => system::address, // done
    0x31 => BALANCE   => host::balance::<H>, // done
    0x32 => ORIGIN    => host_env::origin, // tx_caller
    0x33 => CALLER    => system::caller, // done
    0x34 => CALLVALUE => system::callvalue, // done
    0x35 => CALLDATALOAD => system::calldataload, // done
    0x36 => CALLDATASIZE => system::calldatasize, // done
    0x37 => CALLDATACOPY => system::calldatacopy, // done
    0x38 => CODESIZE     => system::codesize, // done
    0x39 => CODECOPY     => system::codecopy, // TODO rewrite with low level api to reduce size

    0x3A => GASPRICE       => host_env::gasprice, // tx_gas_price
    0x3B => EXTCODESIZE    => host::extcodesize::<H>, // done
    0x3C => EXTCODECOPY    => host::extcodecopy::<H>, // done
    0x3D => RETURNDATASIZE => system::returndatasize::<H>, // TODO
    0x3E => RETURNDATACOPY => system::returndatacopy::<H>, // TODO
    0x3F => EXTCODEHASH    => host::extcodehash::<H>, // done
    0x40 => BLOCKHASH      => host::blockhash, // done
    0x41 => COINBASE       => host_env::coinbase, // done
    0x42 => TIMESTAMP      => host_env::timestamp, // done
    0x43 => NUMBER         => host_env::number, // done
    0x44 => DIFFICULTY     => host_env::difficulty::<H>, // block_difficulty
    0x45 => GASLIMIT       => host_env::gaslimit, // done
    0x46 => CHAINID        => host_env::chainid::<H>, // done
    0x47 => SELFBALANCE    => host::selfbalance::<H>, // done
    0x48 => BASEFEE        => host_env::basefee::<H>, // done
    0x49 => BLOBHASH       => host_env::blob_hash::<H>, // tx_blob_hashes (renamed from DATAHASH)
    0x4A => BLOBBASEFEE    => host_env::blob_basefee::<H>, // tx_blob_gas_price
    // 0x4B
    // 0x4C
    // 0x4D
    // 0x4E
    // 0x4F
    0x50 => POP      => stack::pop, // done
    0x51 => MLOAD    => memory::mload, // load 32 bytes from mem
    0x52 => MSTORE   => memory::mstore, // done
    0x53 => MSTORE8  => memory::mstore8, // done
    0x54 => SLOAD    => host::sload::<H>, // done
    0x55 => SSTORE   => host::sstore::<H>, // done
    0x56 => JUMP     => control::jump, // done (only static params supported)
    0x57 => JUMPI    => control::jumpi, // done (only static params supported)
    0x58 => PC       => control::pc, // just returns 0
    0x59 => MSIZE    => memory::msize, // memory.size
    0x5A => GAS      => system::gas, // return 0
    0x5B => JUMPDEST => control::jumpdest, // done
    0x5C => TLOAD    => host::tload::<H>, // done
    0x5D => TSTORE   => host::tstore::<H>, // done
    0x5E => MCOPY    => memory::mcopy::<H>, // memory.copy

    0x5F => PUSH0  => stack::push::<0, H>, // done
    0x60 => PUSH1  => stack::push::<1, H>, // done
    0x61 => PUSH2  => stack::push::<2, H>, // done
    0x62 => PUSH3  => stack::push::<3, H>, // done
    0x63 => PUSH4  => stack::push::<4, H>, // done
    0x64 => PUSH5  => stack::push::<5, H>, // done
    0x65 => PUSH6  => stack::push::<6, H>, // done
    0x66 => PUSH7  => stack::push::<7, H>, // done
    0x67 => PUSH8  => stack::push::<8, H>, // done
    0x68 => PUSH9  => stack::push::<9, H>, // done
    0x69 => PUSH10 => stack::push::<10, H>, // done
    0x6A => PUSH11 => stack::push::<11, H>, // done
    0x6B => PUSH12 => stack::push::<12, H>, // done
    0x6C => PUSH13 => stack::push::<13, H>, // done
    0x6D => PUSH14 => stack::push::<14, H>, // done
    0x6E => PUSH15 => stack::push::<15, H>, // done
    0x6F => PUSH16 => stack::push::<16, H>, // done
    0x70 => PUSH17 => stack::push::<17, H>, // done
    0x71 => PUSH18 => stack::push::<18, H>, // done
    0x72 => PUSH19 => stack::push::<19, H>, // done
    0x73 => PUSH20 => stack::push::<20, H>, // done
    0x74 => PUSH21 => stack::push::<21, H>, // done
    0x75 => PUSH22 => stack::push::<22, H>, // done
    0x76 => PUSH23 => stack::push::<23, H>, // done
    0x77 => PUSH24 => stack::push::<24, H>, // done
    0x78 => PUSH25 => stack::push::<25, H>, // done
    0x79 => PUSH26 => stack::push::<26, H>, // done
    0x7A => PUSH27 => stack::push::<27, H>, // done
    0x7B => PUSH28 => stack::push::<28, H>, // done
    0x7C => PUSH29 => stack::push::<29, H>, // done
    0x7D => PUSH30 => stack::push::<30, H>, // done
    0x7E => PUSH31 => stack::push::<31, H>, // done
    0x7F => PUSH32 => stack::push::<32, H>, // done

    0x80 => DUP1  => stack::dup::<1, H>,  // done
    0x81 => DUP2  => stack::dup::<2, H>,  // done
    0x82 => DUP3  => stack::dup::<3, H>,  // done
    0x83 => DUP4  => stack::dup::<4, H>,  // done
    0x84 => DUP5  => stack::dup::<5, H>,  // done
    0x85 => DUP6  => stack::dup::<6, H>,  // done
    0x86 => DUP7  => stack::dup::<7, H>,  // done
    0x87 => DUP8  => stack::dup::<8, H>,  // done
    0x88 => DUP9  => stack::dup::<9, H>,  // done
    0x89 => DUP10 => stack::dup::<10, H>,  // done
    0x8A => DUP11 => stack::dup::<11, H>,  // done
    0x8B => DUP12 => stack::dup::<12, H>,  // done
    0x8C => DUP13 => stack::dup::<13, H>,  // done
    0x8D => DUP14 => stack::dup::<14, H>,  // done
    0x8E => DUP15 => stack::dup::<15, H>,  // done
    0x8F => DUP16 => stack::dup::<16, H>,  // done

    0x90 => SWAP1  => stack::swap::<1, H>,  // done
    0x91 => SWAP2  => stack::swap::<2, H>,  // done
    0x92 => SWAP3  => stack::swap::<3, H>,  // done
    0x93 => SWAP4  => stack::swap::<4, H>,  // done
    0x94 => SWAP5  => stack::swap::<5, H>,  // done
    0x95 => SWAP6  => stack::swap::<6, H>,  // done
    0x96 => SWAP7  => stack::swap::<7, H>,  // done
    0x97 => SWAP8  => stack::swap::<8, H>,  // done
    0x98 => SWAP9  => stack::swap::<9, H>,  // done
    0x99 => SWAP10 => stack::swap::<10, H>,  // done
    0x9A => SWAP11 => stack::swap::<11, H>,  // done
    0x9B => SWAP12 => stack::swap::<12, H>,  // done
    0x9C => SWAP13 => stack::swap::<13, H>,  // done
    0x9D => SWAP14 => stack::swap::<14, H>,  // done
    0x9E => SWAP15 => stack::swap::<15, H>,  // done
    0x9F => SWAP16 => stack::swap::<16, H>,  // done

    0xA0 => LOG0 => host::log::<0, H>, // done
    0xA1 => LOG1 => host::log::<1, H>, // done
    0xA2 => LOG2 => host::log::<2, H>, // done
    0xA3 => LOG3 => host::log::<3, H>, // done
    0xA4 => LOG4 => host::log::<4, H>, // done
    // 0xA5
    // 0xA6
    // 0xA7
    // 0xA8
    // 0xA9
    // 0xAA
    // 0xAB
    // 0xAC
    // 0xAD
    // 0xAE
    // 0xAF
    // 0xB0
    // 0xB1
    // 0xB2
    // 0xB3
    // 0xB4
    // 0xB5
    // 0xB6
    // 0xB7
    // 0xB8
    // 0xB9
    // 0xBA
    // 0xBB
    // 0xBC
    // 0xBD
    // 0xBE
    // 0xBF
    // 0xC0
    // 0xC1
    // 0xC2
    // 0xC3
    // 0xC4
    // 0xC5
    // 0xC6
    // 0xC7
    // 0xC8
    // 0xC9
    // 0xCA
    // 0xCB
    // 0xCC
    // 0xCD
    // 0xCE
    // 0xCF
    // 0xD0
    // 0xD1
    // 0xD2
    // 0xD3
    // 0xD4
    // 0xD5
    // 0xD6
    // 0xD7
    // 0xD8
    // 0xD9
    // 0xDA
    // 0xDB
    // 0xDC
    // 0xDD
    // 0xDE
    // 0xDF
    // 0xE0
    // 0xE1
    // 0xE2
    // 0xE3
    // 0xE4
    // 0xE5
    // 0xE6
    // 0xE7
    // 0xE8
    // 0xE9
    // 0xEA
    // 0xEB
    // 0xEC
    // 0xED
    // 0xEE
    // 0xEF
    0xF0 => CREATE       => host::create::<false, H>, // done
    0xF1 => CALL         => host::call::<H>, // done (need to test)
    0xF2 => CALLCODE     => host::call_code::<H>, // not supported
    0xF3 => RETURN       => control::ret, // sdk sys_write + return
    0xF4 => DELEGATECALL => host::delegate_call::<H>, // done
    0xF5 => CREATE2      => host::create::<true, H>, // done
    // 0xF6
    // 0xF7
    // 0xF8
    // 0xF9
    0xFA => STATICCALL   => host::static_call::<H>, // done (need to test)
    // 0xFB
    // 0xFC
    0xFD => REVERT       => control::revert::<H>, // sys_write + sys_halt
    0xFE => INVALID      => control::invalid, // sys_halt
    0xFF => SELFDESTRUCT => host::selfdestruct::<H>, // not supported
}

pub fn compute_push_count(opcode: u8) -> usize {
    (opcode - PUSH0) as usize
}

/// An EVM opcode.
///
/// This is always a valid opcode, as declared in the [`opcode`][self] module or the
/// [`OPCODE_JUMPMAP`] constant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct OpCode(u8);

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.get();
        if let Some(val) = OPCODE_JUMPMAP[n as usize] {
            f.write_str(val)
        } else {
            write!(f, "UNKNOWN(0x{n:02X})")
        }
    }
}

impl OpCode {
    /// Instantiate a new opcode from a u8.
    #[inline]
    pub const fn new(opcode: u8) -> Option<Self> {
        match OPCODE_JUMPMAP[opcode as usize] {
            Some(_) => Some(Self(opcode)),
            None => None,
        }
    }

    /// Instantiate a new opcode from a u8 without checking if it is valid.
    ///
    /// # Safety
    ///
    /// All code using `Opcode` values assume that they are valid opcodes, so providing an invalid
    /// opcode may cause undefined behavior.
    #[inline]
    pub unsafe fn new_unchecked(opcode: u8) -> Self {
        Self(opcode)
    }

    /// Returns the opcode as a string.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        if let Some(str) = OPCODE_JUMPMAP[self.0 as usize] {
            str
        } else {
            "unknown"
        }
    }

    /// Returns the opcode as a u8.
    #[inline]
    pub const fn get(self) -> u8 {
        self.0
    }

    #[inline]
    #[deprecated(note = "use `new` instead")]
    #[doc(hidden)]
    pub const fn try_from_u8(opcode: u8) -> Option<Self> {
        Self::new(opcode)
    }

    #[inline]
    #[deprecated(note = "use `get` instead")]
    #[doc(hidden)]
    pub const fn u8(self) -> u8 {
        self.get()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpInfo {
    /// Data contains few information packed inside u32:
    /// IS_JUMP (1bit) | IS_GAS_BLOCK_END (1bit) | IS_PUSH (1bit) | gas (29bits)
    data: u32,
}

const JUMP_MASK: u32 = 0x80000000;
const GAS_BLOCK_END_MASK: u32 = 0x40000000;
const IS_PUSH_MASK: u32 = 0x20000000;
const GAS_MASK: u32 = 0x1FFFFFFF;

impl OpInfo {
    /// Creates a new empty [`OpInfo`].
    pub const fn none() -> Self {
        Self { data: 0 }
    }

    /// Creates a new dynamic gas [`OpInfo`].
    pub const fn dynamic_gas() -> Self {
        Self { data: 0 }
    }

    /// Creates a new gas block end [`OpInfo`].
    pub const fn gas_block_end(gas: u64) -> Self {
        Self {
            data: gas as u32 | GAS_BLOCK_END_MASK,
        }
    }

    /// Creates a new [`OpInfo`] with the given gas value.
    pub const fn gas(gas: u64) -> Self {
        Self { data: gas as u32 }
    }

    /// Creates a new jumpdest [`OpInfo`].
    pub const fn jumpdest() -> Self {
        Self {
            data: JUMP_MASK | GAS_BLOCK_END_MASK,
        }
    }

    /// Returns whether the opcode is a jump.
    #[inline]
    pub fn is_jump(self) -> bool {
        self.data & JUMP_MASK == JUMP_MASK
    }

    /// Returns whether the opcode is a gas block end.
    #[inline]
    pub fn is_gas_block_end(self) -> bool {
        self.data & GAS_BLOCK_END_MASK == GAS_BLOCK_END_MASK
    }

    /// Returns whether the opcode is a push.
    #[inline]
    pub fn is_push(self) -> bool {
        self.data & IS_PUSH_MASK == IS_PUSH_MASK
    }

    /// Returns the gas cost of the opcode.
    #[inline]
    pub fn get_gas(self) -> u32 {
        self.data & GAS_MASK
    }
}
