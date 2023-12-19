(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (func (;0;) (type 0)
    (local i32 i32 i32 i32 i32 i32 i64)
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
    block  ;; label = @1
      local.get 0
      i32.load8_u offset=32
      local.tee 2
      i32.const 128
      i32.and
      local.tee 3
      local.get 0
      i32.load8_u
      local.tee 4
      i32.const 128
      i32.and
      local.tee 5
      i32.gt_u
      br_if 0 (;@1;)
      i32.const 1
      local.set 1
      local.get 3
      local.get 5
      i32.lt_u
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 2
        i32.const 127
        i32.and
        local.tee 3
        local.get 4
        i32.const 127
        i32.and
        local.tee 1
        i32.ne
        br_if 0 (;@2;)
        i32.const 1
        local.set 3
        loop  ;; label = @3
          block  ;; label = @4
            local.get 3
            i32.const 32
            i32.ne
            br_if 0 (;@4;)
            i32.const 0
            local.set 1
            br 3 (;@1;)
          end
          local.get 0
          local.get 3
          i32.add
          local.set 1
          local.get 0
          i32.const 32
          i32.add
          local.get 3
          i32.add
          local.set 5
          local.get 3
          i32.const 1
          i32.add
          local.set 3
          local.get 5
          i32.load8_u
          local.tee 5
          local.get 1
          i32.load8_u
          local.tee 1
          i32.eq
          br_if 0 (;@3;)
        end
        local.get 5
        local.get 1
        i32.gt_u
        local.set 1
        br 1 (;@1;)
      end
      local.get 3
      local.get 1
      i32.gt_u
      local.set 1
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
    local.tee 6
    i64.store offset=500
    i32.const 523
    local.get 6
    i32.wrap_i64
    local.tee 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 516
    local.get 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 508
    local.get 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 500
    local.get 3
    i32.sub
    i64.const 0
    i64.store align=1
    i32.const 531
    local.get 3
    i32.sub
    local.get 1
    i32.store8
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
  (export "bitwise_sgt" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
