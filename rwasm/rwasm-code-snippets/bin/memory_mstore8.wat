(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i64 i32 i64)
    i32.const 531
    i32.const 0
    i64.load offset=500
    local.tee 0
    i32.wrap_i64
    i32.sub
    i32.load8_u
    local.set 1
    i32.const 0
    local.get 0
    i64.const 32
    i64.shl
    local.tee 2
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 0
    i64.store offset=500
    i32.const 524
    local.get 0
    i32.wrap_i64
    i32.sub
    i64.load align=1
    local.set 0
    i32.const 0
    local.get 2
    i64.const -274877906944
    i64.add
    i64.const 32
    i64.shr_s
    i64.store offset=500
    local.get 0
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 0
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 0
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 0
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i32.wrap_i64
    local.get 1
    i32.store8)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "memory_mstore8" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
