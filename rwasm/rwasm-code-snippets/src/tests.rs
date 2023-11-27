#[cfg(test)]
mod tests {
    use fluentbase_rwasm::{
        rwasm::{Compiler, FuncOrExport, ReducedModule},
        Engine,
    };

    #[test]
    pub fn bitwise_byte() {
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
