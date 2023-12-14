(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (type (;1;) (func (param i32 i32 i32)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i32)
    global.get 0
    i32.const 96
    i32.sub
    local.tee 9
    global.set 0
    local.get 9
    i64.const 1
    i64.store offset=56
    local.get 9
    i64.const 1
    i64.store offset=48
    local.get 9
    i64.const 1
    i64.store offset=40
    local.get 9
    i64.const 1
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
    local.get 9
    local.get 9
    i32.const 32
    i32.add
    local.get 9
    i32.const 64
    i32.add
    call 1
    local.get 9
    i64.load
    local.set 4
    local.get 9
    i64.load offset=8
    local.set 3
    local.get 9
    i64.load offset=16
    local.set 2
    local.get 9
    local.get 9
    i64.load offset=24
    i64.store offset=56
    local.get 9
    local.get 2
    i64.store offset=48
    local.get 9
    local.get 3
    i64.store offset=40
    local.get 9
    local.get 4
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
    local.get 9
    local.get 9
    i32.const 32
    i32.add
    local.get 9
    i32.const 64
    i32.add
    call 1
    local.get 9
    i64.load
    local.set 4
    local.get 9
    i64.load offset=8
    local.set 3
    local.get 9
    i64.load offset=16
    local.set 2
    local.get 0
    local.get 9
    i64.load offset=24
    i64.store offset=24
    local.get 0
    local.get 2
    i64.store offset=16
    local.get 0
    local.get 3
    i64.store offset=8
    local.get 0
    local.get 4
    i64.store
    local.get 9
    i32.const 96
    i32.add
    global.set 0)
  (func (;1;) (type 1) (param i32 i32 i32)
    (local i64 i64 i64 i64 i64 i64 i64 i64)
    local.get 2
    i64.load offset=24
    local.set 3
    local.get 2
    i64.load offset=16
    local.set 4
    local.get 2
    i64.load offset=8
    local.set 5
    local.get 2
    i64.load
    local.set 6
    local.get 1
    i64.load offset=24
    local.set 7
    local.get 1
    i64.load offset=16
    local.set 8
    local.get 1
    i64.load offset=8
    local.set 9
    local.get 1
    i64.load
    local.set 10
    i32.const 1
    local.set 2
    loop  ;; label = @1
      block  ;; label = @2
        local.get 2
        i32.const 1
        i32.and
        br_if 0 (;@2;)
        local.get 0
        local.get 7
        i64.store offset=24
        local.get 0
        local.get 8
        i64.store offset=16
        local.get 0
        local.get 9
        i64.store offset=8
        local.get 0
        local.get 10
        i64.store
        return
      end
      local.get 3
      local.get 7
      i64.mul
      local.set 7
      local.get 4
      local.get 8
      i64.mul
      local.set 8
      local.get 5
      local.get 9
      i64.mul
      local.set 9
      local.get 6
      local.get 10
      i64.mul
      local.set 10
      i32.const 0
      local.set 2
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
