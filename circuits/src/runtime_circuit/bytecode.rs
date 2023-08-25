use std::{
    cmp::Ordering,
    collections::{hash_map::DefaultHasher, BTreeMap, HashMap},
    hash::{Hash, Hasher},
};
use wasm_encoder::{
    CodeSection,
    ConstExpr,
    DataSection,
    Encode,
    Function,
    FunctionSection,
    GlobalSection,
    GlobalType,
    Instruction,
    TypeSection,
    ValType,
};

/// Helper struct that represents a single data section in wasm binary
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd)]
pub enum SectionDescriptor {
    Data {
        index: u32,
        offset: u32,
        data: Vec<u8>,
    },
}

impl SectionDescriptor {
    fn order(&self) -> usize {
        match self {
            SectionDescriptor::Data { .. } => 1usize,
        }
    }
}

impl Ord for SectionDescriptor {
    fn cmp(&self, other: &Self) -> Ordering {
        self.order().cmp(&other.order())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvmCall {
    fn_name: &'static str,
    type_index: u32,
}

/// Helper struct that represents a single element in a bytecode.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BytecodeElement {
    /// The byte value of the element.
    pub value: u8,
    /// Whether the element is an opcode or push data byte.
    pub is_code: bool,
}

///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalVariable {
    pub index: u32,
    pub init_code: Vec<u8>,
    pub is_64bit: bool,
    pub readonly: bool,
}

///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalFunction {
    pub index: u32,
    pub code: Vec<u8>,
}

impl GlobalVariable {
    pub fn default_i32(index: u32, default_value: u32) -> Self {
        Self::default(index, false, default_value as u64)
    }

    pub fn default_i64(index: u32, default_value: u64) -> Self {
        Self::default(index, true, default_value)
    }

    pub fn default(index: u32, is_64bit: bool, default_value: u64) -> Self {
        let mut init_code = Vec::new();
        if is_64bit {
            Instruction::I64Const(default_value as i64).encode(&mut init_code);
        } else {
            Instruction::I32Const(default_value as i32).encode(&mut init_code);
        }
        GlobalVariable {
            index,
            is_64bit,
            init_code,
            readonly: false,
        }
    }

    pub fn zero_i32(index: u32) -> Self {
        Self::default_i32(index, 0)
    }

    pub fn zero_i64(index: u32) -> Self {
        Self::default_i64(index, 0)
    }
}

/// EVM Bytecode
#[derive(Debug, Clone)]
pub struct Bytecode {
    /// Vector for bytecode elements.
    pub bytecode_items: Vec<BytecodeElement>,
    global_data: (u32, Vec<u8>),
    section_descriptors: Vec<SectionDescriptor>,
    variables: Vec<GlobalVariable>,
    existing_types: HashMap<u64, u32>,
    types: TypeSection,
    functions: FunctionSection,
    codes: CodeSection,
    main_locals: Vec<(u32, ValType)>,
    evm_table: HashMap<EvmCall, usize>,
    num_opcodes: usize,
    markers: HashMap<String, usize>,
}

pub trait WasmBinaryBytecode {
    fn wasm_binary(&self) -> Vec<u8>;
}

pub struct UncheckedWasmBinary(Vec<u8>);

impl UncheckedWasmBinary {
    pub fn from(data: Vec<u8>) -> Self {
        Self(data)
    }
}

impl WasmBinaryBytecode for UncheckedWasmBinary {
    fn wasm_binary(&self) -> Vec<u8> {
        self.0.clone()
    }
}

impl WasmBinaryBytecode for Bytecode {
    fn wasm_binary(&self) -> Vec<u8> {
        use wasm_encoder::{
            EntityType,
            ExportKind,
            ExportSection,
            ImportSection,
            MemorySection,
            MemoryType,
            Module,
        };
        let mut module = Module::new();
        // Encode the type & imports section.
        let mut imports = ImportSection::new();
        let ordered_evm_table = self
            .evm_table
            .clone()
            .into_iter()
            .map(|(k, v)| (v, k))
            .collect::<BTreeMap<_, _>>();
        for (_, evm_call) in ordered_evm_table {
            imports.import(
                "env",
                evm_call.fn_name,
                EntityType::Function(evm_call.type_index),
            );
        }
        // Create memory section
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
        });
        // Encode the export section.
        let mut exports = ExportSection::new();
        exports.export(
            "main",
            ExportKind::Func,
            self.evm_table.len() as u32 + self.functions.len(),
        );
        exports.export("memory", ExportKind::Memory, 0);
        // Encode the main function
        let mut functions = self.functions.clone();
        functions.function(0);
        let mut codes = self.codes.clone();
        let mut f = Function::new(self.main_locals.clone());
        f.raw(self.code());
        f.instruction(&Instruction::End);
        codes.function(&f);
        // build sections order
        // (Custom,Type,Import,Function,Table,Memory,Global,Event,Export,Start,Elem,DataCount,Code,
        // Data)
        module.section(&self.types);
        module.section(&imports);
        module.section(&functions);
        module.section(&memories);
        if self.variables.len() > 0 {
            let mut global_section = GlobalSection::new();
            for var in &self.variables {
                let var_type = if var.is_64bit {
                    GlobalType {
                        val_type: ValType::I64,
                        mutable: !var.readonly,
                    }
                } else {
                    GlobalType {
                        val_type: ValType::I32,
                        mutable: !var.readonly,
                    }
                };
                global_section.global(var_type, &ConstExpr::raw(var.init_code.clone()));
            }
            module.section(&global_section);
        }
        module.section(&exports);
        module.section(&codes);
        // if we have global data section then put it into final binary
        let mut sections = self.section_descriptors.clone();
        sections.sort();
        for section in &sections {
            match section {
                SectionDescriptor::Data {
                    index,
                    offset,
                    data,
                } => {
                    let mut data_section = DataSection::new();
                    data_section.active(
                        *index,
                        &ConstExpr::i32_const(*offset as i32),
                        data.clone(),
                    );
                    module.section(&data_section);
                } // _ => unreachable!("unknown section: {:?}", section)
            }
        }
        if self.global_data.1.len() > 0 {
            let mut data_section = DataSection::new();
            data_section.active(
                0,
                &ConstExpr::i32_const(self.global_data.0 as i32),
                self.global_data.1.clone(),
            );
            module.section(&data_section);
        }
        let wasm_bytes = module.finish();
        return wasm_bytes;
    }
}

impl Default for Bytecode {
    fn default() -> Self {
        let mut res = Self {
            bytecode_items: vec![],
            global_data: (0, vec![]),
            section_descriptors: vec![],
            variables: vec![],
            existing_types: Default::default(),
            types: Default::default(),
            functions: Default::default(),
            codes: Default::default(),
            main_locals: Default::default(),
            evm_table: Default::default(),
            num_opcodes: 0,
            markers: Default::default(),
        };
        res.ensure_function_type(vec![], vec![]);
        res
    }
}

impl Bytecode {
    /// Build not checked bytecode
    pub fn from_raw_unchecked(input: Vec<u8>) -> Self {
        let mut res = Self::default();
        res.bytecode_items = input
            .iter()
            .map(|b| BytecodeElement {
                value: *b,
                is_code: true,
            })
            .collect();
        res
    }

    pub fn alloc_default_global_data(&mut self, size: u32) -> u32 {
        self.fill_default_global_data(vec![0].repeat(size as usize))
    }

    pub fn fill_default_global_data(&mut self, data: Vec<u8>) -> u32 {
        let current_offset = self.global_data.1.len();
        self.global_data.1.extend(&data);
        current_offset as u32
    }

    pub fn with_main_locals(&mut self, locals: Vec<(u32, ValType)>) -> &mut Self {
        self.main_locals.extend(&locals);
        self
    }

    #[deprecated(note = "Use `fill_default_global_data` instead")]
    pub fn with_global_data(
        &mut self,
        memory_index: u32,
        memory_offset: u32,
        data: Vec<u8>,
    ) -> &mut Self {
        self.section_descriptors.push(SectionDescriptor::Data {
            index: memory_index,
            offset: memory_offset,
            data,
        });
        self
    }

    pub fn with_global_variable(&mut self, global_variable: GlobalVariable) {
        self.variables.push(global_variable);
    }

    fn encode_function_type(input: &Vec<ValType>, output: &Vec<ValType>) -> u64 {
        let mut buf = Vec::new();
        input.encode(&mut buf);
        output.encode(&mut buf);
        let mut hasher = DefaultHasher::new();
        buf.hash(&mut hasher);
        hasher.finish()
    }

    fn ensure_function_type(&mut self, input: Vec<ValType>, output: Vec<ValType>) -> u32 {
        let type_hash = Self::encode_function_type(&input, &output);
        if let Some(type_index) = self.existing_types.get(&type_hash) {
            return *type_index;
        }
        let type_index = self.existing_types.len() as u32;
        self.existing_types.insert(type_hash, type_index);
        self.types.function(input, output);
        type_index
    }

    pub fn new_function(
        &mut self,
        input: Vec<ValType>,
        output: Vec<ValType>,
        bytecode: Bytecode,
        locals: Vec<(u32, ValType)>,
    ) {
        let type_index = self.ensure_function_type(input, output);
        self.functions.function(type_index);
        let mut f = Function::new(locals);
        f.raw(bytecode.code());
        f.instruction(&Instruction::End);
        self.codes.function(&f);
    }

    /// Get the raw code
    pub fn raw_code(&self) -> Vec<BytecodeElement> {
        self.bytecode_items.clone()
    }

    /// Get the code
    pub fn code(&self) -> Vec<u8> {
        self.bytecode_items.iter().map(|b| b.value).collect()
    }

    /// Get the bytecode element at an index.
    pub fn get(&self, index: usize) -> Option<BytecodeElement> {
        self.bytecode_items.get(index).cloned()
    }

    /// Get the generated code
    pub fn to_vec(&self) -> Vec<u8> {
        self.wasm_binary()
    }

    pub fn write_op(&mut self, op: Instruction) -> &mut Self {
        let mut buf: Vec<u8> = vec![];
        op.encode(&mut buf);
        for (i, b) in buf.iter().enumerate() {
            if i == 0 {
                self.write_op_internal(*b);
            } else {
                self.write(*b, false);
            }
        }
        self
    }

    pub fn write_call(&mut self, index: u32) -> &mut Self {
        let mut buf: Vec<u8> = vec![];
        Instruction::Call(index).encode(&mut buf);
        for (i, b) in buf.iter().enumerate() {
            if i == 0 {
                self.write_op_internal(*b);
            } else {
                self.write(*b, false);
            }
        }
        self
    }

    fn write_op_internal(&mut self, op: u8) -> &mut Self {
        self.num_opcodes += 1;
        self.write(op, true)
    }

    /// Write byte
    pub fn write(&mut self, value: u8, is_code: bool) -> &mut Self {
        self.bytecode_items.push(BytecodeElement { value, is_code });
        self
    }

    /// Add marker
    pub fn add_marker(&mut self, marker: String) -> &mut Self {
        self.insert_marker(&marker, self.num_opcodes);
        self
    }

    /// Insert marker
    pub fn insert_marker(&mut self, marker: &str, pos: usize) {
        debug_assert!(
            !self.markers.contains_key(marker),
            "marker already used: {}",
            marker
        );
        self.markers.insert(marker.to_string(), pos);
    }

    /// Get the position of a marker
    pub fn get_pos(&self, marker: &str) -> usize {
        *self
            .markers
            .get(&marker.to_string())
            .unwrap_or_else(|| panic!("marker '{}' not found", marker))
    }
}

impl From<Vec<u8>> for Bytecode {
    fn from(input: Vec<u8>) -> Self {
        Bytecode::from_raw_unchecked(input)
    }
}

macro_rules! bytecode {
    ($($args:tt)*) => {{
        let mut code = $crate::bytecode::Bytecode::default();
        $crate::bytecode_internal!(code, $($args)*);
        code
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! bytecode_internal {
    // Nothing left to do
    ($code:ident, ) => {};
    // WASM const opcodes
    ($code:ident, $x:ident [$v:expr] $($rest:tt)*) => {{
        $code.write_postfix($crate::evm_types::OpcodeId::$x, $v as i128);
        $crate::bytecode_internal!($code, $($rest)*);
    }};
    // PUSHX opcodes
    ($code:ident, $x:ident ($v:expr) $($rest:tt)*) => {{
        debug_assert!($crate::evm_types::OpcodeId::$x.is_push_with_data(), "invalid push");
        let n = $crate::evm_types::OpcodeId::$x.postfix().expect("opcode with postfix");
        $code.push(n, $v);
        $crate::bytecode_internal!($code, $($rest)*);
    }};
    // Default opcode without any inputs
    ($code:ident, $x:ident $($rest:tt)*) => {{
        $code.write_op($crate::evm_types::OpcodeId::$x);
        $crate::bytecode_internal!($code, $($rest)*);
    }};
    // Marker
    ($code:ident, #[$marker:tt] $($rest:tt)*) => {{
        $code.add_marker(stringify!($marker).to_string());
        $crate::bytecode_internal!($code, $($rest)*);
    }};
    // Function calls
    ($code:ident, .$function:ident ($($args:expr),* $(,)?) $($rest:tt)*) => {{
        $code.$function($($args,)*);
        $crate::bytecode_internal!($code, $($rest)*);
    }};
}
