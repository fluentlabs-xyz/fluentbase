(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i64 i32 i64 i64 i64 i64 i64 i32 i64 i64)
    i32.const 32768
    i32.const 0
    i64.load offset=32768
    local.tee 0
    i32.wrap_i64
    local.tee 1
    i32.sub
    i64.load align=1
    local.set 2
    i32.const 32776
    local.get 1
    i32.sub
    i64.load align=1
    local.set 3
    i32.const 32784
    local.get 1
    i32.sub
    i64.load align=1
    local.set 4
    i32.const 32792
    local.get 1
    i32.sub
    i64.load align=1
    local.set 5
    i32.const 0
    local.get 0
    i64.const 32
    i64.shl
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 6
    i64.store offset=32768
    i64.const 0
    local.set 0
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          i32.const 32776
          local.get 6
          i32.wrap_i64
          local.tee 7
          i32.sub
          i64.load align=1
          i32.const 32768
          local.get 7
          i32.sub
          local.tee 1
          i64.load align=1
          i64.or
          i32.const 32784
          local.get 7
          i32.sub
          i64.load align=1
          i64.or
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          i32.const 32792
          local.get 7
          i32.sub
          i64.load align=1
          local.tee 6
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
          local.tee 8
          i64.const 256
          i64.lt_u
          br_if 1 (;@2;)
        end
        i64.const 0
        local.set 6
        i64.const 0
        local.set 5
        i64.const 0
        local.set 4
        br 1 (;@1;)
      end
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
      local.set 9
      block  ;; label = @2
        block  ;; label = @3
          local.get 8
          i64.const 191
          i64.gt_u
          br_if 0 (;@3;)
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
          local.set 6
          local.get 8
          i64.const 127
          i64.gt_u
          br_if 1 (;@2;)
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
          local.set 5
          i64.const 0
          local.set 0
          i64.const 0
          local.get 8
          i64.sub
          local.set 3
          block  ;; label = @4
            local.get 8
            i64.const 63
            i64.gt_u
            br_if 0 (;@4;)
            local.get 5
            local.get 3
            i64.shr_u
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
            local.get 8
            i64.shl
            i64.or
            local.set 4
            local.get 6
            local.get 3
            i64.shr_u
            local.get 5
            local.get 8
            i64.shl
            i64.or
            local.set 5
            local.get 9
            local.get 3
            i64.shr_u
            local.get 6
            local.get 8
            i64.shl
            i64.or
            local.set 6
            local.get 9
            local.get 8
            i64.shl
            local.set 0
            br 3 (;@1;)
          end
          local.get 6
          local.get 3
          i64.shr_u
          local.get 5
          local.get 8
          i64.shl
          i64.or
          local.set 4
          local.get 9
          local.get 3
          i64.shr_u
          local.get 6
          local.get 8
          i64.shl
          i64.or
          local.set 5
          local.get 9
          local.get 8
          i64.shl
          local.set 6
          br 2 (;@1;)
        end
        local.get 9
        local.get 8
        i64.shl
        local.set 4
        i64.const 0
        local.set 6
        i64.const 0
        local.set 5
        br 1 (;@1;)
      end
      i64.const 0
      local.set 0
      local.get 9
      i64.const 0
      local.get 8
      i64.sub
      i64.shr_u
      local.get 6
      local.get 8
      i64.shl
      i64.or
      local.set 4
      local.get 9
      local.get 8
      i64.shl
      local.set 5
      i64.const 0
      local.set 6
    end
    local.get 1
    local.get 0
    i64.const 56
    i64.shl
    local.get 0
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 0
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 0
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 0
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 0
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 0
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 0
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=24 align=1
    local.get 1
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
    i64.store offset=16 align=1
    local.get 1
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
    i64.store offset=8 align=1
    local.get 1
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
    i64.store align=1)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_shl" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
