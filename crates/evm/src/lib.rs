use fluentbase_sdk::U256;
use rwasm::rwasm::InstructionSet;

fn evm_to_rwasm(evm_bytecode: &[u8]) -> InstructionSet {
    let mut result = InstructionSet::new();
    let mut iter = evm_bytecode.iter();
    while let Some(opcode) = iter.next() {
        match opcode {
            // PUSH1
            0x60 => {
                let value: U256 = U256::from(iter.next().copied().expect("corrupted bytecode"));
                value.into_limbs();
            }
            // ADD
            0x01 => {}
            // POP
            0x50 => {}
            _ => unreachable!("not supported opcode: {}", opcode),
        }
    }
    result
}

#[cfg(test)]
mod tests {}
