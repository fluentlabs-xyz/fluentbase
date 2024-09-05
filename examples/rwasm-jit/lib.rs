#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use core::ptr;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    codec::Encoder,
    create_import_linker,
    derive::Contract,
    SharedAPI,
};
use rwasm::{
    engine::RwasmConfig,
    instance,
    module::{FuncIdx, Imported},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
    Caller,
    Config,
    Engine,
    Error,
    Extern,
    Instance,
    Linker,
    Module,
    Store,
};

#[derive(Contract)]
struct RWASM<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> RWASM<SDK> {
    fn deploy(&mut self) {
        // any custom deployment logic here
    }
    fn main(&mut self) {
        let wasm_binary = self.sdk.input();

        let mut context = match Context::new() {
            Ok(c) => c,
            Err(err) => {
                let idx = match err {
                    Error::Global(_) => 1,
                    Error::Memory(_) => 2,
                    Error::Table(_) => 3,
                    Error::Linker(_) => 14,
                    Error::Instantiation(_) => 5,
                    Error::Module(_) => 6,
                    Error::Store(_) => 7,
                    Error::Func(_) => 8,
                    Error::Trap(_) => 9,
                    _ => 0,
                };
                self.sdk.write(&[0xff, idx]);
                return;
            }
        };

        let instance = match context.compile_module(wasm_binary.as_ref()) {
            Ok(instance) => instance,
            Err(err) => {
                let idx = match err {
                    Error::Global(_) => 1,
                    Error::Memory(_) => 2,
                    Error::Table(_) => 3,
                    Error::Linker(_) => 4,
                    Error::Instantiation(_) => 5,
                    Error::Module(_) => 6,
                    Error::Store(_) => 7,
                    Error::Func(_) => 8,
                    Error::Trap(_) => 9,
                    _ => 0,
                };
                self.sdk.write(&[0xff, idx]);
                return;
            }
        };

        let func = if let Some(func) = instance
            .get_export(&context.store, "main")
            .and_then(Extern::into_func)
        {
            func
        } else {
            self.sdk.write(&[4, 3, 2, 1]);
            return;
        };

        if let Err(err) = func.call(&mut context.store, &[], &mut []) {
            self.sdk.write(&[0, 1, 2, 3, 4]);
        }

        // self.sdk.write(rwasm_bytecode);
    }
}

struct Context {
    engine: Engine,
    linker: Linker<()>,
    store: Store<()>,
}

impl Context {
    fn new() -> Result<Self, Error> {
        let config = Self::make_config(true);

        let engine = Engine::new(&config);
        let mut linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());

        let mut context = Context {
            engine,
            linker,
            store,
        };

        context.register_handler("_input_size", "fluentbase_v1preview")?;
        context.register_handler("_read", "fluentbase_v1preview")?;
        context.register_handler("_exit", "fluentbase_v1preview")?;
        context.register_handler("_write", "fluentbase_v1preview")?;

        Ok(context)
    }

    fn make_config(rwasm_mode: bool) -> Config {
        let mut config = Config::default();

        config
            .wasm_mutable_global(true)
            .wasm_saturating_float_to_int(true)
            .wasm_sign_extension(true)
            .wasm_multi_value(true)
            .wasm_bulk_memory(true)
            .wasm_reference_types(true)
            .wasm_tail_call(true)
            .wasm_extended_const(true);
        if rwasm_mode {
            config.rwasm_config(RwasmConfig {
                state_router: None,
                entrypoint_name: None,
                import_linker: Some(create_import_linker()),
                wrap_import_functions: false,
            });
        }
        config
    }

    fn compile_module(&mut self, wasm: &[u8]) -> Result<Instance, Error> {
        let module = if self.engine.config().get_rwasm_config().is_some() {
            // let original_engine = Engine::new(&self.config);
            let original_engine = &self.engine;
            let original_module = Module::new(original_engine, &wasm[..])?;
            let rwasm_module = RwasmModule::from_module(&original_module);
            // encode and decode rwasm module (to tests encoding/decoding flow)
            let mut encoded_rwasm_module = alloc_slice(wasm.len());
            // let mut encoded_rwasm_module = Vec::new();
            let mut sink = BinaryFormatWriter::new(encoded_rwasm_module);
            rwasm_module.write_binary(&mut sink).unwrap();
            let rwasm_module = RwasmModule::read_from_slice(&encoded_rwasm_module).unwrap();
            // create module builder
            let mut module_builder = rwasm_module.to_module_builder(&self.engine);
            // copy imports
            for (i, imported) in original_module.imports.items.iter().enumerate() {
                match imported {
                    Imported::Func(import_name) => {
                        let func_type = original_module.funcs[i];
                        let func_type =
                            original_engine.resolve_func_type(&func_type, |v| v.clone());
                        let new_func_type = self.engine.alloc_func_type(func_type);
                        module_builder.funcs.insert(0, new_func_type);
                        module_builder.imports.funcs.push(import_name.clone());
                    }
                    Imported::Global(_) => continue,
                    _ => unreachable!("not supported import type ({:?})", imported),
                }
            }
            // copy exports indices (it's not affected, so we can safely copy)
            for (k, v) in original_module.exports.iter() {
                if let Some(func_index) = v.into_func_idx() {
                    let func_index = func_index.into_u32();
                    if func_index < original_module.imports.len_funcs as u32 {
                        unreachable!("this is imported and exported func at the same time... ?")
                    }
                    let func_type = original_module.funcs[func_index as usize];
                    let func_type = original_engine.resolve_func_type(&func_type, |v| v.clone());
                    // remap exported func type
                    let new_func_type = self.engine.alloc_func_type(func_type);
                    module_builder.funcs[func_index as usize] = new_func_type;
                }
                module_builder.push_export(k.clone(), *v);
            }
            let mut module = module_builder.finish();
            // for rWASM set entrypoint as a start function to init module and sections
            let entrypoint_func_index = module.funcs.len() - 1;
            module.start = Some(FuncIdx::from(entrypoint_func_index as u32));
            module
            // let mut module = Module::new(engine(), &wasm[..])?;
            // let entrypoint_func_index = module.funcs.len() - 1;
            // module.start = Some(FuncIdx::from(entrypoint_func_index as u32));
            // module
        } else {
            Module::new(&self.engine, &wasm[..])?
        };
        let instance_pre = self.linker.instantiate(&mut self.store, &module)?;
        instance_pre.start(&mut self.store)
    }

    fn register_handler(&mut self, name: &str, module: &str) -> Result<(), Error> {
        let func = rwasm::Func::wrap(&mut self.store, || -> Result<(), rwasm::core::Trap> {
            return Ok(());
        });

        self.linker.define(module, name, func)?;

        Ok(())
    }
}

basic_entrypoint!(RWASM);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};
    use std::str::from_utf8_unchecked;

    #[test]
    fn test_contract_works() {
        let greeting_bytecode = include_bytes!("../greeting/lib.wasm");
        let native_sdk = TestingContext::empty().with_input(greeting_bytecode);
        let sdk = JournalState::empty(native_sdk.clone());
        let mut rwasm = RWASM::new(sdk);
        rwasm.deploy();
        rwasm.main();
        let output = native_sdk.take_output();
        let module = RwasmModule::new(&output).unwrap();
        assert!(module.code_section.len() > 0);
        assert!(unsafe { from_utf8_unchecked(&module.memory_section).contains("Hello, World") })
    }
}
