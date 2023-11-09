(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32 i32 i32)))
  (type (;2;) (func))
  (import "env" "_sys_read" (func $_sys_read (type 0)))
  (import "env" "_crypto_poseidon" (func $_crypto_poseidon (type 1)))
  (func $deploy (type 2))
  (func $main (type 2)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 0
    i32.store offset=7 align=1
    local.get 0
    i64.const 0
    i64.store
    local.get 0
    i32.const 0
    i32.const 11
    call $_sys_read
    drop
    local.get 0
    i32.const 40
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 32
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store offset=16
    local.get 0
    i32.const 11
    local.get 0
    i32.const 16
    i32.add
    call $_crypto_poseidon
    local.get 0
    i32.const 48
    i32.add
    global.set $__stack_pointer)
  (func $dummy (type 2))
  (func $__wasm_call_dtors (type 2)
    call $dummy
    call $dummy)
  (func $deploy.command_export (type 2)
    call $deploy
    call $__wasm_call_dtors)
  (func $main.command_export (type 2)
    call $main
    call $__wasm_call_dtors)
  (table (;0;) 1 1 funcref)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy.command_export))
  (export "main" (func $main.command_export)))
