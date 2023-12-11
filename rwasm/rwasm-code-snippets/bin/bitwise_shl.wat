(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i64)
    i64.const 0
    local.set 9
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 5
              i64.const 255
              i64.gt_u
              br_if 0 (;@5;)
              local.get 7
              local.get 6
              i64.or
              local.get 8
              i64.or
              i64.eqz
              br_if 1 (;@4;)
            end
            i64.const 0
            local.set 7
            br 1 (;@3;)
          end
          block  ;; label = @4
            block  ;; label = @5
              local.get 5
              i64.const 191
              i64.gt_u
              br_if 0 (;@5;)
              local.get 5
              i64.const 127
              i64.gt_u
              br_if 1 (;@4;)
              i64.const 0
              local.set 6
              i64.const 0
              local.get 5
              i64.sub
              local.set 8
              block  ;; label = @6
                local.get 5
                i64.const 63
                i64.gt_u
                br_if 0 (;@6;)
                local.get 3
                local.get 8
                i64.shr_u
                local.get 4
                local.get 5
                i64.shl
                i64.or
                local.set 7
                local.get 2
                local.get 8
                i64.shr_u
                local.get 3
                local.get 5
                i64.shl
                i64.or
                local.set 9
                local.get 1
                local.get 8
                i64.shr_u
                local.get 2
                local.get 5
                i64.shl
                i64.or
                local.set 8
                local.get 1
                local.get 5
                i64.shl
                local.set 6
                br 5 (;@1;)
              end
              local.get 3
              local.get 5
              i64.shl
              local.get 2
              local.get 8
              i64.shr_u
              i64.or
              local.set 7
              local.get 2
              local.get 5
              i64.shl
              local.get 1
              local.get 8
              i64.shr_u
              i64.or
              local.set 9
              local.get 1
              local.get 5
              i64.shl
              local.set 8
              br 4 (;@1;)
            end
            local.get 1
            local.get 5
            i64.shl
            local.set 7
            br 1 (;@3;)
          end
          i64.const 0
          local.set 8
          local.get 2
          local.get 5
          i64.shl
          local.get 1
          i64.const 0
          local.get 5
          i64.sub
          i64.shr_u
          i64.or
          local.set 7
          local.get 1
          local.get 5
          i64.shl
          local.set 9
          br 1 (;@2;)
        end
        i64.const 0
        local.set 8
      end
      i64.const 0
      local.set 6
    end
    local.get 0
    local.get 7
    i64.store offset=24
    local.get 0
    local.get 9
    i64.store offset=16
    local.get 0
    local.get 8
    i64.store offset=8
    local.get 0
    local.get 6
    i64.store)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_shl" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
