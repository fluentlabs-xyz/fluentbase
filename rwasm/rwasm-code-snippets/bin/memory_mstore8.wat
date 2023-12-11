(module
  (type (;0;) (func (param i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i64 i64 i64 i64 i64 i64 i64 i64)
    local.get 4
    i32.wrap_i64
    local.get 0
    i64.store8)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "memory_mstore8" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
