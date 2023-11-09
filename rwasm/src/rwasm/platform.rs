use crate::{common::ValueType, module::ImportName, FuncType};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

pub trait ImportHandler {
    // sys calls
    fn sys_halt(&mut self, _exit_code: u32) {}
    fn sys_write(&mut self, _offset: u32, _length: u32) {}
    fn sys_read(&mut self, _offset: u32, _length: u32) {}
    // evm calls
    fn evm_return(&mut self, _offset: u32, _length: u32) {}
}

#[derive(Default, Debug)]
pub struct DefaultImportHandler {
    input: Vec<u8>,
    exit_code: u32,
    output: Vec<u8>,
}

impl ImportHandler for DefaultImportHandler {
    fn sys_halt(&mut self, exit_code: u32) {
        self.exit_code = exit_code;
    }

    fn sys_write(&mut self, _offset: u32, _length: u32) {}
    fn sys_read(&mut self, _offset: u32, _length: u32) {}

    fn evm_return(&mut self, _offset: u32, _length: u32) {}
}

impl DefaultImportHandler {
    pub fn new(input: Vec<u8>) -> Self {
        Self {
            input,
            ..Default::default()
        }
    }

    pub fn exit_code(&self) -> u32 {
        self.exit_code
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.output
    }
}

#[derive(Debug, Clone)]
pub struct ImportFuncName(String, String);

impl Into<ImportName> for ImportFuncName {
    fn into(self) -> ImportName {
        ImportName::new(self.0.as_str(), self.1.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ImportFunc {
    import_name: ImportFuncName,
    index: u32,
    func_type: FuncType,
    fuel_amount: u32,
}

impl ImportFunc {
    pub fn new(
        import_name: ImportFuncName,
        index: u32,
        func_type: FuncType,
        fuel_amount: u32,
    ) -> Self {
        Self {
            import_name,
            index,
            func_type,
            fuel_amount,
        }
    }

    pub fn new_env<'a>(
        module_name: String,
        fn_name: String,
        index: u16,
        fuel_amount: u32,
        input: &'a [ValueType],
        output: &'a [ValueType],
    ) -> Self {
        let func_type = FuncType::new_with_refs(input, output);
        Self::new(
            ImportFuncName(module_name, fn_name),
            index as u32,
            func_type,
            fuel_amount,
        )
    }

    pub fn import_name(&self) -> ImportName {
        ImportName::new(self.import_name.0.as_str(), self.import_name.1.as_str())
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn func_type(&self) -> &FuncType {
        &self.func_type
    }
}

#[derive(Default)]
pub struct ImportLinker {
    func_by_index: BTreeMap<u32, ImportFunc>,
    func_by_name: BTreeMap<ImportName, (u32, u32)>,
}

impl ImportLinker {
    pub fn insert_function(&mut self, import_func: ImportFunc) {
        assert!(
            !self.func_by_index.contains_key(&import_func.index),
            "already persist"
        );
        assert!(
            !self.func_by_name.contains_key(&import_func.import_name()),
            "already persist"
        );
        self.func_by_index
            .insert(import_func.index, import_func.clone());
        self.func_by_name.insert(
            import_func.import_name(),
            (import_func.index, import_func.fuel_amount),
        );
    }

    pub fn resolve_by_index(&self, index: u32) -> Option<&ImportFunc> {
        self.func_by_index.get(&index)
    }

    pub fn index_mapping(&self) -> &BTreeMap<ImportName, (u32, u32)> {
        &self.func_by_name
    }
}
