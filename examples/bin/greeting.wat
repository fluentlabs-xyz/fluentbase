(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (func $deploy (type 1))
  (func $main (type 1)
    i32.const 1048576
    i32.const 12
    call $_sys_write)
  (func $dummy (type 1))
  (func $__wasm_call_dtors (type 1)
    call $dummy
    call $dummy)
  (func $deploy.command_export (type 1)
    call $deploy
    call $__wasm_call_dtors)
  (func $main.command_export (type 1)
    call $main
    call $__wasm_call_dtors)
  (table (;0;) 1 1 funcref)
  (memory (;0;) 17)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy.command_export))
  (export "main" (func $main.command_export))
  (data $.rodata (i32.const 1048576) "Hello, World"))
