(module
  (type (;0;) (func (param i32)))
  (import "env" "_evm_address" (func (;0;) (type 0)))
  (func (;1;) (type 0) (param i32)
    (local i32 i32 i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    i32.const 8
    i32.add
    i32.const 16
    i32.add
    local.tee 2
    i32.const 0
    i32.store
    local.get 1
    i32.const 8
    i32.add
    i32.const 8
    i32.add
    local.tee 3
    i64.const 0
    i64.store
    local.get 1
    i64.const 0
    i64.store offset=8
    local.get 1
    i32.const 8
    i32.add
    call 0
    local.get 0
    i32.const 16
    i32.add
    local.get 2
    i32.load
    i32.store align=1
    local.get 0
    i32.const 8
    i32.add
    local.get 3
    i64.load
    i64.store align=1
    local.get 0
    local.get 1
    i64.load offset=8
    i64.store align=1
    local.get 1
    i32.const 32
    i32.add
    global.set 0)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "system_address" (func 1))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
