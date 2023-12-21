(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (func $bitwise_sar (type 0)
    (local i32 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i32)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h733e7d81f7ea8ec7E
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h733e7d81f7ea8ec7E
    local.get 0
    i64.load
    local.tee 1
    i64.const 56
    i64.shl
    local.get 1
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 1
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 1
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 1
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 1
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 1
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 1
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 2
    i64.const -9223372036854775808
    i64.and
    local.set 3
    i64.const 0
    local.set 1
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 0
              i64.load offset=40
              local.get 0
              i64.load offset=32
              i64.or
              local.get 0
              i64.load offset=48
              i64.or
              i64.const 0
              i64.ne
              br_if 0 (;@5;)
              local.get 0
              i64.load offset=56
              local.tee 4
              i64.const 56
              i64.shl
              local.get 4
              i64.const 65280
              i64.and
              i64.const 40
              i64.shl
              i64.or
              local.get 4
              i64.const 16711680
              i64.and
              i64.const 24
              i64.shl
              local.get 4
              i64.const 4278190080
              i64.and
              i64.const 8
              i64.shl
              i64.or
              i64.or
              local.get 4
              i64.const 8
              i64.shr_u
              i64.const 4278190080
              i64.and
              local.get 4
              i64.const 24
              i64.shr_u
              i64.const 16711680
              i64.and
              i64.or
              local.get 4
              i64.const 40
              i64.shr_u
              i64.const 65280
              i64.and
              local.get 4
              i64.const 56
              i64.shr_u
              i64.or
              i64.or
              i64.or
              local.tee 5
              i64.const 256
              i64.lt_u
              br_if 1 (;@4;)
            end
            i64.const 0
            local.set 6
            i64.const 0
            local.set 7
            i64.const 0
            local.set 4
            local.get 3
            i64.eqz
            br_if 3 (;@1;)
            i64.const -1
            local.set 1
            i64.const -1
            local.set 6
            br 1 (;@3;)
          end
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 5
                    i64.const 191
                    i64.gt_u
                    br_if 0 (;@8;)
                    local.get 0
                    i64.load offset=8
                    local.tee 1
                    i64.const 56
                    i64.shl
                    local.get 1
                    i64.const 65280
                    i64.and
                    i64.const 40
                    i64.shl
                    i64.or
                    local.get 1
                    i64.const 16711680
                    i64.and
                    i64.const 24
                    i64.shl
                    local.get 1
                    i64.const 4278190080
                    i64.and
                    i64.const 8
                    i64.shl
                    i64.or
                    i64.or
                    local.get 1
                    i64.const 8
                    i64.shr_u
                    i64.const 4278190080
                    i64.and
                    local.get 1
                    i64.const 24
                    i64.shr_u
                    i64.const 16711680
                    i64.and
                    i64.or
                    local.get 1
                    i64.const 40
                    i64.shr_u
                    i64.const 65280
                    i64.and
                    local.get 1
                    i64.const 56
                    i64.shr_u
                    i64.or
                    i64.or
                    i64.or
                    local.set 6
                    local.get 5
                    i64.const 127
                    i64.gt_u
                    br_if 1 (;@7;)
                    local.get 0
                    i64.load offset=16
                    local.tee 1
                    i64.const 56
                    i64.shl
                    local.get 1
                    i64.const 65280
                    i64.and
                    i64.const 40
                    i64.shl
                    i64.or
                    local.get 1
                    i64.const 16711680
                    i64.and
                    i64.const 24
                    i64.shl
                    local.get 1
                    i64.const 4278190080
                    i64.and
                    i64.const 8
                    i64.shl
                    i64.or
                    i64.or
                    local.get 1
                    i64.const 8
                    i64.shr_u
                    i64.const 4278190080
                    i64.and
                    local.get 1
                    i64.const 24
                    i64.shr_u
                    i64.const 16711680
                    i64.and
                    i64.or
                    local.get 1
                    i64.const 40
                    i64.shr_u
                    i64.const 65280
                    i64.and
                    local.get 1
                    i64.const 56
                    i64.shr_u
                    i64.or
                    i64.or
                    i64.or
                    local.set 7
                    i64.const 0
                    local.set 1
                    i64.const 0
                    local.get 5
                    i64.sub
                    local.set 8
                    local.get 5
                    i64.const 63
                    i64.gt_u
                    br_if 2 (;@6;)
                    local.get 0
                    i64.load offset=24
                    local.tee 1
                    i64.const 56
                    i64.shl
                    local.get 1
                    i64.const 65280
                    i64.and
                    i64.const 40
                    i64.shl
                    i64.or
                    local.get 1
                    i64.const 16711680
                    i64.and
                    i64.const 24
                    i64.shl
                    local.get 1
                    i64.const 4278190080
                    i64.and
                    i64.const 8
                    i64.shl
                    i64.or
                    i64.or
                    local.get 1
                    i64.const 8
                    i64.shr_u
                    i64.const 4278190080
                    i64.and
                    local.get 1
                    i64.const 24
                    i64.shr_u
                    i64.const 16711680
                    i64.and
                    i64.or
                    local.get 1
                    i64.const 40
                    i64.shr_u
                    i64.const 65280
                    i64.and
                    local.get 1
                    i64.const 56
                    i64.shr_u
                    i64.or
                    i64.or
                    i64.or
                    local.get 5
                    i64.shr_u
                    local.get 7
                    local.get 8
                    i64.shl
                    i64.or
                    local.set 4
                    local.get 7
                    local.get 5
                    i64.shr_u
                    local.get 6
                    local.get 8
                    i64.shl
                    i64.or
                    local.set 7
                    local.get 6
                    local.get 5
                    i64.shr_u
                    local.get 2
                    local.get 8
                    i64.shl
                    i64.or
                    local.set 6
                    local.get 2
                    local.get 5
                    i64.shr_u
                    local.set 1
                    local.get 3
                    i64.eqz
                    br_if 7 (;@1;)
                    local.get 1
                    i64.const -1
                    local.get 8
                    i64.const 63
                    i64.and
                    i64.shl
                    i64.or
                    local.set 1
                    br 7 (;@1;)
                  end
                  local.get 2
                  local.get 5
                  i64.shr_u
                  local.set 4
                  i64.const 0
                  local.set 1
                  local.get 3
                  i64.eqz
                  i32.eqz
                  br_if 3 (;@4;)
                  i64.const 0
                  local.set 6
                  i64.const 0
                  local.set 7
                  br 6 (;@1;)
                end
                i64.const 0
                local.set 1
                local.get 6
                local.get 5
                i64.shr_u
                local.get 2
                i64.const 0
                local.get 5
                i64.sub
                local.tee 6
                i64.shl
                i64.or
                local.set 4
                local.get 2
                local.get 5
                i64.shr_u
                local.set 5
                local.get 3
                i64.eqz
                i32.eqz
                br_if 1 (;@5;)
                i64.const 0
                local.set 6
                local.get 5
                local.set 7
                br 5 (;@1;)
              end
              local.get 7
              local.get 5
              i64.shr_u
              local.get 6
              local.get 8
              i64.shl
              i64.or
              local.set 9
              local.get 6
              local.get 5
              i64.shr_u
              local.get 2
              local.get 8
              i64.shl
              i64.or
              local.set 10
              local.get 2
              local.get 5
              i64.shr_u
              local.set 6
              block  ;; label = @6
                local.get 3
                i64.eqz
                i32.eqz
                br_if 0 (;@6;)
                local.get 10
                local.set 7
                local.get 9
                local.set 4
                br 5 (;@1;)
              end
              i64.const -1
              local.set 4
              local.get 6
              i64.const -1
              local.get 8
              i64.const 63
              i64.and
              i64.shl
              i64.or
              local.set 7
              local.get 9
              local.set 1
              local.get 10
              local.set 6
              br 4 (;@1;)
            end
            i64.const -1
            local.set 7
            local.get 5
            i64.const -1
            local.get 6
            i64.const 63
            i64.and
            i64.shl
            i64.or
            local.set 6
            local.get 4
            local.set 1
            br 2 (;@2;)
          end
          i64.const -1
          local.set 6
          local.get 4
          i64.const -1
          i64.const 0
          local.get 5
          i64.sub
          i64.shl
          i64.or
          local.set 1
        end
        i64.const -1
        local.set 7
      end
      i64.const -1
      local.set 4
    end
    i32.const 0
    i32.const 0
    i64.load offset=500
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 5
    i64.store offset=500
    i32.const 524
    local.get 5
    i32.wrap_i64
    local.tee 11
    i32.sub
    local.get 4
    i64.const 56
    i64.shl
    local.get 4
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 4
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 4
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 4
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 4
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 4
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 4
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 516
    local.get 11
    i32.sub
    local.get 7
    i64.const 56
    i64.shl
    local.get 7
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 7
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 7
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 7
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 7
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 7
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 7
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 508
    local.get 11
    i32.sub
    local.get 6
    i64.const 56
    i64.shl
    local.get 6
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 6
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 6
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 6
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 6
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 6
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 6
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 500
    local.get 11
    i32.sub
    local.get 1
    i64.const 56
    i64.shl
    local.get 1
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 1
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 1
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 1
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 1
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 1
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 1
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    local.get 0
    i32.const 64
    i32.add
    global.set $__stack_pointer)
  (func $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h733e7d81f7ea8ec7E (type 1) (param i32)
    (local i64 i32)
    local.get 0
    i32.const 500
    i32.const 0
    i64.load offset=500
    local.tee 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    i32.const 508
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    i32.const 516
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 24
    i32.add
    i32.const 524
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    i32.const 0
    local.get 1
    i64.const 32
    i64.shl
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    i64.store offset=500)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_sar" (func $bitwise_sar))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
