(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $arithmetic_sub (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i32)
    local.get 0
    local.get 4
    local.get 8
    i64.sub
    i64.store offset=24
    local.get 0
    local.get 3
    local.get 7
    local.get 4
    local.get 8
    i64.lt_u
    i64.extend_i32_u
    local.get 7
    i64.add
    local.tee 8
    local.get 8
    local.get 3
    i64.gt_u
    local.tee 9
    select
    i64.sub
    i64.store offset=16
    local.get 0
    local.get 2
    local.get 6
    local.get 9
    i64.extend_i32_u
    local.get 6
    i64.add
    local.tee 8
    local.get 8
    local.get 2
    i64.gt_u
    local.tee 9
    select
    i64.sub
    i64.store offset=8
    local.get 0
    i64.const -9223372036854775808
    local.get 1
    local.get 5
    i64.sub
    local.get 1
    i64.const -1
    i64.gt_s
    select
    local.get 1
    local.get 9
    i64.extend_i32_u
    local.get 5
    i64.add
    local.tee 8
    i64.sub
    local.get 1
    local.get 8
    i64.lt_u
    select
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_sub" (func $arithmetic_sub))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
