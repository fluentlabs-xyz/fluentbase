(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    block  ;; label = @1
      local.get 5
      i64.const 31
      i64.gt_u
      br_if 0 (;@1;)
      local.get 7
      local.get 6
      i64.or
      local.get 8
      i64.or
      i64.eqz
      i32.eqz
      br_if 0 (;@1;)
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 5
            i64.const 8
            i64.lt_u
            br_if 0 (;@4;)
            block  ;; label = @5
              local.get 5
              i64.const 16
              i64.lt_u
              br_if 0 (;@5;)
              i64.const 128
              local.get 5
              i64.const 3
              i64.shl
              local.tee 6
              i64.shl
              local.set 7
              block  ;; label = @6
                local.get 5
                i64.const 24
                i64.lt_u
                br_if 0 (;@6;)
                block  ;; label = @7
                  local.get 7
                  local.get 4
                  i64.and
                  i64.eqz
                  i32.eqz
                  br_if 0 (;@7;)
                  i64.const -1
                  i64.const 56
                  local.get 6
                  i64.sub
                  i64.const 56
                  i64.and
                  i64.shr_u
                  local.get 4
                  i64.and
                  local.set 4
                  br 6 (;@1;)
                end
                i64.const -1
                local.get 6
                i64.const 8
                i64.add
                i64.const 56
                i64.and
                i64.shl
                local.get 4
                i64.or
                local.set 4
                br 5 (;@1;)
              end
              local.get 7
              local.get 3
              i64.and
              i64.eqz
              br_if 3 (;@2;)
              i64.const -1
              local.set 4
              i64.const -1
              local.get 6
              i64.const 8
              i64.add
              i64.const 56
              i64.and
              i64.shl
              local.get 3
              i64.or
              local.set 3
              br 4 (;@1;)
            end
            local.get 2
            local.get 5
            i64.const 3
            i64.shl
            local.tee 5
            i64.shr_u
            i64.const 128
            i64.and
            i64.eqz
            br_if 1 (;@3;)
            i64.const -1
            local.set 4
            i64.const -1
            local.get 5
            i64.const 8
            i64.add
            i64.const 56
            i64.and
            i64.shl
            local.get 2
            i64.or
            local.set 2
            i64.const -1
            local.set 3
            br 3 (;@1;)
          end
          block  ;; label = @4
            local.get 1
            local.get 5
            i64.const 3
            i64.shl
            local.tee 5
            i64.shr_u
            i64.const 128
            i64.and
            i64.eqz
            br_if 0 (;@4;)
            i64.const -1
            local.set 4
            i64.const -1
            local.get 5
            i64.const 8
            i64.add
            i64.const 56
            i64.and
            i64.shl
            local.get 1
            i64.or
            local.set 1
            i64.const -1
            local.set 3
            i64.const -1
            local.set 2
            br 3 (;@1;)
          end
          i64.const -1
          i64.const 56
          local.get 5
          i64.sub
          i64.const 56
          i64.and
          i64.shr_u
          local.get 1
          i64.and
          local.set 1
          i64.const 0
          local.set 4
          i64.const 0
          local.set 3
          i64.const 0
          local.set 2
          br 2 (;@1;)
        end
        i64.const -1
        i64.const 56
        local.get 5
        i64.sub
        i64.const 56
        i64.and
        i64.shr_u
        local.get 2
        i64.and
        local.set 2
        i64.const 0
        local.set 4
        i64.const 0
        local.set 3
        br 1 (;@1;)
      end
      i64.const -1
      i64.const 56
      local.get 6
      i64.sub
      i64.const 56
      i64.and
      i64.shr_u
      local.get 3
      i64.and
      local.set 3
      i64.const 0
      local.set 4
    end
    local.get 0
    local.get 4
    i64.store offset=24
    local.get 0
    local.get 3
    i64.store offset=16
    local.get 0
    local.get 2
    i64.store offset=8
    local.get 0
    local.get 1
    i64.store)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_signextend" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
