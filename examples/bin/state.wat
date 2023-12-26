(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32)))
  (import "env" "_sys_write" (func (;0;) (type 0)))
  (func (;1;) (type 1)
    (local i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 100
    i32.store8 offset=15
    local.get 0
    i32.const 15
    i32.add
    call 2
    local.get 0
    i32.const 16
    i32.add
    global.set 0)
  (func (;2;) (type 2) (param i32)
    local.get 0
    i32.const 1
    call 0)
  (func (;3;) (type 1)
    (local i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 200
    i32.store8 offset=15
    local.get 0
    i32.const 15
    i32.add
    call 2
    local.get 0
    i32.const 16
    i32.add
    global.set 0)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "deploy" (func 1))
  (export "main" (func 3))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
