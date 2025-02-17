use rwasm::{
    engine::DropKeep,
    instruction_set,
    rwasm::{
        BinaryFormat,
        BinaryFormatError,
        BinaryFormatReader,
        InstructionSet,
        RWASM_MAGIC_BYTE_0,
        RWASM_MAGIC_BYTE_1,
        RWASM_SECTION_CODE,
        RWASM_SECTION_ELEMENT,
        RWASM_SECTION_END,
        RWASM_SECTION_FUNC,
        RWASM_SECTION_MEMORY,
        RWASM_VERSION_V1,
    },
};

pub struct RwasmModule2 {
    // module
    pub code_section: Vec<u8>,
    pub memory_section: Vec<u8>,
    pub func_section: Vec<u32>,
    pub element_section: Vec<u32>,
    // instance
    pub func_segments: Vec<u32>,
    pub entrypoint_offset: u32,
}

impl RwasmModule2 {
    pub fn new(rwasm_bytecode: &[u8]) -> Self {
        if rwasm_bytecode.is_empty() {
            return Self::empty();
        }
        Self::new_checked(rwasm_bytecode).unwrap_or_else(|_| {
            unreachable!("rwasm: invalid binary format");
        })
    }

    pub fn new_checked(rwasm_bytecode: &[u8]) -> Result<Self, BinaryFormatError> {
        // parse rwasm binary
        let mut sink = BinaryFormatReader::new(rwasm_bytecode);
        sink.assert_u8(RWASM_MAGIC_BYTE_0)?;
        sink.assert_u8(RWASM_MAGIC_BYTE_1)?;
        sink.assert_u8(RWASM_VERSION_V1)?;
        sink.assert_u8(RWASM_SECTION_CODE)?;
        let code_section_length = sink.read_u32_le()?;
        sink.assert_u8(RWASM_SECTION_MEMORY)?;
        let memory_section_length = sink.read_u32_le()?;
        sink.assert_u8(RWASM_SECTION_FUNC)?;
        let decl_section_length = sink.read_u32_le()?;
        sink.assert_u8(RWASM_SECTION_ELEMENT)?;
        let element_section_length = sink.read_u32_le()?;
        sink.assert_u8(RWASM_SECTION_END)?;
        let mut code_section = vec![0u8; code_section_length as usize];
        sink.read_bytes(&mut code_section)?;
        let mut memory_section = vec![0u8; memory_section_length as usize];
        sink.read_bytes(&mut memory_section)?;
        let mut func_section = Vec::with_capacity(decl_section_length as usize);
        let mut decl_section_sink = sink.limit_with(decl_section_length as usize);
        while !decl_section_sink.is_empty() {
            func_section.push(decl_section_sink.read_u32_le()?);
        }
        sink.pos += decl_section_length as usize;
        let mut element_section = Vec::with_capacity(element_section_length as usize);
        let mut element_section_sink = sink.limit_with(element_section_length as usize);
        while !element_section_sink.is_empty() {
            element_section.push(element_section_sink.read_u32_le()?);
        }
        sink.pos += element_section_length as usize;
        // process sections
        let mut func_segments = vec![0u32];
        let mut total_func_len = 0u32;
        for func_len in func_section.iter().take(func_section.len() - 1) {
            total_func_len += *func_len;
            func_segments.push(total_func_len);
        }
        let source_pc = func_segments
            .last()
            .copied()
            .expect("rwasm: empty function section");
        // build rwasm module
        Ok(Self {
            code_section,
            memory_section,
            func_section,
            element_section,
            entrypoint_offset: source_pc,
            func_segments,
        })
    }

    pub fn empty() -> Self {
        let instruction_set = instruction_set! {
            Return(DropKeep::none())
        };
        Self::from(instruction_set)
    }
}

impl From<InstructionSet> for RwasmModule2 {
    fn from(value: InstructionSet) -> Self {
        let mut code_section = Vec::new();
        value
            .write_binary_to_vec(&mut code_section)
            .unwrap_or_else(|_| unreachable!("rwasm: can't encode instruction set to binary"));
        let code_section_len = value.len() as u32;
        Self {
            code_section,
            memory_section: vec![],
            func_section: vec![code_section_len],
            element_section: vec![],
            func_segments: vec![0u32],
            entrypoint_offset: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::module::RwasmModule2;
    use rwasm::{
        instruction_set,
        rwasm::{BinaryFormat, InstructionSet, RwasmModule},
    };

    #[test]
    fn test_module_encoding() {
        let module = RwasmModule {
            code_section: instruction_set! {
                I32Const(100)
                I32Const(20)
                I32Add
                I32Const(3)
                I32Add
                Drop
            },
            memory_section: Default::default(),
            func_section: vec![1, 2, 3],
            element_section: vec![5, 6, 7, 8, 9],
        };
        let mut encoded_data = Vec::new();
        module.write_binary_to_vec(&mut encoded_data).unwrap();
        assert_eq!(module.encoded_length(), encoded_data.len());
        let module2 = RwasmModule2::new(&encoded_data);
        assert_eq!(
            module.code_section,
            InstructionSet::read_from_slice(module2.code_section.as_slice()).unwrap()
        );
    }
}
