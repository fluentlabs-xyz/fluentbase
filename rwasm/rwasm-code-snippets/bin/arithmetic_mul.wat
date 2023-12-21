(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (type (;2;) (func (param i32 i32)))
  (func $arithmetic_mul (type 0)
    (local i32 i32 i64 i64 i64 i32 i32 i32 i32 i32 i64 i64 i64 i64 i64 i64)
    global.get $__stack_pointer
    i32.const 224
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h3738ce35a5e73a51E
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h3738ce35a5e73a51E
    local.get 0
    i32.const 64
    i32.add
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h82bcdbec7994b9c1E
    local.get 0
    i32.const 96
    i32.add
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h82bcdbec7994b9c1E
    local.get 0
    i32.const 152
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 144
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 128
    i32.add
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store offset=128
    local.get 0
    local.get 0
    i64.load offset=88
    i64.store offset=184
    local.get 0
    local.get 0
    i64.load offset=80
    i64.store offset=176
    local.get 0
    local.get 0
    i64.load offset=72
    i64.store offset=168
    local.get 0
    local.get 0
    i64.load offset=64
    i64.store offset=160
    local.get 0
    local.get 0
    i64.load offset=120
    i64.store offset=216
    local.get 0
    local.get 0
    i64.load offset=112
    i64.store offset=208
    local.get 0
    local.get 0
    i64.load offset=104
    i64.store offset=200
    local.get 0
    local.get 0
    i64.load offset=96
    i64.store offset=192
    i32.const 0
    local.set 1
    loop  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          i32.const 4
          i32.eq
          br_if 0 (;@3;)
          local.get 0
          i32.const 192
          i32.add
          local.get 1
          i32.const 3
          i32.shl
          i32.add
          i64.load
          local.tee 2
          i64.const 32
          i64.shr_u
          local.set 3
          local.get 2
          i64.const 4294967295
          i64.and
          local.set 4
          i64.const 0
          local.set 2
          i32.const 0
          local.set 5
          loop  ;; label = @4
            local.get 5
            i32.const -1
            i32.add
            local.set 6
            loop  ;; label = @5
              local.get 6
              local.tee 5
              i32.const 3
              i32.eq
              br_if 3 (;@2;)
              local.get 1
              local.get 5
              i32.const 1
              i32.add
              local.tee 6
              i32.add
              local.tee 7
              i32.const 3
              i32.gt_u
              br_if 0 (;@5;)
            end
            local.get 5
            i32.const 2
            i32.add
            local.set 5
            local.get 0
            i32.const 128
            i32.add
            local.get 7
            i32.const 3
            i32.shl
            local.tee 8
            i32.add
            local.tee 9
            local.get 0
            i32.const 160
            i32.add
            local.get 6
            i32.const 3
            i32.shl
            i32.add
            i64.load
            local.tee 10
            i64.const 4294967295
            i64.and
            local.tee 11
            local.get 4
            i64.mul
            local.tee 12
            local.get 11
            local.get 3
            i64.mul
            local.tee 13
            local.get 10
            i64.const 32
            i64.shr_u
            local.tee 14
            local.get 4
            i64.mul
            i64.add
            local.tee 11
            i64.const 32
            i64.shl
            i64.add
            local.tee 10
            local.get 9
            i64.load
            i64.add
            local.tee 15
            i64.store
            local.get 2
            local.get 15
            local.get 10
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 2
            local.get 7
            i32.const 3
            i32.eq
            br_if 0 (;@4;)
            local.get 8
            local.get 0
            i32.const 128
            i32.add
            i32.add
            i32.const 8
            i32.add
            local.tee 6
            local.get 11
            i64.const 32
            i64.shr_u
            local.get 14
            local.get 3
            i64.mul
            i64.add
            i64.const 4294967296
            i64.const 0
            local.get 11
            local.get 13
            i64.lt_u
            select
            i64.add
            local.get 10
            local.get 12
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.tee 10
            local.get 2
            i64.add
            local.tee 2
            local.get 6
            i64.load
            i64.add
            local.tee 11
            i64.store
            local.get 11
            local.get 2
            i64.lt_u
            i64.extend_i32_u
            local.get 2
            local.get 10
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 2
            br 0 (;@4;)
          end
        end
        local.get 0
        i64.load offset=152
        local.set 2
        local.get 0
        i64.load offset=144
        local.set 4
        local.get 0
        i64.load offset=136
        local.set 3
        local.get 0
        i64.load offset=128
        local.set 10
        i32.const 0
        i32.const 0
        i64.load offset=500
        i64.const 32
        i64.shl
        i64.const 137438953472
        i64.add
        i64.const 32
        i64.shr_s
        local.tee 11
        i64.store offset=500
        i32.const 524
        local.get 11
        i32.wrap_i64
        local.tee 6
        i32.sub
        local.get 10
        i64.const 56
        i64.shl
        local.get 10
        i64.const 65280
        i64.and
        i64.const 40
        i64.shl
        i64.or
        local.get 10
        i64.const 16711680
        i64.and
        i64.const 24
        i64.shl
        local.get 10
        i64.const 4278190080
        i64.and
        i64.const 8
        i64.shl
        i64.or
        i64.or
        local.get 10
        i64.const 8
        i64.shr_u
        i64.const 4278190080
        i64.and
        local.get 10
        i64.const 24
        i64.shr_u
        i64.const 16711680
        i64.and
        i64.or
        local.get 10
        i64.const 40
        i64.shr_u
        i64.const 65280
        i64.and
        local.get 10
        i64.const 56
        i64.shr_u
        i64.or
        i64.or
        i64.or
        i64.store align=1
        i32.const 516
        local.get 6
        i32.sub
        local.get 3
        i64.const 56
        i64.shl
        local.get 3
        i64.const 65280
        i64.and
        i64.const 40
        i64.shl
        i64.or
        local.get 3
        i64.const 16711680
        i64.and
        i64.const 24
        i64.shl
        local.get 3
        i64.const 4278190080
        i64.and
        i64.const 8
        i64.shl
        i64.or
        i64.or
        local.get 3
        i64.const 8
        i64.shr_u
        i64.const 4278190080
        i64.and
        local.get 3
        i64.const 24
        i64.shr_u
        i64.const 16711680
        i64.and
        i64.or
        local.get 3
        i64.const 40
        i64.shr_u
        i64.const 65280
        i64.and
        local.get 3
        i64.const 56
        i64.shr_u
        i64.or
        i64.or
        i64.or
        i64.store align=1
        i32.const 508
        local.get 6
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
        i32.const 500
        local.get 6
        i32.sub
        local.get 2
        i64.const 56
        i64.shl
        local.get 2
        i64.const 65280
        i64.and
        i64.const 40
        i64.shl
        i64.or
        local.get 2
        i64.const 16711680
        i64.and
        i64.const 24
        i64.shl
        local.get 2
        i64.const 4278190080
        i64.and
        i64.const 8
        i64.shl
        i64.or
        i64.or
        local.get 2
        i64.const 8
        i64.shr_u
        i64.const 4278190080
        i64.and
        local.get 2
        i64.const 24
        i64.shr_u
        i64.const 16711680
        i64.and
        i64.or
        local.get 2
        i64.const 40
        i64.shr_u
        i64.const 65280
        i64.and
        local.get 2
        i64.const 56
        i64.shr_u
        i64.or
        i64.or
        i64.or
        i64.store align=1
        local.get 0
        i32.const 224
        i32.add
        global.set $__stack_pointer
        return
      end
      local.get 1
      i32.const 1
      i32.add
      local.set 1
      br 0 (;@1;)
    end)
  (func $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h3738ce35a5e73a51E (type 1) (param i32)
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
  (func $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h82bcdbec7994b9c1E (type 2) (param i32 i32)
    (local i64)
    local.get 0
    local.get 1
    i64.load align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=24
    local.get 0
    local.get 1
    i64.load offset=8 align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=16
    local.get 0
    local.get 1
    i64.load offset=16 align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=8
    local.get 0
    local.get 1
    i64.load offset=24 align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_mul" (func $arithmetic_mul))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
