#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(unused)]

mod macros;
mod runtime;

extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    bytes::BytesMut,
    codec::{Encoder, FluentABI},
    create_import_linker,
    derive::Contract,
    Bytes,
    SharedAPI,
    SharedContextInputV1,
    STATE_MAIN,
};
use runtime::{runtime_register_sovereign_handlers, RuntimeContext};
use rwasm::{
    engine::RwasmConfig,
    errors::InstantiationError,
    module::{FuncIdx, Imported},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
    AsContext,
    Config,
    Engine,
    Error,
    Extern,
    Instance,
    Linker,
    Module,
    Store,
};

extern crate alloc;

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

        let wasm_binary = wasm_binary.0.as_ref();
        let is_run = if wasm_binary[0] == 0 { false } else { true };
        let input_size = wasm_binary[1];

        let (input, bytecode) = wasm_binary.split_at((input_size + 2).into());
        let input = input[2..].to_vec();
        let bytecode = Bytes::from(bytecode.to_vec());

        let mut context = match Context::new(bytecode.clone(), input) {
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
        let instance = match context.compile_module(bytecode.as_ref()) {
            Ok(instance) => instance,
            Err(err) => {
                if let Error::Instantiation(InstantiationError::SignatureMismatch {
                    expected,
                    actual,
                }) = err
                {
                    self.sdk.write(&[0xfe]);
                    for param in expected.params() {
                        self.sdk.write(&[*param as u8])
                    }
                    self.sdk.write(&[0xfe]);
                    for param in expected.results() {
                        self.sdk.write(&[*param as u8])
                    }
                    self.sdk.write(&[0xfe]);
                    for param in actual.params() {
                        self.sdk.write(&[*param as u8])
                    }
                    self.sdk.write(&[0xfe]);
                    for param in actual.results() {
                        self.sdk.write(&[*param as u8])
                    }
                    return;
                }

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
                self.sdk.exit(-71);
            }
        };

        if is_run {
            let func = if let Some(func) = instance
                .get_export(&context.store, "main")
                .and_then(Extern::into_func)
            {
                func
            } else {
                self.sdk.write(&[4, 3, 2, 1]);
                return;
            };

            if let Err(_err) = func.call(&mut context.store, &[], &mut []) {
                // println!("Error: {:?}", err);
            }
            let ctx = context.store.as_context();
            let runtime_context = ctx.data();
            self.sdk.write(runtime_context.output());
            self.sdk.exit(runtime_context.exit_code());
            // self.sdk.write(rwasm_bytecode);
        }
    }
}

struct Context {
    engine: Engine,
    linker: Linker<RuntimeContext>,
    store: Store<RuntimeContext>,
}

impl Context {
    fn new(wasm_binary: Bytes, input: Vec<u8>) -> Result<Self, Error> {
        let config = Self::make_config(true);
        let engine = Engine::new(&config);

        let context_input = {
            let default_ctx = SharedContextInputV1 {
                block: Default::default(),
                tx: Default::default(),
                contract: Default::default(),
            };
            let mut buf = BytesMut::new();
            FluentABI::encode(&default_ctx, &mut buf, 0).unwrap();
            buf.extend_from_slice(&input);

            buf.freeze().to_vec()
        };

        let ctx = RuntimeContext::new(wasm_binary)
            .with_state(STATE_MAIN)
            .with_fuel_limit(100_000_000_000)
            .with_input(context_input)
            .with_tracer();

        let mut linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ctx);

        runtime_register_sovereign_handlers(&mut linker, &mut store);

        let context = Context {
            engine,
            linker,
            store,
        };

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
            let encoded_rwasm_module = alloc_slice(wasm.len());
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
                        module_builder.funcs.insert(i, new_func_type);
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
        let func = rwasm::Func::wrap(&mut self.store, || -> Result<u32, rwasm::core::Trap> {
            return Ok(0);
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

    #[test]
    #[ignore]
    fn test_contract_works() {
        let greeting_bytecode = include_bytes!("../hashing/lib.wasm");
        let native_sdk = TestingContext::empty().with_input(greeting_bytecode);
        let sdk = JournalState::empty(native_sdk.clone());
        let mut rwasm = RWASM::new(sdk);
        rwasm.deploy();
        rwasm.main();
        let output = native_sdk.take_output();

        println!("Output: {:?}", output);
        println!("Output: {:?}", String::from_utf8(output))

        // let module = RwasmModule::new(&output).unwrap();
        // assert!(module.code_section.len() > 0);
        // assert!(unsafe { from_utf8_unchecked(&module.memory_section).contains("Hello, World") })
    }
}
