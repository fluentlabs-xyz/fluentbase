use crate::{
    binary_format::{instruction::decode_rwasm_instruction, BinaryFormatError, BinaryFormatReader},
    module::RwasmModule2,
};

/// Rwasm magic bytes 0xef52
const RWASM_MAGIC_BYTE_0: u8 = 0xef;
const RWASM_MAGIC_BYTE_1: u8 = 0x52;

/// Rwasm binary version that is equal to 'R' symbol (0x52 in hex)
const RWASM_VERSION_V1: u8 = 0x01;

/// Sections that are presented in Rwasm binary:
/// - code
/// - memory
/// - decl
/// - element
const RWASM_SECTION_CODE: u8 = 0x01;
const RWASM_SECTION_MEMORY: u8 = 0x02;
const RWASM_SECTION_FUNC: u8 = 0x03;
const RWASM_SECTION_ELEMENT: u8 = 0x04;
const RWASM_SECTION_END: u8 = 0x00;

pub fn decode_rwasm_module(
    sink: &mut BinaryFormatReader,
) -> Result<RwasmModule2, BinaryFormatError> {
    let mut result = RwasmModule2::default();
    // magic prefix (0xef 0x52)
    if sink.read_u8()? != RWASM_MAGIC_BYTE_0 || sink.read_u8()? != RWASM_MAGIC_BYTE_1 {
        return Err(BinaryFormatError::MalformedWasmModule);
    }
    // version check
    let version = sink.read_u8()?;
    if version != RWASM_VERSION_V1 {
        return Err(BinaryFormatError::MalformedWasmModule);
    }
    // code section header
    sink.assert_u8(RWASM_SECTION_CODE)?;
    let code_section_length = sink.read_u32_le()?;
    // memory section header
    sink.assert_u8(RWASM_SECTION_MEMORY)?;
    let memory_section_length = sink.read_u32_le()?;
    // decl section header
    sink.assert_u8(RWASM_SECTION_FUNC)?;
    let decl_section_length = sink.read_u32_le()?;
    // element section header
    sink.assert_u8(RWASM_SECTION_ELEMENT)?;
    let element_section_length = sink.read_u32_le()?;
    // section terminator
    sink.assert_u8(RWASM_SECTION_END)?;
    // read the code section
    let mut code_section_sink = sink.limit_with(code_section_length as usize);
    while !code_section_sink.is_empty() {
        let (instr, data) = decode_rwasm_instruction(&mut code_section_sink)?;
        result.code_section.push(instr);
        result.instr_data.push(data);
    }
    sink.pos += code_section_length as usize;
    // read the memory section
    {
        let mut memory_section_sink = sink.limit_with(memory_section_length as usize);
        result
            .memory_section
            .resize(memory_section_length as usize, 0u8);
        memory_section_sink.read_bytes(&mut result.memory_section)?;
        sink.pos += memory_section_length as usize;
    }
    // read decl section
    {
        let mut decl_section_sink = sink.limit_with(decl_section_length as usize);
        while !decl_section_sink.is_empty() {
            result.func_section.push(decl_section_sink.read_u32_le()?);
        }
        sink.pos += decl_section_length as usize;
    }
    // read element section
    {
        let mut element_section_sink = sink.limit_with(element_section_length as usize);
        while !element_section_sink.is_empty() {
            result
                .element_section
                .push(element_section_sink.read_u32_le()?);
        }
        sink.pos += element_section_length as usize;
    }
    // return the final module
    Ok(result)
}

#[cfg(test)]
mod tests {
    use rwasm::{
        instruction_set,
        rwasm::{BinaryFormat, RwasmModule},
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
        let module2 = RwasmModule::read_from_slice(&encoded_data).unwrap();
        assert_eq!(module, module2);
    }
}
