#![allow(dead_code)]

pub mod binary_format;
mod compiler;
mod instruction_set;
mod module;
mod platform;

pub use self::{binary_format::*, compiler::*, instruction_set::*, module::*, platform::*};

#[cfg(test)]
mod tests {
    use crate::{
        common::ValueType,
        rwasm::{
            compiler::Compiler,
            module::ReducedModule,
            platform::{ImportHandler, ImportLinker},
            ImportFunc,
        },
        Config,
        Engine,
        Linker,
        Store,
    };
    use alloc::{string::ToString, vec::Vec};

    #[derive(Default, Debug, Clone)]
    struct HostState {
        return_data: Vec<u8>,
        exit_code: u32,
    }

    impl ImportHandler for HostState {
        fn sys_halt(&mut self, exit_code: u32) {
            self.exit_code = exit_code;
        }
    }

    fn execute_binary(wat: &str) -> HostState {
        let wasm_binary = wat::parse_str(wat).unwrap();
        // translate and compile module
        let mut import_linker = ImportLinker::default();
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_halt".to_string(),
            10,
            &[ValueType::I32],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_read".to_string(),
            11,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_return".to_string(),
            12,
            &[ValueType::I32; 2],
            &[],
        ));
        let mut translator = Compiler::new_with_linker(&wasm_binary, Some(&import_linker)).unwrap();
        translator.translate().unwrap();
        let binary = translator.finalize().unwrap();
        let reduced_module = ReducedModule::new(binary.as_slice()).unwrap();
        // assert_eq!(translator.code_section, reduced_module.bytecode().clone());
        let _trace = reduced_module.trace_binary();
        // execute translated rwasm
        let config = Config::default();
        let engine = Engine::new(&config);
        let mut store = Store::new(&engine, HostState::default());
        let mut linker = Linker::<HostState>::new(&engine);
        let module = reduced_module.to_module(&engine, &mut import_linker);
        import_linker
            .attach_linker(&mut linker, &mut store)
            .unwrap();
        // run start entrypoint
        linker
            .instantiate(&mut store, &module)
            .unwrap()
            .start(&mut store)
            .unwrap();
        store.data().clone()
    }

    #[test]
    fn test_execute_br_and_drop_keep() {
        execute_binary(
            r#"
    (module
      (type (;0;) (func))
      (func (;0;) (type 0)
        i32.const 7
        (block $my_block
          i32.const 100
          i32.const 20
          i32.const 3
          br $my_block
          )
        i32.const 3
        i32.add
        return
        )
      (memory (;0;) 17)
      (export "main" (func 0))
      (data (;0;) (i32.const 1048576) "Hello, World"))
        "#,
        );
    }

    #[test]
    fn test_executed_nested_function_calls() {
        execute_binary(
            r#"
    (module
      (type (;0;) (func))
      (func (;0;) (type 0)
        i32.const 100
        i32.const 20
        i32.add
        i32.const 20
        i32.add
        drop
        )
      (func (;1;) (type 0)
        call 0
        )
      (memory (;0;) 17)
      (export "main" (func 1)))
        "#,
        );
    }

    #[test]
    fn test_execute_simple_add_program() {
        execute_binary(
            r#"
    (module
      (func $main
        global.get 0
        global.get 1
        call $add
        global.get 2
        call $add
        drop
        )
      (func $add (param $lhs i32) (param $rhs i32) (result i32)
        local.get $lhs
        local.get $rhs
        i32.add
        )
      (global (;0;) i32 (i32.const 100))
      (global (;1;) i32 (i32.const 20))
      (global (;2;) i32 (i32.const 3))
      (export "main" (func $main)))
        "#,
        );
    }

    #[test]
    fn test_exit_code() {
        let host_state = execute_binary(
            r#"
    (module
      (type (;0;) (func (param i32)))
      (type (;1;) (func))
      (import "env" "_sys_halt" (func (;0;) (type 0)))
      (func (;1;) (type 1)
        i32.const 123
        call 0)
      (memory (;0;) 17)
      (export "memory" (memory 0))
      (export "main" (func 1)))
        "#,
        );
        assert_eq!(host_state.exit_code, 123);
    }
}
