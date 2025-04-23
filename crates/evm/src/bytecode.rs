use alloc::vec::Vec;
use bincode::{de::read::SliceReader, enc::write::SliceWriter};
use bitvec::{bitvec, order::Lsb0, vec::BitVec};
use fluentbase_sdk::{alloc_slice, Bytes, B256};

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
    pub hash: B256,
}

impl Default for AnalyzedBytecode {
    fn default() -> Self {
        Self::new(&[], B256::ZERO)
    }
}

impl AnalyzedBytecode {
    pub fn new(bytecode: &[u8], hash: B256) -> Self {
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
            hash,
        }
    }

    pub fn serialize<'a>(&self) -> &'a [u8] {
        let mut buffer = alloc_slice(
            8 + self.bytecode.len()
                + size_of::<usize>()
                + 8
                + self.jump_table.as_slice().len()
                + 32
                + 8, // why? alignment?
        );
        let mut writer = SliceWriter::new(&mut buffer);
        let config = bincode::config::legacy();
        bincode::encode_into_writer(&self.bytecode[..], &mut writer, config)
            .unwrap_or_else(|_| panic!("evm: can't serialize"));
        bincode::encode_into_writer(self.len, &mut writer, config)
            .unwrap_or_else(|_| panic!("evm: can't serialize"));
        bincode::encode_into_writer(self.jump_table.as_slice(), &mut writer, config)
            .unwrap_or_else(|_| panic!("evm: can't serialize"));
        bincode::encode_into_writer(&self.hash.0, &mut writer, config)
            .unwrap_or_else(|_| panic!("evm: can't serialize"));
        buffer
    }

    pub fn deserialize(bytes: &[u8]) -> Self {
        use alloc::vec::Vec;
        let config = bincode::config::legacy();
        let mut reader = SliceReader::new(&bytes);
        let bytecode: Vec<u8> = bincode::decode_from_reader(&mut reader, config).unwrap();
        let len: usize = bincode::decode_from_reader(&mut reader, config).unwrap();
        let jump_table: Vec<u8> = bincode::decode_from_reader(&mut reader, config).unwrap();
        let hash: [u8; 32] = bincode::decode_from_reader(&mut reader, config).unwrap();
        Self {
            bytecode: bytecode.into(),
            len,
            jump_table: JumpTable::from_slice(&jump_table),
            hash: B256::from(hash),
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

#[cfg(test)]
mod tests {
    use crate::bytecode::AnalyzedBytecode;
    use fluentbase_sdk::B256;

    #[test]
    fn test_analyzed_bytecode_encoding() {
        let bytecode = [
            0x60, 0x03, // PUSH1 3
            0x56, // JUMP
            0x5b, // JUMPDEST
        ];
        let original_bytecode = AnalyzedBytecode::new(bytecode.as_ref(), B256::ZERO);
        let raw = original_bytecode.serialize();
        let new_bytecode = AnalyzedBytecode::deserialize(raw);
        assert_eq!(original_bytecode.bytecode, new_bytecode.bytecode);
        assert_eq!(original_bytecode.len, new_bytecode.len);
        assert_eq!(
            original_bytecode.jump_table.as_slice(),
            new_bytecode.jump_table.as_slice()
        );
        for (pc, _) in bytecode.iter().enumerate() {
            assert_eq!(
                original_bytecode.jump_table.is_valid(pc),
                new_bytecode.jump_table.is_valid(pc)
            );
        }
    }
}
