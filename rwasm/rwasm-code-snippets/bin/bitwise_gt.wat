(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func $bitwise_gt (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                local.get 1
                local.get 5
                i64.gt_u
                br_if 0 (;@6;)
                local.get 1
                local.get 5
                i64.lt_u
                br_if 1 (;@5;)
                local.get 2
                local.get 6
                i64.gt_u
                br_if 2 (;@4;)
                local.get 2
                local.get 6
                i64.lt_u
                br_if 3 (;@3;)
                local.get 3
                local.get 7
                i64.gt_u
                br_if 4 (;@2;)
                local.get 3
                local.get 7
                i64.lt_u
                br_if 5 (;@1;)
                block  ;; label = @7
                  local.get 4
                  local.get 8
                  i64.gt_u
                  br_if 0 (;@7;)
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
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_gt" (func $bitwise_gt))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
