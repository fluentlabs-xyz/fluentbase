(module
  (type (;0;) (func (param i64 i64 i64 i64 i64 i64 i64 i64) (result i32)))
  (func $memory_mstore8 (type 0) (param i64 i64 i64 i64 i64 i64 i64 i64) (result i32)
    local.get 0
    i32.wrap_i64)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "memory_mstore8" (func $memory_mstore8))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
