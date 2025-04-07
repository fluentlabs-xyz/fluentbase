use alloc::vec::Vec;
use bitvec::{bitvec, order::Lsb0, vec::BitVec};
use fluentbase_sdk::Bytes;

/// A map of valid `jump` destinations.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct JumpTable(pub BitVec<u8>);

impl JumpTable {
    fn new(code: &[u8]) -> Self {
        let mut jumps: BitVec<u8> = bitvec![u8, Lsb0; 0; code.len()];
        let range = code.as_ptr_range();
        let start = range.start;
        let mut iterator = start;
        let end = range.end;
        while iterator < end {
            let opcode = unsafe { *iterator };
            // JUMPDEST
            if 0x5B == opcode {
                // SAFETY: jumps are max length of the code
                unsafe { jumps.set_unchecked(iterator.offset_from(start) as usize, true) }
                iterator = unsafe { iterator.offset(1) };
            } else {
                // PUSH1
                let push_offset = opcode.wrapping_sub(0x60);
                if push_offset < 32 {
                    // SAFETY: iterator access range is checked in the while loop
                    iterator = unsafe { iterator.offset((push_offset + 2) as isize) };
                } else {
                    // SAFETY: iterator access range is checked in the while loop
                    iterator = unsafe { iterator.offset(1) };
                }
            }
        }
        JumpTable(jumps)
    }

    /// Get the raw bytes of the jump map
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_raw_slice()
    }

    /// Construct a jump map from raw bytes
    #[inline]
    pub fn from_slice(slice: &[u8]) -> Self {
        Self(BitVec::from_slice(slice))
    }

    /// Check if `pc` is a valid jump destination.
    #[inline]
    pub fn is_valid(&self, pc: usize) -> bool {
        pc < self.0.len() && self.0[pc]
    }
}

pub struct AnalyzedBytecode {
    pub bytecode: Bytes,
    pub len: usize,
    pub jump_table: JumpTable,
}

impl AnalyzedBytecode {
    #[inline]
    pub fn new(bytecode: &[u8]) -> Self {
        let original_len = bytecode.len();
        let mut padded_bytecode = Vec::with_capacity(original_len + 33);
        padded_bytecode.extend_from_slice(&bytecode);
        padded_bytecode.resize(original_len + 33, 0);
        let bytecode = Bytes::from(padded_bytecode);
        let jump_table = JumpTable::new(&bytecode[..]);
        Self {
            bytecode,
            len: original_len,
            jump_table,
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytecode[..self.len]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
}
