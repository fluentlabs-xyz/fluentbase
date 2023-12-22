(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i32 i32 i64 i64 i64 i64 i64)
    global.get 0
    i32.const 32
    i32.sub
    drop
    i32.const 32832
    i32.const 0
    i32.load offset=32768
    local.tee 0
    i32.sub
    local.tee 1
    i64.load align=1
    local.set 2
    i32.const 32776
    local.get 0
    i32.sub
    i64.load align=1
    local.set 3
    i32.const 32784
    local.get 0
    i32.sub
    i64.load align=1
    local.set 4
    i32.const 32792
    local.get 0
    i32.sub
    i64.load align=1
    local.set 5
    local.get 1
    i32.const 32768
    local.get 0
    i32.sub
    i64.load align=1
    i64.store align=1
    i32.const 32856
    local.get 0
    i32.sub
    local.tee 1
    i64.load align=1
    local.set 6
    local.get 1
    local.get 5
    i64.store align=1
    i32.const 32848
    local.get 0
    i32.sub
    local.tee 1
    i64.load align=1
    local.set 5
    local.get 1
    local.get 4
    i64.store align=1
    i32.const 32840
    local.get 0
    i32.sub
    local.tee 0
    i64.load align=1
    local.set 4
    local.get 0
    local.get 3
    i64.store align=1
    i32.const 32792
    i32.const 0
    i32.load offset=32768
    local.tee 0
    i32.sub
    local.get 6
    i64.store align=1
    i32.const 32784
    local.get 0
    i32.sub
    local.get 5
    i64.store align=1
    i32.const 32776
    local.get 0
    i32.sub
    local.get 4
    i64.store align=1
    i32.const 32768
    local.get 0
    i32.sub
    local.get 2
    i64.store align=1)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "stack_swap2" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
