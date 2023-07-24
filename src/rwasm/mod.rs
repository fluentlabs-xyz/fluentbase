#![allow(dead_code)]

use crate::rwasm::binary_format::BinaryFormatError;
use alloc::fmt;
use alloc::fmt::Display;

pub mod binary_format;
mod compiler;
mod instruction_set;
// mod executor;
// mod module;

#[derive(Debug)]
pub enum WazmError {
    TranslationError,
    ModuleError(crate::Error),
    MissingEntrypoint,
    NotSupportedOpcode,
    MissingFunction,
    NotSupportedImport,
    NotSupportedMemory(&'static str),
    ParseError(&'static str),
    OutOfBuffer,
    ReachedUnreachable,
    IllegalOpcode(u8),
    ImpossibleJump,
    InternalError(&'static str),
    MemoryOverflow,
    EmptyBytecode,
    BinaryFormat(BinaryFormatError),
}

impl Display for WazmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WazmError::TranslationError => write!(f, "translation error"),
            WazmError::MissingEntrypoint => write!(f, "missing entrypoint"),
            WazmError::NotSupportedOpcode => write!(f, "not supported opcode"),
            WazmError::MissingFunction => write!(f, "missing function"),
            WazmError::NotSupportedImport => write!(f, "not supported import"),
            WazmError::NotSupportedMemory(err) => write!(f, "not supported memory ({})", err),
            WazmError::ParseError(err) => write!(f, "parse error ({})", err),
            WazmError::OutOfBuffer => write!(f, "out of buffer"),
            WazmError::ReachedUnreachable => write!(f, "reached unreachable"),
            WazmError::IllegalOpcode(code) => write!(f, "illegal opcode ({})", code),
            WazmError::ImpossibleJump => write!(f, "impossible jump"),
            WazmError::InternalError(err) => write!(f, "internal error ({})", err),
            WazmError::MemoryOverflow => write!(f, "memory overflow"),
            WazmError::EmptyBytecode => write!(f, "empty bytecode"),
            _ => write!(f, "unknown error"),
        }
    }
}

pub type WazmResult<T> = Result<T, WazmError>;

pub const MAX_MEMORY_PAGES: u32 = 512;
pub const MAX_MEMORY_SIZE: u32 = 512 * 0x10000;

// #[cfg(test)]
// mod tests {
//     use alloc::fs;
//     use alloc::path::Path;
//     use alloc::vec::Vec;
//
//     fn wat2wasm(wat: &str) -> Vec<u8> {
//         wat::parse_str(wat).unwrap()
//     }
//
//     #[test]
//     fn main() {
//         let wasm_binary = wat2wasm(
//             r#"
// (module
//   (func $main
//     global.get 0
//     global.get 1
//     call $add
//     global.get 2
//     call $add
//     drop
//     )
//   (func $add (param $lhs i32) (param $rhs i32) (result i32)
//     local.get $lhs
//     local.get $rhs
//     i32.add
//     )
//   (global (;0;) i32 (i32.const 100))
//   (global (;1;) i32 (i32.const 20))
//   (global (;2;) i32 (i32.const 3))
//   (export "main" (func $main)))
//     "#,
//         );
//         let mut translator = Compiler::new(&wasm_binary).unwrap();
//         translator.translate().unwrap();
//         let binary = translator.finalize().unwrap();
//         println!("{:?}", binary);
//         let module = CompiledModule::from_vec(&binary).unwrap();
//         let trace = module.trace_binary();
//         println!("{}", trace);
//     }
//
//     fn execute_binary(wat: &str) -> WazmResult<()> {
//         let wasm_binary = wat2wasm(wat);
//         let mut translator = Compiler::new(&wasm_binary).unwrap();
//         translator.translate().unwrap();
//         let binary = translator.finalize().unwrap();
//         // println!("{:?}", binary);
//         let module = CompiledModule::from_vec(&binary).unwrap();
//         let trace = module.trace_binary();
//         println!("{}", trace);
//         execute_wazm(&module)?;
//         Ok(())
//     }
//
//     #[test]
//     fn test_execute_br_and_drop_keep() {
//         execute_binary(
//             r#"
// (module
//   (type (;0;) (func))
//   (func (;0;) (type 0)
//     i32.const 7
//     (block $my_block
//       i32.const 100
//       i32.const 20
//       i32.const 3
//       br $my_block
//       )
//     i32.const 3
//     i32.add
//     return
//     )
//   (memory (;0;) 17)
//   (export "main" (func 0))
//   (data (;0;) (i32.const 1048576) "Hello, World"))
//     "#,
//         )
//         .unwrap();
//     }
//
//     #[test]
//     fn test_executed_nested_function_calls() {
//         execute_binary(
//             r#"
// (module
//   (type (;0;) (func))
//   (func (;0;) (type 0)
//     i32.const 100
//     i32.const 20
//     i32.add
//     i32.const 20
//     i32.add
//     drop
//     )
//   (func (;1;) (type 0)
//     call 0
//     )
//   (memory (;0;) 17)
//   (export "main" (func 1)))
//     "#,
//         )
//         .unwrap();
//     }
//
//     #[test]
//     fn test_execute_simple_add_program() {
//         execute_binary(
//             r#"
// (module
//   (func $main
//     global.get 0
//     global.get 1
//     call $add
//     global.get 2
//     call $add
//     drop
//     )
//   (func $add (param $lhs i32) (param $rhs i32) (result i32)
//     local.get $lhs
//     local.get $rhs
//     i32.add
//     )
//   (global (;0;) i32 (i32.const 100))
//   (global (;1;) i32 (i32.const 20))
//   (global (;2;) i32 (i32.const 3))
//   (export "main" (func $main)))
//     "#,
//         )
//         .unwrap();
//     }
//
//     #[test]
//     fn test_host_call() {
//         let wasm_binary = wat2wasm(
//             r#"
// (module
//   (type (;0;) (func (param i32 i32)))
//   (type (;1;) (func))
//   (import "env" "_evm_return" (func (;0;) (type 0)))
//   (func (;1;) (type 1)
//     i32.const 1048576
//     i32.const 12
//     call 0)
//   (memory (;0;) 17)
//   (global (;0;) (mut i32) (i32.const 1048576))
//   (global (;1;) i32 (i32.const 1048588))
//   (global (;2;) i32 (i32.const 1048592))
//   (export "memory" (memory 0))
//   (export "main" (func 1))
//   (export "__data_end" (global 1))
//   (export "__heap_base" (global 2))
//   (data (;0;) (i32.const 1048576) "Hello, World"))
//     "#,
//         );
//
//         let mut translator = Compiler::new(&wasm_binary).unwrap();
//         translator.translate().unwrap();
//         let binary = translator.finalize().unwrap();
//         let mut module = CompiledModule::from_vec(&binary).unwrap();
//         module.linker_mut().define_function(
//             "env",
//             "_evm_return",
//             |input, output| -> WazmResult<()> {
//                 println!("123");
//                 Ok(())
//             },
//             IMPORT_EVM_RETURN,
//         );
//         execute_wazm(&module).unwrap();
//     }
//
//     #[test]
//     #[ignore]
//     fn test_self_translation() {
//         let wast_data = fs::read(Path::new("../../test/wazm_contract.wat")).unwrap();
//         let wast_data = String::from_utf8(wast_data).unwrap();
//         execute_binary(wast_data.as_str()).unwrap();
//     }
// }
