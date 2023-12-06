(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $arithmetic_add (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    local.get 0
    local.get 5
    i64.const 32
    i64.shr_u
    local.get 1
    i64.const 32
    i64.shr_u
    i64.add
    local.get 5
    i64.const 4294967295
    i64.and
    local.get 1
    i64.const 4294967295
    i64.and
    i64.add
    local.tee 1
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 5
    i64.const 32
    i64.shl
    local.get 1
    i64.const 4294967295
    i64.and
    i64.or
    i64.store
    local.get 0
    local.get 6
    i64.const 32
    i64.shr_u
    local.get 2
    i64.const 32
    i64.shr_u
    i64.add
    local.get 6
    i64.const 4294967295
    i64.and
    local.get 2
    i64.const 4294967295
    i64.and
    i64.add
    local.get 5
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 1
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 5
    i64.const 32
    i64.shl
    local.get 1
    i64.const 4294967295
    i64.and
    i64.or
    i64.store offset=8
    local.get 0
    local.get 8
    i64.const 4294967295
    i64.and
    local.get 4
    i64.const 4294967295
    i64.and
    i64.add
    local.get 8
    i64.const -4294967296
    i64.and
    local.get 4
    i64.add
    i64.const -4294967296
    i64.and
    i64.add
    local.get 7
    i64.const 32
    i64.shr_u
    local.get 3
    i64.const 32
    i64.shr_u
    i64.add
    local.get 7
    i64.const 4294967295
    i64.and
    local.get 3
    i64.const 4294967295
    i64.and
    i64.add
    local.get 5
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 1
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 5
    i64.const 32
    i64.shr_u
    i64.add
    i64.store offset=24
    local.get 0
    local.get 5
    i64.const 32
    i64.shl
    local.get 1
    i64.const 4294967295
    i64.and
    i64.or
    i64.store offset=16)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_add" (func $arithmetic_add))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
