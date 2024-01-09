(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i64 i32)
    i32.const 0
    i32.const 0
    i64.load offset=32768
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 0
    i64.store offset=32768
    i32.const 32792
    local.get 0
    i32.wrap_i64
    local.tee 1
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 32784
    local.get 1
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 32776
    local.get 1
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 32768
    local.get 1
    i32.sub
    i64.const 0
    i64.store align=1)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "system_gas" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
