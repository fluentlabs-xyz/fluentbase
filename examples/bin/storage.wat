(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (import "env" "_evm_sstore" (func $_evm_sstore (type 0)))
  (import "env" "_evm_sload" (func $_evm_sload (type 0)))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (func $deploy (type 1)
    (local i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 30
    i32.add
    i32.const 0
    i32.store16
    local.get 0
    i32.const 22
    i32.add
    i64.const 0
    i64.store align=2
    local.get 0
    i64.const 0
    i64.store offset=14 align=2
    local.get 0
    i32.const 0
    i64.load offset=1048576 align=1
    i64.store
    local.get 0
    i32.const 0
    i64.load offset=1048582 align=1
    i64.store offset=6 align=2
    i32.const 1048590
    local.get 0
    call $_evm_sstore
    local.get 0
    i32.const 32
    i32.add
    global.set $__stack_pointer)
  (func $main (type 1)
    (local i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store
    i32.const 1048590
    local.get 0
    call $_evm_sload
    local.get 0
    i32.const 32
    call $_sys_write
    local.get 0
    i32.const 32
    i32.add
    global.set $__stack_pointer)
  (memory (;0;) 17)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048622))
  (global (;2;) i32 (i32.const 1048624))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data $.rodata (i32.const 1048576) "Hello, Storage\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01"))
