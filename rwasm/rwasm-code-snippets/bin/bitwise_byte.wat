(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $bitwise_byte (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i64)
    i64.const 0
    local.set 9
    block  ;; label = @1
      local.get 2
      local.get 1
      i64.or
      local.get 3
      i64.or
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      i64.const 0
      local.set 9
      local.get 4
      i64.const 32
      i64.ge_u
      br_if 0 (;@1;)
      block  ;; label = @2
        block  ;; label = @3
          local.get 4
          i64.const 23
          i64.gt_u
          br_if 0 (;@3;)
          local.get 4
          i64.const 15
          i64.gt_u
          br_if 1 (;@2;)
          block  ;; label = @4
            local.get 4
            i64.const 7
            i64.gt_u
            br_if 0 (;@4;)
            local.get 5
            i64.const 56
            local.get 4
            i64.const 3
            i64.shl
            i64.sub
            i64.shr_u
            i64.const 255
            i64.and
            local.set 9
            br 3 (;@1;)
          end
          local.get 6
          local.get 4
          i64.const -1
          i64.xor
          i64.const 3
          i64.shl
          i64.shr_u
          i64.const 255
          i64.and
          local.set 9
          br 2 (;@1;)
        end
        local.get 8
        i64.const 56
        local.get 4
        i64.const 3
        i64.shl
        i64.sub
        i64.shr_u
        i64.const 255
        i64.and
        local.set 9
        br 1 (;@1;)
      end
      local.get 7
      i64.const 56
      local.get 4
      i64.const 3
      i64.shl
      i64.sub
      i64.shr_u
      i64.const 255
      i64.and
      local.set 9
    end
    local.get 0
    i64.const 0
    i64.store
    local.get 0
    local.get 9
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
  (export "bitwise_byte" (func $bitwise_byte))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
