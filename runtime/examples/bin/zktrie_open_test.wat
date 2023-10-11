(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func (param i32 i32 i32 i32 i32)))
  (type (;2;) (func))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (import "env" "zktrie_open" (func $zktrie_open (type 1)))
  (func $main (type 2)
    i32.const 0
    i32.const 212
    call $_sys_write
    i32.const 0
    i32.const 32
    i32.const 32
    i32.const 52
    i32.const 1
    call $zktrie_open)
  (func $dummy (type 2))
  (func $__wasm_call_dtors (type 2)
    call $dummy
    call $dummy)
  (func $main.command_export (type 2)
    call $main
    call $__wasm_call_dtors)
  (table (;0;) 1 1 funcref)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (export "memory" (memory 0))
  (export "main" (func $main.command_export)))
