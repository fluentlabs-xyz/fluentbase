(module
  (type (;0;) (func (param i32 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64)
    local.get 0
    i64.const 0
    i64.store offset=8
    local.get 0
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 0
    local.get 2
    local.get 1
    i64.or
    local.get 3
    i64.or
    local.get 4
    i64.or
    i64.eqz
    i64.extend_i32_u
    i64.store)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_eq" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
