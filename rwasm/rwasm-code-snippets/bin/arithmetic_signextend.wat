(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i32 i64 i32 i64 i64 i64 i64 i64 i64 i64 i32 i64 i32 i32 i32 i32)
    global.get 0
    local.set 0
    i32.const 32792
    i32.const 0
    i64.load offset=32768
    local.tee 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    local.set 3
    i32.const 32784
    local.get 2
    i32.sub
    i64.load align=1
    local.set 4
    i32.const 32776
    local.get 2
    i32.sub
    i64.load align=1
    local.set 5
    i32.const 32768
    local.get 2
    i32.sub
    i64.load align=1
    local.set 6
    i32.const 0
    local.get 1
    i64.const 32
    i64.shl
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 7
    i64.store offset=32768
    i32.const 32784
    local.get 7
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    local.set 8
    i32.const 32776
    local.get 2
    i32.sub
    i64.load align=1
    local.set 9
    i32.const 32768
    local.get 2
    i32.sub
    local.tee 10
    i64.load align=1
    local.set 11
    i32.const 32792
    local.get 2
    i32.sub
    i64.load align=1
    local.set 1
    local.get 0
    i32.const 32
    i32.sub
    local.tee 12
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
    i64.store offset=24
    local.get 12
    local.get 5
    i64.const 56
    i64.shl
    local.get 5
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 5
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 5
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 5
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 5
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 5
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 5
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=16
    local.get 12
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
    i64.store offset=8
    local.get 12
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
    i64.store
    block  ;; label = @1
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
      local.tee 4
      i64.const 31
      i64.gt_u
      br_if 0 (;@1;)
      local.get 8
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      local.get 9
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      local.get 11
      i64.eqz
      i32.eqz
      br_if 0 (;@1;)
      i64.const 0
      local.set 3
      block  ;; label = @2
        block  ;; label = @3
          local.get 12
          local.get 4
          i32.wrap_i64
          local.tee 2
          i32.const -8
          i32.and
          i32.add
          local.tee 0
          i64.load
          local.tee 5
          local.get 4
          i64.const 3
          i64.shl
          local.tee 4
          i64.shr_u
          i64.const 128
          i64.and
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 5
          i64.const -1
          i64.const 56
          local.get 4
          i64.sub
          i64.const 56
          i64.and
          i64.shr_u
          i64.and
          local.set 4
          br 1 (;@2;)
        end
        i64.const -1
        local.set 3
        local.get 5
        i64.const -1
        local.get 4
        i64.const 8
        i64.add
        i64.const 56
        i64.and
        i64.shl
        i64.or
        local.set 4
      end
      local.get 0
      local.get 4
      i64.store
      local.get 2
      i32.const 23
      i32.gt_u
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 2
        i32.const 3
        i32.shr_u
        local.tee 2
        i32.const 2
        local.get 2
        i32.const 2
        i32.gt_u
        select
        local.tee 13
        local.get 2
        i32.sub
        local.tee 14
        i32.const 1
        i32.add
        i32.const 7
        i32.and
        local.tee 0
        i32.eqz
        br_if 0 (;@2;)
        local.get 2
        local.get 0
        i32.add
        local.set 15
        local.get 0
        i32.const 3
        i32.shl
        local.set 0
        local.get 2
        i32.const 3
        i32.shl
        local.get 12
        i32.add
        i32.const 8
        i32.add
        local.set 2
        loop  ;; label = @3
          local.get 2
          local.get 3
          i64.store
          local.get 2
          i32.const 8
          i32.add
          local.set 2
          local.get 0
          i32.const -8
          i32.add
          local.tee 0
          br_if 0 (;@3;)
        end
        local.get 15
        local.set 2
      end
      local.get 14
      i32.const 7
      i32.lt_u
      br_if 0 (;@1;)
      local.get 13
      local.get 2
      i32.sub
      i32.const 1
      i32.add
      local.set 0
      local.get 12
      local.get 2
      i32.const 3
      i32.shl
      i32.add
      local.set 2
      loop  ;; label = @2
        local.get 2
        i32.const 64
        i32.add
        local.tee 14
        local.get 3
        i64.store
        local.get 2
        i32.const 56
        i32.add
        local.get 3
        i64.store
        local.get 2
        i32.const 48
        i32.add
        local.get 3
        i64.store
        local.get 2
        i32.const 40
        i32.add
        local.get 3
        i64.store
        local.get 2
        i32.const 32
        i32.add
        local.get 3
        i64.store
        local.get 2
        i32.const 24
        i32.add
        local.get 3
        i64.store
        local.get 2
        i32.const 16
        i32.add
        local.get 3
        i64.store
        local.get 2
        i32.const 8
        i32.add
        local.get 3
        i64.store
        local.get 14
        local.set 2
        local.get 0
        i32.const -8
        i32.add
        local.tee 0
        br_if 0 (;@2;)
      end
    end
    i32.const 0
    local.get 7
    i64.store offset=32768
    local.get 10
    local.get 12
    i64.load
    local.tee 3
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
    i64.store offset=24 align=1
    local.get 10
    local.get 12
    i64.load offset=8
    local.tee 3
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
    i64.store offset=16 align=1
    local.get 10
    local.get 12
    i64.load offset=16
    local.tee 3
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
    i64.store offset=8 align=1
    local.get 10
    local.get 12
    i64.load offset=24
    local.tee 3
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
    i64.store align=1)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_signextend" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
