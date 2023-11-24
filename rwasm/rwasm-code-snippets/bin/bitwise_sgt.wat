(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $bitwise_sgt (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i64 i64)
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        i64.const -9223372036854775808
        i64.and
        local.tee 9
        local.get 5
        i64.const -9223372036854775808
        i64.and
        local.tee 10
        i64.gt_u
        br_if 0 (;@2;)
        local.get 9
        local.get 10
        i64.lt_u
        br_if 1 (;@1;)
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            i64.const 9223372036854775807
            i64.and
            local.get 5
            i64.const 9223372036854775807
            i64.and
            i64.gt_u
            br_if 0 (;@4;)
            local.get 2
            local.get 6
            i64.gt_u
            br_if 0 (;@4;)
            local.get 3
            local.get 7
            i64.gt_u
            br_if 0 (;@4;)
            local.get 4
            local.get 8
            i64.le_u
            br_if 1 (;@3;)
          end
          local.get 0
          i64.const 0
          i64.store
          local.get 0
          i64.const 1
          i64.store offset=24
          local.get 0
          i32.const 16
          i32.add
          i64.const 0
          i64.store
          local.get 0
          i32.const 8
          i32.add
          i64.const 0
          i64.store
          return
        end
        local.get 0
        i64.const 0
        i64.store
        local.get 0
        i32.const 24
        i32.add
        i64.const 0
        i64.store
        local.get 0
        i32.const 16
        i32.add
        i64.const 0
        i64.store
        local.get 0
        i32.const 8
        i32.add
        i64.const 0
        i64.store
        return
      end
      local.get 0
      i64.const 0
      i64.store
      local.get 0
      i32.const 24
      i32.add
      i64.const 0
      i64.store
      local.get 0
      i32.const 16
      i32.add
      i64.const 0
      i64.store
      local.get 0
      i32.const 8
      i32.add
      i64.const 0
      i64.store
      return
    end
    local.get 0
    i64.const 0
    i64.store
    local.get 0
    i64.const 1
    i64.store offset=24
    local.get 0
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 8
    i32.add
    i64.const 0
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_sgt" (func $bitwise_sgt))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
