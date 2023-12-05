(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $arithmetic_sub (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i64 i64)
    block  ;; label = @1
      block  ;; label = @2
        local.get 6
        local.get 5
        local.get 1
        i64.lt_u
        i64.extend_i32_u
        local.tee 9
        local.get 2
        i64.add
        local.tee 10
        i64.ge_u
        br_if 0 (;@2;)
        i64.const 1
        local.set 10
        local.get 2
        i64.const -1
        i64.xor
        local.get 6
        i64.add
        local.get 9
        i64.const 1
        i64.xor
        i64.add
        local.set 2
        br 1 (;@1;)
      end
      local.get 6
      local.get 10
      i64.sub
      local.set 2
      i64.const 0
      local.set 10
    end
    block  ;; label = @1
      block  ;; label = @2
        local.get 7
        local.get 10
        local.get 3
        i64.add
        local.tee 6
        i64.ge_u
        br_if 0 (;@2;)
        i64.const 1
        local.set 6
        local.get 3
        i64.const -1
        i64.xor
        local.get 7
        i64.add
        local.get 10
        i64.const 1
        i64.xor
        i64.add
        local.set 7
        br 1 (;@1;)
      end
      local.get 7
      local.get 6
      i64.sub
      local.set 7
      i64.const 0
      local.set 6
    end
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 8
          local.get 6
          local.get 4
          i64.add
          local.tee 3
          i64.ge_u
          br_if 0 (;@3;)
          local.get 8
          i64.const -1
          i64.gt_s
          br_if 1 (;@2;)
          i64.const -9223372036854775808
          local.set 8
          br 2 (;@1;)
        end
        local.get 8
        local.get 3
        i64.sub
        local.set 8
        br 1 (;@1;)
      end
      local.get 4
      i64.const -1
      i64.xor
      local.get 8
      i64.add
      local.get 6
      i64.const 1
      i64.xor
      i64.add
      local.set 8
    end
    local.get 0
    local.get 8
    i64.store offset=24
    local.get 0
    local.get 7
    i64.store offset=16
    local.get 0
    local.get 2
    i64.store offset=8
    local.get 0
    local.get 5
    local.get 1
    i64.sub
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_sub" (func $arithmetic_sub))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
