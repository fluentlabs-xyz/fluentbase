(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func))
  (import "env" "_sys_read" (func (;0;) (type 0)))
  (func (;1;) (type 1)
    (local i32 i32 i32 i32 i64 i64 i64 i64 i64)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 24
    i32.add
    local.tee 1
    i64.const 0
    i64.store
    local.get 0
    i32.const 16
    i32.add
    local.tee 2
    i64.const 0
    i64.store
    local.get 0
    i32.const 8
    i32.add
    local.tee 3
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store
    local.get 0
    i32.const 140
    i32.const 32
    call 0
    drop
    local.get 3
    i64.load
    local.set 4
    local.get 2
    i64.load
    local.set 5
    local.get 1
    i64.load
    local.set 6
    local.get 0
    i64.load
    local.set 7
    i32.const 0
    i32.const 0
    i64.load offset=32768
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 8
    i64.store offset=32768
    i32.const 32792
    local.get 8
    i32.wrap_i64
    local.tee 1
    i32.sub
    local.get 6
    i64.store align=1
    i32.const 32784
    local.get 1
    i32.sub
    local.get 5
    i64.store align=1
    i32.const 32776
    local.get 1
    i32.sub
    local.get 4
    i64.store align=1
    i32.const 32768
    local.get 1
    i32.sub
    local.get 7
    i64.store align=1
    local.get 0
    i32.const 32
    i32.add
    global.set 0)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "host_blockhash" (func 1))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
