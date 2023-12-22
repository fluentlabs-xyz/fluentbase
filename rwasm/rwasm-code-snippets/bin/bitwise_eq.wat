(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i32 i64 i32 i64 i32 i32 i32 i32 i32)
    global.get 0
    i32.const 64
    i32.sub
    local.tee 0
    i32.const 8
    i32.add
    i32.const 32776
    i32.const 0
    i64.load offset=32768
    local.tee 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    i64.store
    local.get 0
    i32.const 16
    i32.add
    i32.const 32784
    local.get 2
    i32.sub
    i64.load align=1
    i64.store
    local.get 0
    i32.const 24
    i32.add
    i32.const 32792
    local.get 2
    i32.sub
    i64.load align=1
    i64.store
    i32.const 32768
    local.get 2
    i32.sub
    i64.load align=1
    local.set 3
    i32.const 0
    local.get 1
    i64.const 32
    i64.shl
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 1
    i64.store offset=32768
    local.get 0
    local.get 3
    i64.store
    local.get 0
    i32.const 32
    i32.add
    i32.const 24
    i32.add
    i32.const 32792
    local.get 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    i64.store
    local.get 0
    i32.const 32
    i32.add
    i32.const 16
    i32.add
    i32.const 32784
    local.get 2
    i32.sub
    i64.load align=1
    i64.store
    local.get 0
    i32.const 32
    i32.add
    i32.const 8
    i32.add
    i32.const 32776
    local.get 2
    i32.sub
    i64.load align=1
    i64.store
    local.get 0
    i32.const 32768
    local.get 2
    i32.sub
    local.tee 4
    i64.load align=1
    i64.store offset=32
    i32.const 1
    local.set 5
    i32.const 0
    local.set 6
    loop  ;; label = @1
      local.get 0
      i32.const 32
      i32.add
      local.get 6
      i32.add
      local.set 2
      block  ;; label = @2
        block  ;; label = @3
          local.get 5
          i32.const 1
          i32.and
          br_if 0 (;@3;)
          i32.const 0
          local.set 5
          local.get 2
          i32.const 0
          i32.store8
          br 1 (;@2;)
        end
        local.get 2
        i32.load8_u
        local.set 7
        i32.const 0
        local.set 5
        local.get 2
        i32.const 0
        i32.store8
        local.get 7
        local.get 0
        local.get 6
        i32.add
        local.tee 8
        i32.load8_u
        i32.ne
        br_if 0 (;@2;)
        local.get 2
        i32.const 1
        i32.add
        i32.load8_u
        local.get 8
        i32.const 1
        i32.add
        i32.load8_u
        i32.eq
        local.set 5
      end
      local.get 2
      i32.const 1
      i32.add
      i32.const 0
      i32.store8
      local.get 6
      i32.const 2
      i32.add
      local.tee 6
      i32.const 32
      i32.ne
      br_if 0 (;@1;)
    end
    i32.const 0
    local.get 1
    i64.store offset=32768
    local.get 4
    local.get 0
    i64.load offset=32
    i64.store align=1
    local.get 4
    i32.const 8
    i32.add
    local.get 0
    i32.const 32
    i32.add
    i32.const 8
    i32.add
    i64.load
    i64.store align=1
    local.get 4
    i32.const 16
    i32.add
    local.get 0
    i32.const 32
    i32.add
    i32.const 16
    i32.add
    i64.load
    i64.store align=1
    local.get 0
    local.get 5
    i32.store8 offset=63
    local.get 4
    i32.const 24
    i32.add
    local.get 0
    i32.const 32
    i32.add
    i32.const 24
    i32.add
    i64.load
    i64.store align=1)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_eq" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
