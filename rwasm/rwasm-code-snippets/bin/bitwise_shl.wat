(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $bitwise_shl (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    block  ;; label = @1
      block  ;; label = @2
        local.get 2
        local.get 1
        i64.or
        local.get 3
        i64.or
        i64.const 0
        i64.ne
        br_if 0 (;@2;)
        local.get 4
        i64.const 256
        i64.lt_u
        br_if 1 (;@1;)
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
    block  ;; label = @1
      block  ;; label = @2
        local.get 4
        i64.const 191
        i64.gt_u
        br_if 0 (;@2;)
        local.get 4
        i64.const 127
        i64.gt_u
        br_if 1 (;@1;)
        i64.const 0
        local.get 4
        i64.sub
        local.set 1
        block  ;; label = @3
          local.get 4
          i64.const 63
          i64.gt_u
          br_if 0 (;@3;)
          local.get 0
          local.get 8
          local.get 4
          i64.shl
          i64.store offset=24
          local.get 0
          local.get 8
          local.get 1
          i64.shr_u
          local.get 7
          local.get 4
          i64.shl
          i64.or
          i64.store offset=16
          local.get 0
          local.get 7
          local.get 1
          i64.shr_u
          local.get 6
          local.get 4
          i64.shl
          i64.or
          i64.store offset=8
          local.get 0
          local.get 6
          local.get 1
          i64.shr_u
          local.get 5
          local.get 4
          i64.shl
          i64.or
          i64.store
          return
        end
        local.get 0
        i64.const 0
        i64.store offset=24
        local.get 0
        local.get 8
        local.get 4
        i64.shl
        i64.store offset=16
        local.get 0
        local.get 8
        local.get 1
        i64.shr_u
        local.get 7
        local.get 4
        i64.shl
        i64.or
        i64.store offset=8
        local.get 0
        local.get 7
        local.get 1
        i64.shr_u
        local.get 6
        local.get 4
        i64.shl
        i64.or
        i64.store
        return
      end
      local.get 0
      i64.const 0
      i64.store offset=8
      local.get 0
      local.get 8
      local.get 4
      i64.shl
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
      i64.store
      return
    end
    local.get 0
    i64.const 0
    i64.store offset=16
    local.get 0
    local.get 8
    local.get 4
    i64.shl
    i64.store offset=8
    local.get 0
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 0
    local.get 8
    i64.const 0
    local.get 4
    i64.sub
    i64.shr_u
    local.get 7
    local.get 4
    i64.shl
    i64.or
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_shl" (func $bitwise_shl))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))