(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func (param i32)))
  (type (;2;) (func))
  (import "env" "_sys_write" (func (;0;) (type 0)))
  (import "env" "_sys_halt" (func (;1;) (type 1)))
  (func (;2;) (type 0) (param i32 i32)
    (local i32 i32 i64)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 2
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 2
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 2
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 2
    i64.const 0
    i64.store
    local.get 1
    i32.const 24
    i32.add
    local.set 3
    i32.const 0
    local.set 1
    block  ;; label = @1
      loop  ;; label = @2
        local.get 1
        i32.const 32
        i32.eq
        br_if 1 (;@1;)
        local.get 2
        local.get 1
        i32.add
        local.get 3
        i64.load align=1
        local.tee 4
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
        i64.store
        local.get 1
        i32.const 8
        i32.add
        local.set 1
        local.get 3
        i32.const -8
        i32.add
        local.set 3
        br 0 (;@2;)
      end
    end
    local.get 0
    local.get 2
    i64.load
    i64.store
    local.get 0
    i32.const 24
    i32.add
    local.get 2
    i32.const 24
    i32.add
    i64.load
    i64.store
    local.get 0
    i32.const 16
    i32.add
    local.get 2
    i32.const 16
    i32.add
    i64.load
    i64.store
    local.get 0
    i32.const 8
    i32.add
    local.get 2
    i32.const 8
    i32.add
    i64.load
    i64.store)
  (func (;3;) (type 1) (param i32))
  (func (;4;) (type 2)
    (local i32 i32 i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 0
    global.set 0
    block  ;; label = @1
      i32.const 0
      i32.load offset=12
      i32.const 1
      i32.add
      local.tee 1
      i32.eqz
      br_if 0 (;@1;)
      local.get 1
      i32.const 0
      i32.load offset=4
      local.tee 2
      local.get 2
      i32.const 1
      i32.add
      i32.const 3
      i32.shr_u
      i32.const 7
      i32.mul
      local.get 2
      i32.const 8
      i32.lt_u
      select
      i32.const 1
      i32.add
      local.tee 2
      local.get 1
      local.get 2
      i32.gt_u
      select
      local.tee 1
      i32.const 8
      i32.lt_u
      br_if 0 (;@1;)
      local.get 1
      i32.const 536870911
      i32.gt_u
      drop
    end
    local.get 0
    i32.const 20
    i32.add
    i64.const 0
    i64.store align=4
    local.get 0
    i32.const 1
    i32.store offset=12
    local.get 0
    i32.const 1048604
    i32.store offset=8
    local.get 0
    i32.const 1048640
    i32.store offset=16
    local.get 0
    i32.const 8
    i32.add
    call 5
    unreachable)
  (func (;5;) (type 1) (param i32)
    (local i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    i32.const 1048640
    call 8
    block  ;; label = @1
      local.get 1
      i64.load
      i64.const -4493808902380553279
      i64.xor
      local.get 1
      i32.const 8
      i32.add
      i64.load
      i64.const -163230743173927068
      i64.xor
      i64.or
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      local.get 1
      local.get 1
      call 0
    end
    i32.const -71
    call 1
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func (;6;) (type 1) (param i32)
    (local i64 i32)
    local.get 0
    i32.const 32768
    i32.const 0
    i64.load offset=32768
    local.tee 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    i32.const 32776
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    i32.const 32784
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 24
    i32.add
    i32.const 32792
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
    i64.store offset=32768)
  (func (;7;) (type 2)
    (local i32)
    global.get 0
    i32.const 128
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    call 6
    local.get 0
    i32.const 32
    i32.add
    call 6
    local.get 0
    i32.const 64
    i32.add
    local.get 0
    call 2
    local.get 0
    i32.const 96
    i32.add
    local.get 0
    i32.const 32
    i32.add
    call 2
    i32.const 0
    i32.const 0
    i64.load offset=1048632
    i64.store offset=8
    i32.const 0
    i32.const 0
    i64.load offset=1048624
    i64.store
    local.get 0
    i32.const 96
    i32.add
    call 3
    call 4
    unreachable)
  (func (;8;) (type 0) (param i32 i32)
    local.get 0
    i64.const 568815540544143123
    i64.store offset=8
    local.get 0
    i64.const 5657071353825360256
    i64.store)
  (memory (;0;) 17)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048640))
  (global (;2;) i32 (i32.const 1048640))
  (export "memory" (memory 0))
  (export "host_tstore" (func 7))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data (;0;) (i32.const 1048576) "Hash table capacity overflow\00\00\10\00\1c\00\00\00\00\00\00\00\ff\ff\ff\ff\ff\ff\ff\ff(\00\10\00\00\00\00\00\00\00\00\00\00\00\00\00"))
