(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func))
  (import "env" "_sys_read" (func (;0;) (type 0)))
  (func (;1;) (type 1)
    (local i32 i32 i64 i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 0
    i32.store offset=12
    local.get 0
    i32.const 12
    i32.add
    i32.const 60
    i32.const 4
    call 0
    drop
    local.get 0
    i32.load offset=12
    local.set 1
    i32.const 0
    i32.const 0
    i64.load offset=32768
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 2
    i64.store offset=32768
    i32.const 32792
    local.get 2
    i32.wrap_i64
    local.tee 3
    i32.sub
    i32.const 0
    i32.store align=1
    i32.const 32784
    local.get 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 32776
    local.get 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 32768
    local.get 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 32796
    local.get 3
    i32.sub
    local.get 1
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
    i32.store align=1
    local.get 0
    i32.const 16
    i32.add
    global.set 0)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "system_codesize" (func 1))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
