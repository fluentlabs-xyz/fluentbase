//! A simple program that takes a number `n` as input, and writes the `n-1`th and `n`th fibonacci
//! number as an output.

// These two lines are necessary for the program to properly compile.
//
// Under the hood, we wrap your main function with some extra code so that it behaves properly
// inside the zkVM.
#![no_main]
sp1_zkvm::entrypoint!(main);

mod macros;
mod runtime;

extern crate alloc;

use alloc::vec::Vec;
use core::ptr;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    codec::Encoder,
    create_import_linker,
    derive::Contract,
    Bytes,
    SharedAPI,
};
use fluentbase_types::{SharedContextInputV1, STATE_MAIN};
use runtime::{runtime_register_sovereign_handlers, RuntimeContext};
use rwasm::{
    engine::RwasmConfig,
    errors::InstantiationError,
    instance,
    module::{FuncIdx, Imported},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
    AsContext,
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
use alloy_sol_types::SolType;
// use fibonacci_lib::{fibonacci, PublicValuesStruct};

pub fn main() {
    println!("cycle-tracker-report-start: block name");
    // Read an input to the program.
    //
    // Behind the scenes, this compiles down to a custom system call which handles reading inputs
    // from the prover.
    let wasm_binary = sp1_zkvm::io::read::<Vec<u8>>();

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

            return
        }
    };

    if is_run {
        let func = if let Some(func) = instance
            .get_export(&context.store, "main")
            .and_then(Extern::into_func)
        {
            func
        } else {
            return;
        };

        if let Err(err) = func.call(&mut context.store, &[], &mut []) {
            println!("Err: {}", err.to_string());
        }
        let ctx = context.store.as_context();
        let runtime_context = ctx.data();

        sp1_zkvm::io::commit_slice(&runtime_context.output());
    }

    println!("cycle-tracker-report-end: block name");
}

#[derive(Contract)]
struct RWASM<SDK> {
    sdk: SDK,
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

        let mut context_input = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: Default::default(),
        }
            .encode_to_vec(0);

        context_input.append(&mut input.to_vec());

        let ctx = RuntimeContext::new(wasm_binary)
            .with_state(STATE_MAIN)
            .with_fuel_limit(100_000_000_000)
            .with_input(context_input)
            .with_tracer();

        let mut linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ctx);

        runtime_register_sovereign_handlers(&mut linker, &mut store);

        let mut context = Context {
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
