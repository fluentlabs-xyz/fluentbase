(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
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
    local.get 8
    i64.store offset=56
    local.get 9
    local.get 7
    i64.store offset=48
    local.get 9
    local.get 6
    i64.store offset=40
    local.get 9
    local.get 5
    i64.store offset=32
    local.get 9
    local.get 4
    i64.store offset=88
    local.get 9
    local.get 3
    i64.store offset=80
    local.get 9
    local.get 2
    i64.store offset=72
    local.get 9
    local.get 1
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
          local.tee 8
          i64.const 32
          i64.shr_u
          local.set 6
          local.get 8
          i64.const 4294967295
          i64.and
          local.set 7
          i64.const 0
          local.set 8
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
            local.tee 5
            i64.const 4294967295
            i64.and
            local.tee 4
            local.get 7
            i64.mul
            local.tee 2
            local.get 4
            local.get 6
            i64.mul
            local.tee 1
            local.get 5
            i64.const 32
            i64.shr_u
            local.tee 16
            local.get 7
            i64.mul
            i64.add
            local.tee 4
            i64.const 32
            i64.shl
            i64.add
            local.tee 5
            local.get 15
            i64.load
            i64.add
            local.tee 3
            i64.store
            local.get 8
            local.get 3
            local.get 5
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 8
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
            local.get 4
            i64.const 32
            i64.shr_u
            local.get 16
            local.get 6
            i64.mul
            i64.add
            i64.const 4294967296
            i64.const 0
            local.get 4
            local.get 1
            i64.lt_u
            select
            i64.add
            local.get 5
            local.get 2
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.tee 5
            local.get 8
            i64.add
            local.tee 8
            local.get 12
            i64.load
            i64.add
            local.tee 4
            i64.store
            local.get 4
            local.get 8
            i64.lt_u
            i64.extend_i32_u
            local.get 8
            local.get 5
            i64.lt_u
            i64.extend_i32_u
            i64.add
            local.set 8
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
  (export "arithmetic_mul" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
