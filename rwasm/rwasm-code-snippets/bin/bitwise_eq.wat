(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (func (;0;) (type 0)
    (local i32 i32 i32 i32 i64)
    global.get 0
    i32.const 64
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    call 1
    local.get 0
    i32.const 32
    i32.add
    call 1
    i32.const 0
    local.set 1
    i32.const 1
    local.set 2
    block  ;; label = @1
      loop  ;; label = @2
        local.get 1
        i32.const 32
        i32.eq
        br_if 1 (;@1;)
        local.get 2
        i32.const 1
        i32.and
        local.set 3
        i32.const 0
        local.set 2
        block  ;; label = @3
          local.get 3
          i32.eqz
          br_if 0 (;@3;)
          local.get 0
          i32.const 32
          i32.add
          local.get 1
          i32.add
          i32.load8_u
          local.get 0
          local.get 1
          i32.add
          i32.load8_u
          i32.eq
          local.set 2
        end
        local.get 0
        i32.const 32
        i32.add
        local.get 1
        i32.add
        i32.const 0
        i32.store8
        local.get 1
        i32.const 1
        i32.add
        local.set 1
        br 0 (;@2;)
      end
    end
    i32.const 0
    i32.const 0
    i64.load offset=500
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 4
    i64.store offset=500
    local.get 0
    local.get 2
    i32.const 1
    i32.and
    i32.store8 offset=63
    i32.const 524
    local.get 4
    i32.wrap_i64
    local.tee 1
    i32.sub
    local.get 0
    i32.const 56
    i32.add
    i64.load align=1
    i64.store align=1
    i32.const 516
    local.get 1
    i32.sub
    local.get 0
    i32.const 48
    i32.add
    i64.load align=1
    i64.store align=1
    i32.const 508
    local.get 1
    i32.sub
    local.get 0
    i32.const 40
    i32.add
    i64.load align=1
    i64.store align=1
    i32.const 500
    local.get 1
    i32.sub
    local.get 0
    i64.load offset=32 align=1
    i64.store align=1
    local.get 0
    i32.const 64
    i32.add
    global.set 0)
  (func (;1;) (type 1) (param i32)
    (local i64 i32)
    local.get 0
    i32.const 500
    i32.const 0
    i64.load offset=500
    local.tee 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    i32.const 508
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    i32.const 516
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 24
    i32.add
    i32.const 524
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
    i64.store offset=500)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_eq" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
