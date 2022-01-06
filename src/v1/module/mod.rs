#![allow(dead_code)] // TODO: remove

mod compile;
mod error;
mod instantiate;

#[cfg(test)]
mod tests;

use self::compile::FuncBodyTranslator;
pub use self::{
    error::TranslationError,
    instantiate::{InstancePre, InstantiationError},
};
use super::{
    engine::{FuncBody, InstructionsBuilder},
    Engine,
};
use alloc::vec::Vec;
use core::mem;
use parity_wasm::elements as pwasm;
use validation::{validate_module, FuncValidator, Validator};

/// A compiled and validated WebAssembly module.
///
/// Can be used to create new [`Instance`] instantiations.
///
/// [`Instance`]: [`super::Instance`]
#[derive(Debug)]
pub struct Module {
    pub(crate) module: pwasm::Module,
    engine: Engine,
    func_bodies: Vec<FuncBody>,
}

#[derive(Debug)]
pub struct ModuleValidation<'engine> {
    engine: &'engine Engine,
    inst_builder: InstructionsBuilder,
    func_bodies: Vec<FuncBody>,
}

impl<'engine> Validator for ModuleValidation<'engine> {
    type Input = &'engine Engine;
    type Output = Vec<FuncBody>;
    type FuncValidator = FuncBodyTranslator<'engine>;

    fn new(_module: &pwasm::Module, engine: Self::Input) -> Self {
        ModuleValidation {
            engine,
            inst_builder: InstructionsBuilder::default(),
            func_bodies: Vec::new(),
        }
    }

    fn on_function_validated(
        &mut self,
        _index: u32,
        (func_body, inst_builder): <Self::FuncValidator as FuncValidator>::Output,
    ) {
        self.inst_builder = inst_builder;
        self.func_bodies.push(func_body);
    }

    fn finish(self) -> Self::Output {
        self.func_bodies
    }

    fn func_validator_input(
        &mut self,
    ) -> <Self::FuncValidator as validation::FuncValidator>::Input {
        let inst_builder = mem::take(&mut self.inst_builder);
        (self.engine, inst_builder)
    }
}

impl Module {
    /// Create a new module from the binary Wasm encoded bytes.
    pub fn new(engine: &Engine, bytes: impl AsRef<[u8]>) -> Result<Module, TranslationError> {
        let module = pwasm::deserialize_buffer(bytes.as_ref())?;
        let func_bodies = validate_module::<ModuleValidation>(&module, engine)?;
        Ok(Self {
            module,
            engine: engine.clone(),
            func_bodies,
        })
    }
}
