(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $bitwise_eq (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
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
    local.get 1
    local.get 5
    i64.eq
    local.get 2
    local.get 6
    i64.eq
    i32.and
    local.get 3
    local.get 7
    i64.eq
    i32.and
    local.get 4
    local.get 8
    i64.eq
    i32.and
    i64.extend_i32_u
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_eq" (func $bitwise_eq))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
