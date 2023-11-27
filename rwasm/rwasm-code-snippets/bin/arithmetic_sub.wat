(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $arithmetic_sub (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    local.get 0
    i64.const -1
    i64.store
    local.get 0
    i32.const 24
    i32.add
    i64.const -1
    i64.store
    local.get 0
    i32.const 16
    i32.add
    i64.const -1
    i64.store
    local.get 0
    i32.const 8
    i32.add
    i64.const -1
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_sub" (func $arithmetic_sub))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
