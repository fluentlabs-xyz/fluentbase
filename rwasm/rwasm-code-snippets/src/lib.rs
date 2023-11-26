#![no_std]

extern crate alloc;
#[cfg(test)]
#[macro_use]
extern crate std;
extern crate wat;

mod arithmetic;
mod bitwise;
pub(crate) mod consts;
#[cfg(test)]
pub(crate) mod test_helper;

#[cfg(test)]
#[ctor::ctor]
fn log_init() {
    let init_res =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .try_init();
    if let Err(_) = init_res {
        println!("failed to init logger");
    }
}

#[cfg(feature = "fluentbase-runtime")]
mod all_tests {
    use fluentbase_rwasm::{
        rwasm::{Compiler, FuncOrExport, ReducedModule},
        Engine,
    };

    #[test]
    pub fn bitwise_byte_rwasm() {
        let wasm_binary = wat::parse_file("./bin/bitwise_byte.wat").unwrap();
        let engine = Engine::default();
        let module = fluentbase_rwasm::module::Module::new(&engine, &wasm_binary[..]).unwrap();
        println!("exports:");
        for export in module.exports().into_iter() {
            println!("export index {:?} name '{}'", export.index(), export.name());
        }
        println!("module.exports().count(): {}", module.exports().count());
        // let import_linker = Runtime::new_linker();
        let rwasm = Compiler::new(&wasm_binary, false)/*new_with_linker(&wasm_binary.to_vec(), Some(&import_linker))*/
            .unwrap()
            .finalize(Some(FuncOrExport::Func(0)), false)
            .unwrap();
        println!("rwasm {:x?}", &rwasm);
        let reduced_module = ReducedModule::new(&rwasm, false).unwrap();
        println!(
            "reduced_module.trace_binary(): |||\n{}\n|||",
            reduced_module.trace()
        );
    }
}
