(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32 i32 i32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func))
  (import "env" "_sys_read" (func (;0;) (type 0)))
  (import "env" "_crypto_keccak256" (func (;1;) (type 1)))
  (import "env" "_sys_write" (func (;2;) (type 2)))
  (func (;3;) (type 3))
  (func (;4;) (type 3)
    (local i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 0
    i32.store offset=7 align=1
    local.get 0
    i64.const 0
    i64.store
    local.get 0
    i32.const 0
    i32.const 11
    call 0
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
    call 1
    local.get 0
    i32.const 16
    i32.add
    i32.const 32
    call 2
    local.get 0
    i32.const 48
    i32.add
    global.set 0)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "deploy" (func 3))
  (export "main" (func 4))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
