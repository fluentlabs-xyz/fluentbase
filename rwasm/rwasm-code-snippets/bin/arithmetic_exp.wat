(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i32 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 9
    global.set 0
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 7
            local.get 6
            i64.or
            local.get 8
            i64.or
            i64.eqz
            i32.eqz
            br_if 0 (;@4;)
            local.get 5
            i64.const 1
            i64.gt_u
            br_if 0 (;@4;)
            i64.const 0
            local.set 10
            i64.const 0
            local.set 11
            i64.const 0
            local.set 12
            block  ;; label = @5
              local.get 5
              i32.wrap_i64
              br_table 0 (;@5;) 4 (;@1;) 0 (;@5;)
            end
            local.get 3
            local.get 2
            i64.or
            local.get 1
            i64.or
            local.get 4
            i64.or
            i64.eqz
            i64.extend_i32_u
            local.set 5
            br 1 (;@3;)
          end
          local.get 3
          local.get 2
          i64.or
          local.get 4
          i64.or
          i64.eqz
          i32.eqz
          br_if 1 (;@2;)
          local.get 1
          i64.const 1
          i64.gt_u
          br_if 1 (;@2;)
          local.get 8
          local.set 10
          local.get 7
          local.set 11
          local.get 6
          local.set 12
          block  ;; label = @4
            local.get 1
            i32.wrap_i64
            br_table 0 (;@4;) 3 (;@1;) 0 (;@4;)
          end
          i64.const 1
          local.set 5
          i64.const 0
          local.set 10
        end
        i64.const 0
        local.set 11
        i64.const 0
        local.set 12
        br 1 (;@1;)
      end
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
      local.get 5
      local.set 21
      loop  ;; label = @2
        local.get 16
        local.set 5
        local.get 15
        local.set 12
        local.get 14
        local.set 11
        local.get 13
        local.set 10
        local.get 2
        local.set 22
        local.get 3
        local.set 2
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            i64.const 1
            i64.and
            i64.eqz
            i32.eqz
            br_if 0 (;@4;)
            local.get 10
            local.set 13
            local.get 11
            local.set 14
            local.get 12
            local.set 15
            local.get 5
            local.set 16
            br 1 (;@3;)
          end
          local.get 9
          local.get 20
          local.get 19
          local.get 18
          local.get 17
          local.get 21
          local.get 6
          local.get 7
          local.get 8
          call 1
          local.get 9
          i64.load offset=24
          local.set 13
          local.get 9
          i64.load offset=16
          local.set 14
          local.get 9
          i64.load offset=8
          local.set 15
          block  ;; label = @4
            local.get 9
            i64.load
            local.tee 16
            local.get 5
            i64.ne
            br_if 0 (;@4;)
            local.get 15
            local.get 12
            i64.ne
            br_if 0 (;@4;)
            local.get 14
            local.get 11
            i64.ne
            br_if 0 (;@4;)
            local.get 13
            local.set 17
            local.get 14
            local.set 18
            local.get 15
            local.set 19
            local.get 16
            local.set 20
            local.get 13
            local.get 10
            i64.eq
            br_if 3 (;@1;)
            br 1 (;@3;)
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
        local.get 4
        i64.const 63
        i64.shl
        local.get 2
        i64.const 1
        i64.shr_u
        i64.or
        local.set 3
        local.get 2
        i64.const 63
        i64.shl
        local.get 22
        i64.const 1
        i64.shr_u
        i64.or
        local.set 2
        block  ;; label = @3
          local.get 22
          i64.const 63
          i64.shl
          local.get 1
          i64.const 1
          i64.shr_u
          i64.or
          local.tee 1
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 2
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 3
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 4
          i64.const 2
          i64.ge_u
          br_if 0 (;@3;)
          local.get 17
          local.set 10
          local.get 18
          local.set 11
          local.get 19
          local.set 12
          local.get 20
          local.set 5
          br 2 (;@1;)
        end
        local.get 9
        local.get 21
        local.get 6
        local.get 7
        local.get 8
        local.get 21
        local.get 6
        local.get 7
        local.get 8
        call 1
        local.get 4
        i64.const 1
        i64.shr_u
        local.set 4
        local.get 9
        i64.load offset=24
        local.set 8
        local.get 9
        i64.load offset=16
        local.set 7
        local.get 9
        i64.load offset=8
        local.set 6
        local.get 9
        i64.load
        local.set 21
        br 0 (;@2;)
      end
    end
    local.get 0
    local.get 10
    i64.store offset=24
    local.get 0
    local.get 11
    i64.store offset=16
    local.get 0
    local.get 12
    i64.store offset=8
    local.get 0
    local.get 5
    i64.store
    local.get 9
    i32.const 32
    i32.add
    global.set 0)
  (func (;1;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i32 i32 i32 i32 i32 i32 i32 i64)
    global.get 0
    i32.const 96
    i32.sub
    local.tee 9
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 9
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 9
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 9
    i64.const 0
    i64.store
    local.get 9
    local.get 4
    i64.store offset=56
    local.get 9
    local.get 3
    i64.store offset=48
    local.get 9
    local.get 2
    i64.store offset=40
    local.get 9
    local.get 1
    i64.store offset=32
    local.get 9
    local.get 8
    i64.store offset=88
    local.get 9
    local.get 7
    i64.store offset=80
    local.get 9
    local.get 6
    i64.store offset=72
    local.get 9
    local.get 5
    i64.store offset=64
    i32.const 0
    local.set 10
    loop  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 10
          i32.const 4
          i32.eq
          br_if 0 (;@3;)
          local.get 9
          i32.const 64
          i32.add
          local.get 10
          i32.const 3
          i32.shl
          i32.add
          i64.load
          local.tee 4
          i64.const 32
          i64.shr_u
          local.set 2
          local.get 4
          i64.const 4294967295
          i64.and
          local.set 3
          i64.const 0
          local.set 4
          i32.const 0
          local.set 11
          loop  ;; label = @4
            local.get 11
            i32.const -1
            i32.add
            local.set 12
            loop  ;; label = @5
              local.get 12
              local.tee 11
              i32.const 3
              i32.eq
              br_if 3 (;@2;)
              local.get 10
              local.get 11
              i32.const 1
              i32.add
              local.tee 12
              i32.add
              local.tee 13
              i32.const 3
              i32.gt_u
              br_if 0 (;@5;)
            end
            local.get 11
            i32.const 2
            i32.add
            local.set 11
            local.get 9
            local.get 13
            i32.const 3
            i32.shl
            local.tee 14
            i32.add
            local.tee 15
            local.get 9
            i32.const 32
            i32.add
            local.get 12
            i32.const 3
            i32.shl
            i32.add
            i64.load
            local.tee 1
            i64.const 4294967295
            i64.and
            local.tee 8
            local.get 3
            i64.mul
            local.tee 6
            local.get 8
            local.get 2
            i64.mul
            local.tee 5
            local.get 1
            i64.const 32
            i64.shr_u
            local.tee 16
            local.get 3
            i64.mul
            i64.add
            local.tee 8
            i64.const 32
            i64.shl
            i64.add
            local.tee 1
            local.get 15
            i64.load
            i64.add
            local.tee 7
            i64.store
            local.get 4
            local.get 7
            local.get 1
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 4
            local.get 13
            i32.const 3
            i32.eq
            br_if 0 (;@4;)
            local.get 14
            local.get 9
            i32.add
            i32.const 8
            i32.add
            local.tee 12
            local.get 8
            i64.const 32
            i64.shr_u
            local.get 16
            local.get 2
            i64.mul
            i64.add
            i64.const 4294967296
            i64.const 0
            local.get 8
            local.get 5
            i64.lt_u
            select
            i64.add
            local.get 1
            local.get 6
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.tee 1
            local.get 4
            i64.add
            local.tee 4
            local.get 12
            i64.load
            i64.add
            local.tee 8
            i64.store
            local.get 8
            local.get 4
            i64.lt_u
            i64.extend_i32_u
            local.get 4
            local.get 1
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 4
            br 0 (;@4;)
          end
        end
        local.get 0
        local.get 9
        i64.load offset=24
        i64.store offset=24
        local.get 0
        local.get 9
        i64.load offset=16
        i64.store offset=16
        local.get 0
        local.get 9
        i64.load offset=8
        i64.store offset=8
        local.get 0
        local.get 9
        i64.load
        i64.store
        return
      end
      local.get 10
      i32.const 1
      i32.add
      local.set 10
      br 0 (;@1;)
    end)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_exp" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
