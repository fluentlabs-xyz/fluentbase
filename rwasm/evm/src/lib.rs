use fluentbase_rwasm::rwasm::InstructionSet;

#[derive(Default)]
struct EvmCompiler<'a> {
    evm_bytecode: &'a [u8],
    code_section: InstructionSet,
}

impl<'a> EvmCompiler<'a> {
    pub fn new(evm_bytecode: &'a [u8]) -> Self {
        Self {
            evm_bytecode,
            ..Default::default()
        }
    }

    pub fn translate(&mut self) {}
}
