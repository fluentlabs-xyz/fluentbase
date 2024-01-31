use crate::primitives::Bytecode;

use super::analysis::{to_analysed, BytecodeLocked};

/// EVM contract information.
#[derive(Clone, Debug, Default)]
pub struct Contract {
    /// Bytecode contains contract code, size of original code, analysis with gas block and jump table.
    /// Note that current code is extended with push padding and STOP at end.
    pub bytecode: BytecodeLocked,
}

impl Contract {
    /// Instantiates a new contract by analyzing the given bytecode.
    #[inline]
    pub fn new(bytecode: Bytecode) -> Self {
        let bytecode = to_analysed(bytecode).try_into().expect("it is analyzed");

        Self { bytecode }
    }

    /// Returns whether the given position is a valid jump destination.
    #[inline]
    pub fn is_valid_jump(&self, pos: usize) -> bool {
        self.bytecode.jump_map().is_valid(pos)
    }
}
