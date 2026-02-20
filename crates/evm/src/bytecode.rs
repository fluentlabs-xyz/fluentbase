//! Compact representation of analyzed EVM bytecode.
//!
//! Stores padded bytecode, original length, jump table, and code hash for
//! fast interpreter setup and inexpensive serialization.
use bincode::{
    de::read::SliceReader, decode_from_reader, enc::write::SliceWriter, encode_into_writer, error,
};
use bitvec::vec::BitVec;
use fluentbase_sdk::{Bytes, B256};
use revm_bytecode::{legacy::analyze_legacy, JumpTable};

#[derive(Debug)]
/// A legacy bytecode
pub struct LegacyBytecode {
    pub hash: B256,
    pub bytecode: Bytes,
}

#[derive(Debug)]
/// Bytecode plus metadata required by the interpreter.
pub struct AnalyzedBytecode {
    /// A padded bytecode (length might be different)
    pub bytecode: Bytes,
    /// An original bytecode len (w/o padding)
    pub len: u64,
    /// Jump table for JUMPDEST checks
    pub jump_table: JumpTable,
    /// Code hash of bytecode (non-padded version)
    pub hash: B256,
}

impl Default for AnalyzedBytecode {
    fn default() -> Self {
        Self::new(Bytes::default(), B256::ZERO)
    }
}

impl AnalyzedBytecode {
    /// Analyze legacy bytecode, compute jump table, and keep orthe iginal length and hash.
    pub fn new(bytecode: Bytes, hash: B256) -> Self {
        let len = bytecode.len() as u64;
        let (jump_table, bytecode) = analyze_legacy(bytecode);
        Self {
            bytecode,
            len,
            jump_table,
            hash,
        }
    }

    pub fn hint_size(&self) -> usize {
        8 // padded bytecode len
        + self.bytecode.len() // padded bytecode
        + 8 // original bytecode len
        + 8 // jump table len
        + self.jump_table.as_slice().len() // jump table
        + 32 // code hash
    }

    /// Serialize into a contiguous buffer suitable for metadata storage.
    pub fn serialize(&self, buffer: &mut [u8]) -> Result<(), error::EncodeError> {
        debug_assert!(buffer.len() >= self.hint_size());
        let mut writer = SliceWriter::new(buffer);
        let config = bincode::config::legacy();
        encode_into_writer(&self.hash.0, &mut writer, config)?;
        encode_into_writer(self.len, &mut writer, config)?;
        encode_into_writer(&self.bytecode[..], &mut writer, config)?;
        encode_into_writer(self.jump_table.as_slice(), &mut writer, config)?;
        Ok(())
    }

    /// Deserialize from a buffer produced by `serialize`.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, error::DecodeError> {
        use alloc::vec::Vec;
        let config = bincode::config::legacy();
        let mut reader = SliceReader::new(&bytes);
        let hash: [u8; 32] = decode_from_reader(&mut reader, config)?;
        let len: u64 = decode_from_reader(&mut reader, config)?;
        let bytecode: Vec<u8> = decode_from_reader(&mut reader, config)?;
        let jump_table: Vec<u8> = decode_from_reader(&mut reader, config)?;
        if len > bytecode.len() as u64 {
            return Err(error::DecodeError::ArrayLengthMismatch {
                required: len as usize,
                found: bytecode.len(),
            });
        }
        Ok(Self {
            bytecode: bytecode.into(),
            len,
            jump_table: JumpTable::new(BitVec::from_vec(jump_table)),
            hash: B256::from(hash),
        })
    }

    /// Return the original (unpadded) bytecode slice.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytecode[..self.len as usize]
    }

    /// Original bytecode length (before padding).
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::AnalyzedBytecode;
    use fluentbase_sdk::{hex, B256};

    #[test]
    fn test_analyzed_bytecode_encoding() {
        let bytecode = [
            0x60, 0x03, // PUSH1 3
            0x56, // JUMP
            0x5b, // JUMPDEST
        ];
        let original_bytecode = AnalyzedBytecode::new(bytecode.into(), B256::ZERO);
        let mut raw = vec![0u8; original_bytecode.hint_size()];
        original_bytecode.serialize(&mut raw).unwrap();
        let new_bytecode = AnalyzedBytecode::deserialize(&raw).unwrap();
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

    #[test]
    fn test_analyzed_bytecode() {
        let bytecode = hex!("0x608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033");
        let original_bytecode = AnalyzedBytecode::new(bytecode.into(), B256::ZERO);
        assert!(original_bytecode.jump_table.is_valid(0x039a));
        let mut raw = vec![0u8; original_bytecode.hint_size()];
        original_bytecode.serialize(&mut raw).unwrap();
        let new_bytecode = AnalyzedBytecode::deserialize(&raw).unwrap();
        assert!(new_bytecode.jump_table.is_valid(0x039a));
    }
}
