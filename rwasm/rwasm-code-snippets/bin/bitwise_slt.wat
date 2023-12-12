(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i64 i64 i64)
    i64.const 0
    local.set 9
    block  ;; label = @1
      local.get 5
      i64.const -9223372036854775808
      i64.and
      local.tee 10
      local.get 1
      i64.const -9223372036854775808
      i64.and
      local.tee 11
      i64.lt_u
      br_if 0 (;@1;)
      i64.const 1
      local.set 9
      local.get 10
      local.get 11
      i64.gt_u
      br_if 0 (;@1;)
      local.get 5
      i64.const 9223372036854775807
      i64.and
      local.get 1
      i64.const 9223372036854775807
      i64.and
      i64.lt_u
      local.get 6
      local.get 2
      i64.lt_u
      i32.or
      local.get 7
      local.get 3
      i64.lt_u
      i32.or
      local.get 8
      local.get 4
      i64.lt_u
      i32.or
      i64.extend_i32_u
      local.set 9
    end
    local.get 0
    i64.const 0
    i64.store offset=8
    local.get 0
    local.get 9
    i64.store
    local.get 0
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 24
    i32.add
    i64.const 0
    i64.store)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_slt" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
