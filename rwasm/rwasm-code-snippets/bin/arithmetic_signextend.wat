(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i32 i32 i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 9
    local.get 4
    i64.store offset=24
    local.get 9
    local.get 3
    i64.store offset=16
    local.get 9
    local.get 2
    i64.store offset=8
    local.get 9
    local.get 1
    i64.store
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
      i64.const 0
      local.set 4
      local.get 5
      i32.wrap_i64
      local.tee 10
      i32.const 3
      i32.shr_u
      local.set 11
      block  ;; label = @2
        block  ;; label = @3
          local.get 9
          local.get 10
          i32.const -8
          i32.and
          i32.add
          local.tee 10
          i64.load
          local.tee 3
          local.get 5
          i64.const 3
          i64.shl
          local.tee 2
          i64.shr_u
          i64.const 128
          i64.and
          i64.const 0
          i64.ne
          br_if 0 (;@3;)
          local.get 3
          i64.const -1
          i64.const 56
          local.get 2
          i64.sub
          i64.const 56
          i64.and
          i64.shr_u
          i64.and
          local.set 3
          br 1 (;@2;)
        end
        i64.const -1
        local.set 4
        local.get 3
        i64.const -1
        local.get 2
        i64.const 8
        i64.add
        i64.const 56
        i64.and
        i64.shl
        i64.or
        local.set 3
      end
      local.get 10
      local.get 3
      i64.store
      local.get 11
      i32.const -3
      i32.add
      local.set 10
      local.get 11
      i32.const 3
      i32.shl
      local.get 9
      i32.add
      i32.const 8
      i32.add
      local.set 11
      loop  ;; label = @2
        block  ;; label = @3
          local.get 10
          br_if 0 (;@3;)
          local.get 9
          i64.load offset=24
          local.set 4
          local.get 9
          i64.load offset=16
          local.set 3
          local.get 9
          i64.load offset=8
          local.set 2
          local.get 9
          i64.load
          local.set 1
          br 2 (;@1;)
        end
        local.get 11
        local.get 4
        i64.store
        local.get 10
        i32.const 1
        i32.add
        local.set 10
        local.get 11
        i32.const 8
        i32.add
        local.set 11
        br 0 (;@2;)
      end
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
