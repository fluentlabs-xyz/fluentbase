(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func))
  (import "env" "_sys_read" (func (;0;) (type 0)))
  (import "env" "_sys_write" (func (;1;) (type 0)))
  (func (;2;) (type 1)
    (local i32 i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 8
    i32.add
    local.tee 1
    i32.const 0
    i32.store16
    local.get 0
    i64.const 0
    i64.store
    local.get 0
    i32.const 0
    i32.const 10
    call 0
    local.get 0
    local.get 0
    i32.load8_u
    local.get 0
    i32.load8_u offset=1
    i32.add
    local.get 0
    i32.load8_u offset=2
    i32.add
    local.get 0
    i32.load8_u offset=3
    i32.add
    local.get 0
    i32.load8_u offset=4
    i32.add
    local.get 0
    i32.load8_u offset=5
    i32.add
    local.get 0
    i32.load8_u offset=6
    i32.add
    local.get 0
    i32.load8_u offset=7
    i32.add
    local.get 1
    i32.load8_u
    i32.add
    local.get 0
    i32.load8_u offset=9
    i32.add
    local.tee 1
    i32.const 24
    i32.shl
    local.get 1
    i32.const 65280
    i32.and
    i32.const 8
    i32.shl
    i32.or
    local.get 1
    i32.const 8
    i32.shr_u
    i32.const 65280
    i32.and
    local.get 1
    i32.const 24
    i32.shr_u
    i32.or
    i32.or
    i32.store offset=12
    local.get 0
    i32.const 12
    i32.add
    i32.const 0
    i32.const 4
    call 1
    local.get 0
    i32.const 16
    i32.add
    global.set 0)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "main" (func 2))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
