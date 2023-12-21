(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func (param i32 i32 i32)))
  (func $arithmetic_exp (type 0)
    (local i32 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i32)
    global.get $__stack_pointer
    i32.const 224
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h7733c55a32ed7abcE
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h7733c55a32ed7abcE
    local.get 0
    i32.const 64
    i32.add
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h64e5c1ec4cb4d1c4E
    local.get 0
    i32.const 96
    i32.add
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h64e5c1ec4cb4d1c4E
    local.get 0
    i64.load offset=104
    local.set 1
    local.get 0
    i64.load offset=112
    local.set 2
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 0
          i64.load offset=120
          local.tee 3
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 2
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 1
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          i64.const 1
          local.set 4
          local.get 0
          i64.load offset=96
          local.tee 5
          i64.const 1
          i64.gt_u
          br_if 0 (;@3;)
          i64.const 0
          local.set 6
          local.get 5
          i64.const 0
          i64.ne
          br_if 1 (;@2;)
          local.get 0
          i64.load offset=88
          local.get 0
          i64.load offset=80
          i64.or
          local.get 0
          i64.load offset=72
          i64.or
          local.get 0
          i64.load offset=64
          i64.or
          i64.eqz
          i64.extend_i32_u
          local.set 4
          br 1 (;@2;)
        end
        local.get 0
        i64.load offset=64
        local.set 7
        local.get 0
        i64.load offset=72
        local.set 8
        local.get 0
        i64.load offset=80
        local.set 9
        block  ;; label = @3
          local.get 0
          i64.load offset=88
          local.tee 5
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 9
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 8
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          i64.const 1
          local.set 4
          local.get 7
          i64.const 1
          i64.gt_u
          br_if 0 (;@3;)
          i64.const 0
          local.set 6
          i64.const 0
          local.set 10
          i64.const 0
          local.set 11
          local.get 7
          i64.const 1
          i64.ne
          br_if 2 (;@1;)
          local.get 0
          i64.load offset=96
          local.set 4
          local.get 3
          local.set 6
          local.get 2
          local.set 10
          local.get 1
          local.set 11
          br 2 (;@1;)
        end
        local.get 0
        i64.load offset=96
        local.set 12
        i64.const 0
        local.set 13
        i64.const 0
        local.set 14
        i64.const 0
        local.set 15
        i64.const 1
        local.set 16
        i64.const 0
        local.set 17
        i64.const 0
        local.set 18
        i64.const 0
        local.set 19
        i64.const 1
        local.set 20
        loop  ;; label = @3
          local.get 16
          local.set 4
          local.get 15
          local.set 11
          local.get 14
          local.set 10
          local.get 13
          local.set 6
          local.get 8
          local.set 21
          local.get 9
          local.set 8
          block  ;; label = @4
            block  ;; label = @5
              local.get 7
              i64.const 1
              i64.and
              i64.eqz
              i32.eqz
              br_if 0 (;@5;)
              local.get 6
              local.set 13
              local.get 10
              local.set 14
              local.get 11
              local.set 15
              local.get 4
              local.set 16
              br 1 (;@4;)
            end
            local.get 0
            local.get 17
            i64.store offset=184
            local.get 0
            local.get 18
            i64.store offset=176
            local.get 0
            local.get 19
            i64.store offset=168
            local.get 0
            local.get 20
            i64.store offset=160
            local.get 0
            local.get 3
            i64.store offset=216
            local.get 0
            local.get 2
            i64.store offset=208
            local.get 0
            local.get 1
            i64.store offset=200
            local.get 0
            local.get 12
            i64.store offset=192
            local.get 0
            i32.const 128
            i32.add
            local.get 0
            i32.const 160
            i32.add
            local.get 0
            i32.const 192
            i32.add
            call $_ZN30fluentbase_rwasm_code_snippets6common3mul17h10e5e5cbcc8da7f4E
            local.get 0
            i64.load offset=136
            local.set 15
            local.get 0
            i64.load offset=144
            local.set 14
            local.get 0
            i64.load offset=152
            local.set 13
            block  ;; label = @5
              local.get 0
              i64.load offset=128
              local.tee 16
              local.get 4
              i64.ne
              br_if 0 (;@5;)
              local.get 15
              local.get 11
              i64.ne
              br_if 0 (;@5;)
              local.get 14
              local.get 10
              i64.ne
              br_if 0 (;@5;)
              local.get 13
              local.set 17
              local.get 14
              local.set 18
              local.get 15
              local.set 19
              local.get 16
              local.set 20
              local.get 13
              local.get 6
              i64.eq
              br_if 4 (;@1;)
              br 1 (;@4;)
            end
            local.get 13
            local.set 17
            local.get 14
            local.set 18
            local.get 15
            local.set 19
            local.get 16
            local.set 20
          end
          local.get 5
          i64.const 63
          i64.shl
          local.get 8
          i64.const 1
          i64.shr_u
          i64.or
          local.set 9
          local.get 8
          i64.const 63
          i64.shl
          local.get 21
          i64.const 1
          i64.shr_u
          i64.or
          local.set 8
          block  ;; label = @4
            local.get 21
            i64.const 63
            i64.shl
            local.get 7
            i64.const 1
            i64.shr_u
            i64.or
            local.tee 7
            i64.const 0
            i64.ne
            br_if 0 (;@4;)
            local.get 8
            i64.const 0
            i64.ne
            br_if 0 (;@4;)
            local.get 9
            i64.const 0
            i64.ne
            br_if 0 (;@4;)
            local.get 5
            i64.const 2
            i64.ge_u
            br_if 0 (;@4;)
            local.get 17
            local.set 6
            local.get 18
            local.set 10
            local.get 19
            local.set 11
            local.get 20
            local.set 4
            br 3 (;@1;)
          end
          local.get 0
          local.get 3
          i64.store offset=184
          local.get 0
          local.get 2
          i64.store offset=176
          local.get 0
          local.get 1
          i64.store offset=168
          local.get 0
          local.get 12
          i64.store offset=160
          local.get 0
          local.get 3
          i64.store offset=216
          local.get 0
          local.get 2
          i64.store offset=208
          local.get 0
          local.get 1
          i64.store offset=200
          local.get 0
          local.get 12
          i64.store offset=192
          local.get 5
          i64.const 1
          i64.shr_u
          local.set 5
          local.get 0
          i32.const 128
          i32.add
          local.get 0
          i32.const 160
          i32.add
          local.get 0
          i32.const 192
          i32.add
          call $_ZN30fluentbase_rwasm_code_snippets6common3mul17h10e5e5cbcc8da7f4E
          local.get 0
          i64.load offset=128
          local.set 12
          local.get 0
          i64.load offset=136
          local.set 1
          local.get 0
          i64.load offset=144
          local.set 2
          local.get 0
          i64.load offset=152
          local.set 3
          br 0 (;@3;)
        end
      end
      i64.const 0
      local.set 10
      i64.const 0
      local.set 11
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
    local.tee 2
    i64.store offset=500
    i32.const 524
    local.get 2
    i32.wrap_i64
    local.tee 22
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
    local.get 22
    i32.sub
    local.get 11
    i64.const 56
    i64.shl
    local.get 11
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 11
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 11
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 11
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 11
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 11
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 11
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 508
    local.get 22
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
    i32.const 500
    local.get 22
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
    local.get 0
    i32.const 224
    i32.add
    global.set $__stack_pointer)
  (func $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h7733c55a32ed7abcE (type 1) (param i32)
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
  (func $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h64e5c1ec4cb4d1c4E (type 2) (param i32 i32)
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
  (func $_ZN30fluentbase_rwasm_code_snippets6common3mul17h10e5e5cbcc8da7f4E (type 3) (param i32 i32 i32)
    (local i32 i32 i64 i64 i64 i32 i32 i32 i64 i64 i64 i64 i64 i64)
    global.get $__stack_pointer
    i32.const 96
    i32.sub
    local.tee 3
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 3
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 3
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 3
    i64.const 0
    i64.store
    local.get 3
    local.get 1
    i64.load offset=24
    i64.store offset=56
    local.get 3
    local.get 1
    i64.load offset=16
    i64.store offset=48
    local.get 3
    local.get 1
    i64.load offset=8
    i64.store offset=40
    local.get 3
    local.get 1
    i64.load
    i64.store offset=32
    local.get 3
    local.get 2
    i64.load offset=24
    i64.store offset=88
    local.get 3
    local.get 2
    i64.load offset=16
    i64.store offset=80
    local.get 3
    local.get 2
    i64.load offset=8
    i64.store offset=72
    local.get 3
    local.get 2
    i64.load
    i64.store offset=64
    i32.const 0
    local.set 4
    loop  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 4
          i32.const 4
          i32.eq
          br_if 0 (;@3;)
          local.get 3
          i32.const 64
          i32.add
          local.get 4
          i32.const 3
          i32.shl
          i32.add
          i64.load
          local.tee 5
          i64.const 32
          i64.shr_u
          local.set 6
          local.get 5
          i64.const 4294967295
          i64.and
          local.set 7
          i64.const 0
          local.set 5
          i32.const 0
          local.set 2
          loop  ;; label = @4
            local.get 2
            i32.const -1
            i32.add
            local.set 1
            loop  ;; label = @5
              local.get 1
              local.tee 2
              i32.const 3
              i32.eq
              br_if 3 (;@2;)
              local.get 4
              local.get 2
              i32.const 1
              i32.add
              local.tee 1
              i32.add
              local.tee 8
              i32.const 3
              i32.gt_u
              br_if 0 (;@5;)
            end
            local.get 2
            i32.const 2
            i32.add
            local.set 2
            local.get 3
            local.get 8
            i32.const 3
            i32.shl
            local.tee 9
            i32.add
            local.tee 10
            local.get 3
            i32.const 32
            i32.add
            local.get 1
            i32.const 3
            i32.shl
            i32.add
            i64.load
            local.tee 11
            i64.const 4294967295
            i64.and
            local.tee 12
            local.get 7
            i64.mul
            local.tee 13
            local.get 12
            local.get 6
            i64.mul
            local.tee 14
            local.get 11
            i64.const 32
            i64.shr_u
            local.tee 15
            local.get 7
            i64.mul
            i64.add
            local.tee 12
            i64.const 32
            i64.shl
            i64.add
            local.tee 11
            local.get 10
            i64.load
            i64.add
            local.tee 16
            i64.store
            local.get 5
            local.get 16
            local.get 11
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 5
            local.get 8
            i32.const 3
            i32.eq
            br_if 0 (;@4;)
            local.get 9
            local.get 3
            i32.add
            i32.const 8
            i32.add
            local.tee 1
            local.get 12
            i64.const 32
            i64.shr_u
            local.get 15
            local.get 6
            i64.mul
            i64.add
            i64.const 4294967296
            i64.const 0
            local.get 12
            local.get 14
            i64.lt_u
            select
            i64.add
            local.get 11
            local.get 13
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.tee 11
            local.get 5
            i64.add
            local.tee 5
            local.get 1
            i64.load
            i64.add
            local.tee 12
            i64.store
            local.get 12
            local.get 5
            i64.lt_u
            i64.extend_i32_u
            local.get 5
            local.get 11
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 5
            br 0 (;@4;)
          end
        end
        local.get 0
        local.get 3
        i64.load offset=24
        i64.store offset=24
        local.get 0
        local.get 3
        i64.load offset=16
        i64.store offset=16
        local.get 0
        local.get 3
        i64.load offset=8
        i64.store offset=8
        local.get 0
        local.get 3
        i64.load
        i64.store
        return
      end
      local.get 4
      i32.const 1
      i32.add
      local.set 4
      br 0 (;@1;)
    end)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_exp" (func $arithmetic_exp))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
