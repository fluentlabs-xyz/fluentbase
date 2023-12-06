(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (import "env" "_evm_sstore" (func (;0;) (type 0)))
  (import "env" "_evm_sload" (func (;1;) (type 0)))
  (import "env" "_sys_write" (func (;2;) (type 0)))
  (func (;3;) (type 1)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 0
    global.set 0
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
    call 0
    local.get 0
    i32.const 32
    i32.add
    global.set 0)
  (func (;4;) (type 1)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 0
    global.set 0
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
    call 1
    local.get 0
    i32.const 32
    call 2
    local.get 0
    i32.const 32
    i32.add
    global.set 0)
  (memory (;0;) 17)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048622))
  (global (;2;) i32 (i32.const 1048624))
  (export "memory" (memory 0))
  (export "deploy" (func 3))
  (export "main" (func 4))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data (;0;) (i32.const 1048576) "Hello, Storage\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01"))
