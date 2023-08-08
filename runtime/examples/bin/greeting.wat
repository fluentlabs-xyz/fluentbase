(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func (param i32 i32 i32) (result i32)))
  (type (;2;) (func (param i32)))
  (type (;3;) (func))
  (import "env" "_sys_read" (func (;0;) (type 1)))
  (import "env" "_evm_return" (func (;1;) (type 0)))
  (import "env" "_sys_halt" (func (;2;) (type 2)))
  (func (;3;) (type 3)
    (local i32 i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 8
    i32.add
    i32.const 8
    i32.add
    i32.const 0
    i32.store16
    local.get 0
    i64.const 0
    i64.store offset=8
    block  ;; label = @1
      local.get 0
      i32.const 8
      i32.add
      i32.const 0
      i32.const 10
      call 0
      i32.const 10
      i32.lt_u
      br_if 0 (;@1;)
      local.get 0
      local.get 0
      i32.load8_u offset=8
      local.get 0
      i32.load8_u offset=9
      i32.add
      local.get 0
      i32.load8_u offset=10
      i32.add
      local.get 0
      i32.load8_u offset=11
      i32.add
      local.get 0
      i32.load8_u offset=12
      i32.add
      local.get 0
      i32.load8_u offset=13
      i32.add
      local.get 0
      i32.load8_u offset=14
      i32.add
      local.get 0
      i32.load8_u offset=15
      i32.add
      local.get 0
      i32.load8_u offset=16
      i32.add
      local.get 0
      i32.load8_u offset=17
      i32.add
      local.tee 1
      i32.const 24
      i32.shl
      local.get 1
      i32.const 65280
      i32.and
      i32.const 8
      i32.shl
      i32.or
      local.get 1
      i32.const 8
      i32.shr_u
      i32.const 65280
      i32.and
      local.get 1
      i32.const 24
      i32.shr_u
      i32.or
      i32.or
      i32.store offset=24
      local.get 0
      i32.const 24
      i32.add
      i32.const 4
      call 1
      local.get 0
      i32.const 48
      i32.add
      global.set 0
      return
    end
    local.get 0
    i32.const 36
    i32.add
    i64.const 0
    i64.store align=4
    local.get 0
    i32.const 1
    i32.store offset=28
    local.get 0
    i32.const 1048592
    i32.store offset=24
    local.get 0
    i32.const 1048576
    i32.store offset=32
    local.get 0
    i32.const 24
    i32.add
    i32.const 1048636
    call 4
    unreachable)
  (func (;4;) (type 0) (param i32 i32)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 2
    global.set 0
    local.get 2
    local.get 0
    i32.store offset=20
    local.get 2
    i32.const 1048652
    i32.store offset=12
    local.get 2
    i32.const 1048652
    i32.store offset=8
    local.get 2
    i32.const 1
    i32.store8 offset=24
    local.get 2
    local.get 1
    i32.store offset=16
    local.get 2
    i32.const 8
    i32.add
    call 5
    unreachable)
  (func (;5;) (type 2) (param i32)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    i32.const 24
    i32.add
    local.get 0
    call 8
    local.get 1
    i32.const 8
    i32.add
    local.get 1
    i32.load offset=24
    local.tee 0
    local.get 1
    i32.load offset=28
    i32.load offset=12
    call_indirect (type 0)
    block  ;; label = @1
      local.get 0
      i32.eqz
      br_if 0 (;@1;)
      local.get 1
      i64.load offset=8
      i64.const -4493808902380553279
      i64.xor
      local.get 1
      i32.const 16
      i32.add
      i64.load
      i64.const -163230743173927068
      i64.xor
      i64.or
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      local.get 0
      i32.load
      local.get 0
      i32.load offset=4
      call 1
    end
    i32.const 1
    call 2
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func (;6;) (type 2) (param i32))
  (func (;7;) (type 0) (param i32 i32)
    local.get 0
    i64.const -4375259500804815539
    i64.store offset=8
    local.get 0
    i64.const 1135650936325378667
    i64.store)
  (func (;8;) (type 0) (param i32 i32)
    local.get 0
    local.get 1
    i64.load align=4
    i64.store)
  (table (;0;) 3 3 funcref)
  (memory (;0;) 17)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048668))
  (global (;2;) i32 (i32.const 1048672))
  (export "memory" (memory 0))
  (export "main" (func 3))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (elem (;0;) (i32.const 1) func 6 7)
  (data (;0;) (i32.const 1048576) "input not enough\00\00\10\00\10\00\00\00runtime/examples/greeting/src/lib.rs\18\00\10\00$\00\00\00\0a\00\00\00\09\00\00\00\01\00\00\00\00\00\00\00\01\00\00\00\02\00\00\00"))
