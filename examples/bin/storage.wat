(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32 i32) (result i32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func (param i32)))
  (type (;4;) (func))
  (type (;5;) (func (param i32 i32 i32)))
  (type (;6;) (func (param i32 i32 i32 i32)))
  (type (;7;) (func (param i32) (result i32)))
  (type (;8;) (func (param i32 i32 i32 i32) (result i32)))
  (import "env" "_evm_sstore" (func (;0;) (type 2)))
  (import "env" "_evm_sload" (func (;1;) (type 2)))
  (import "env" "_sys_write" (func (;2;) (type 2)))
  (import "env" "_sys_halt" (func (;3;) (type 3)))
  (func (;4;) (type 4)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 30
    i32.add
    i32.const 0
    i32.store16
    local.get 0
    i32.const 22
    i32.add
    i64.const 0
    i64.store align=2
    local.get 0
    i64.const 0
    i64.store offset=14 align=2
    local.get 0
    i32.const 0
    i64.load offset=1048576 align=1
    i64.store
    local.get 0
    i32.const 0
    i64.load offset=1048582 align=1
    i64.store offset=6 align=2
    i32.const 1048590
    local.get 0
    call 0
    local.get 0
    i32.const 32
    i32.add
    global.set 0)
  (func (;5;) (type 4)
    (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    global.get 0
    i32.const 560
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store
    i32.const 1048590
    local.get 0
    call 1
    local.get 0
    i32.const 504
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 496
    i32.add
    local.tee 1
    i64.const 17179869184
    i64.store
    local.get 0
    i64.const 0
    i64.store offset=488
    i32.const 0
    local.set 2
    local.get 0
    i32.const 0
    i32.store8 offset=461
    local.get 0
    i32.const 0
    i32.store8 offset=440
    local.get 0
    i32.const 0
    i32.store8 offset=407
    local.get 0
    i32.const 0
    i32.store8 offset=374
    local.get 0
    i32.const 0
    i32.store8 offset=353
    local.get 0
    i32.const 0
    i32.store8 offset=332
    local.get 0
    i32.const 0
    i32.store offset=320
    local.get 0
    i32.const 0
    i32.store offset=308
    local.get 0
    i32.const 0
    i32.store offset=296
    local.get 0
    i32.const 0
    i32.store offset=288
    local.get 0
    i64.const 2
    i64.store offset=248
    local.get 0
    i64.const 0
    i64.store offset=232
    local.get 0
    i64.const 0
    i64.store offset=192
    local.get 0
    i64.const 0
    i64.store offset=152
    local.get 0
    i64.const 0
    i64.store offset=136
    local.get 0
    i64.const 0
    i64.store offset=120
    local.get 0
    i64.const 0
    i64.store offset=104
    local.get 0
    i64.const 0
    i64.store offset=88
    local.get 0
    i64.const 0
    i64.store offset=48
    local.get 0
    i64.const 0
    i64.store offset=32
    local.get 0
    i32.const 1
    i32.store offset=488
    local.get 0
    i32.const 488
    i32.add
    local.get 0
    i32.const 32
    call 6
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i32.load offset=488
        local.tee 3
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        i32.const 512
        i32.add
        i32.const 24
        call 7
        local.get 3
        local.get 1
        i32.load
        local.get 0
        i32.const 512
        i32.add
        i32.const 0
        call 8
        local.get 0
        i32.const 512
        i32.add
        i32.const 12
        local.get 0
        i32.const 508
        i32.add
        i32.load
        local.tee 1
        call 9
        local.get 0
        i32.const 528
        i32.add
        local.get 1
        i32.const 44
        i32.mul
        local.tee 1
        call 7
        local.get 0
        i32.const 500
        i32.add
        i32.load
        local.tee 4
        i32.const 24
        i32.add
        local.set 5
        local.get 4
        local.get 1
        i32.add
        local.set 6
        i32.const 0
        local.set 7
        loop  ;; label = @3
          block  ;; label = @4
            local.get 4
            local.get 6
            i32.ne
            br_if 0 (;@4;)
            local.get 0
            i32.const 512
            i32.add
            i32.const 16
            local.get 0
            i32.load offset=528
            local.get 0
            i32.load offset=536
            call 10
            local.get 0
            i32.load offset=512
            local.get 0
            i32.load offset=520
            call 2
            i32.const 0
            call 3
            local.get 3
            i32.eqz
            br_if 3 (;@1;)
            local.get 0
            i32.const 500
            i32.add
            i32.load
            local.get 0
            i32.const 508
            i32.add
            i32.load
            call 11
            br 3 (;@1;)
          end
          local.get 5
          local.get 2
          i32.add
          local.set 8
          local.get 7
          i32.const 44
          i32.mul
          local.set 9
          local.get 4
          i32.const 44
          i32.add
          local.set 10
          i32.const 0
          local.set 1
          block  ;; label = @4
            loop  ;; label = @5
              local.get 1
              i32.const 20
              i32.eq
              br_if 1 (;@4;)
              local.get 8
              local.get 1
              i32.add
              i32.load8_u
              local.get 0
              i32.const 528
              i32.add
              local.get 2
              local.get 1
              i32.add
              call 12
              local.get 1
              i32.const 1
              i32.add
              local.set 1
              br 0 (;@5;)
            end
          end
          local.get 0
          i32.const 528
          i32.add
          local.get 9
          i32.const 20
          i32.add
          local.get 4
          i32.load offset=8
          local.tee 1
          call 9
          local.get 0
          i32.const 544
          i32.add
          local.get 1
          i32.const 5
          i32.shl
          local.tee 1
          call 7
          local.get 4
          i32.load
          local.tee 11
          local.get 1
          i32.add
          local.set 12
          i32.const 0
          local.set 8
          local.get 11
          local.set 13
          loop  ;; label = @4
            block  ;; label = @5
              local.get 13
              local.get 12
              i32.ne
              br_if 0 (;@5;)
              local.get 0
              i32.const 528
              i32.add
              local.get 9
              i32.const 24
              i32.add
              local.get 0
              i32.load offset=544
              local.get 0
              i32.load offset=552
              call 10
              local.get 4
              i32.const 12
              i32.add
              i32.load
              local.get 4
              i32.const 20
              i32.add
              i32.load
              local.get 0
              i32.const 528
              i32.add
              local.get 9
              i32.const 32
              i32.add
              call 8
              local.get 2
              i32.const 44
              i32.add
              local.set 2
              local.get 7
              i32.const 1
              i32.add
              local.set 7
              local.get 10
              local.set 4
              br 2 (;@3;)
            end
            local.get 11
            local.get 8
            i32.add
            local.set 14
            local.get 13
            i32.const 32
            i32.add
            local.set 13
            i32.const 0
            local.set 1
            block  ;; label = @5
              loop  ;; label = @6
                local.get 1
                i32.const 32
                i32.eq
                br_if 1 (;@5;)
                local.get 14
                local.get 1
                i32.add
                i32.load8_u
                local.get 0
                i32.const 544
                i32.add
                local.get 8
                local.get 1
                i32.add
                call 12
                local.get 1
                i32.const 1
                i32.add
                local.set 1
                br 0 (;@6;)
              end
            end
            local.get 8
            i32.const 32
            i32.add
            local.set 8
            br 0 (;@4;)
          end
        end
      end
      i32.const 0
      call 3
    end
    local.get 0
    i32.const 560
    i32.add
    global.set 0)
  (func (;6;) (type 5) (param i32 i32 i32)
    (local i32)
    local.get 0
    local.get 2
    call 21
    local.get 0
    i32.load
    local.get 0
    i32.load offset=8
    local.tee 3
    i32.add
    local.get 1
    local.get 2
    call 30
    drop
    local.get 0
    local.get 3
    local.get 2
    i32.add
    i32.store offset=8)
  (func (;7;) (type 2) (param i32 i32)
    (local i32 i32 i32 i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 2
    global.set 0
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            br_if 0 (;@4;)
            local.get 2
            i32.const 0
            i32.store offset=28
            local.get 2
            local.get 1
            i32.store offset=24
            local.get 2
            i32.const 1
            i32.store offset=20
            br 1 (;@3;)
          end
          local.get 1
          i32.const -1
          i32.le_s
          br_if 1 (;@2;)
          local.get 2
          i32.const 8
          i32.add
          local.get 1
          call 19
          local.get 2
          i32.load offset=8
          local.tee 3
          i32.eqz
          br_if 2 (;@1;)
          local.get 2
          i32.const 0
          i32.store offset=28
          local.get 2
          local.get 1
          i32.store offset=24
          local.get 2
          local.get 3
          i32.store offset=20
          local.get 2
          i32.const 20
          i32.add
          local.get 1
          call 21
          local.get 1
          i32.const -1
          i32.add
          local.set 3
          local.get 2
          i32.load offset=20
          local.get 2
          i32.load offset=28
          local.tee 4
          i32.add
          local.set 5
          block  ;; label = @4
            loop  ;; label = @5
              local.get 5
              i32.const 0
              i32.store8
              local.get 3
              i32.eqz
              br_if 1 (;@4;)
              local.get 3
              i32.const -1
              i32.add
              local.set 3
              local.get 5
              i32.const 1
              i32.add
              local.set 5
              br 0 (;@5;)
            end
          end
          local.get 2
          local.get 4
          local.get 1
          i32.add
          i32.store offset=28
        end
        local.get 0
        local.get 2
        i64.load offset=20 align=4
        i64.store align=4
        local.get 0
        local.get 1
        i32.store offset=12
        local.get 0
        i32.const 8
        i32.add
        local.get 2
        i32.const 20
        i32.add
        i32.const 8
        i32.add
        i32.load
        i32.store
        local.get 2
        i32.const 32
        i32.add
        global.set 0
        return
      end
      call 15
      unreachable
    end
    local.get 1
    call 13
    unreachable)
  (func (;8;) (type 6) (param i32 i32 i32 i32)
    (local i32 i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 4
    global.set 0
    local.get 2
    local.get 3
    local.get 1
    call 9
    local.get 4
    local.get 1
    call 7
    i32.const 0
    local.set 5
    loop  ;; label = @1
      block  ;; label = @2
        local.get 1
        local.get 5
        i32.ne
        br_if 0 (;@2;)
        local.get 2
        local.get 3
        i32.const 4
        i32.add
        local.get 4
        i32.load
        local.get 4
        i32.load offset=8
        call 10
        local.get 4
        i32.const 16
        i32.add
        global.set 0
        return
      end
      local.get 0
      local.get 5
      i32.add
      i32.load8_u
      local.get 4
      local.get 5
      call 12
      local.get 5
      i32.const 1
      i32.add
      local.set 5
      br 0 (;@1;)
    end)
  (func (;9;) (type 5) (param i32 i32 i32)
    (local i32)
    local.get 0
    i32.load offset=12
    local.get 1
    i32.const 4
    call 28
    local.set 1
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i32.load offset=8
        local.tee 3
        local.get 1
        i32.lt_u
        br_if 0 (;@2;)
        local.get 3
        local.get 1
        i32.sub
        local.tee 3
        i32.const 3
        i32.le_u
        br_if 1 (;@1;)
        local.get 0
        i32.load
        local.get 1
        i32.add
        local.get 2
        i32.store align=1
        return
      end
      local.get 1
      local.get 3
      call 24
      unreachable
    end
    local.get 3
    call 27
    unreachable)
  (func (;10;) (type 6) (param i32 i32 i32 i32)
    local.get 0
    local.get 1
    local.get 0
    i32.load offset=8
    call 9
    local.get 0
    local.get 1
    i32.const 4
    i32.add
    local.get 3
    call 9
    local.get 0
    local.get 2
    local.get 3
    call 6)
  (func (;11;) (type 2) (param i32 i32))
  (func (;12;) (type 5) (param i32 i32 i32)
    (local i32)
    local.get 1
    i32.load offset=12
    local.get 2
    i32.const 1
    call 28
    local.set 2
    block  ;; label = @1
      local.get 1
      i32.load offset=8
      local.tee 3
      local.get 2
      i32.gt_u
      br_if 0 (;@1;)
      local.get 2
      local.get 3
      call 25
      unreachable
    end
    local.get 1
    i32.load
    local.get 2
    i32.add
    local.get 0
    i32.store8)
  (func (;13;) (type 3) (param i32)
    local.get 0
    call 14
    unreachable)
  (func (;14;) (type 3) (param i32)
    (local i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    local.get 0
    i32.store offset=12
    local.get 1
    i32.const 28
    i32.add
    i64.const 1
    i64.store align=4
    local.get 1
    i32.const 2
    i32.store offset=20
    local.get 1
    i32.const 1048728
    i32.store offset=16
    local.get 1
    i32.const 1
    i32.store offset=44
    local.get 1
    local.get 1
    i32.const 40
    i32.add
    i32.store offset=24
    local.get 1
    local.get 1
    i32.const 12
    i32.add
    i32.store offset=40
    local.get 1
    i32.const 16
    i32.add
    call 18
    unreachable)
  (func (;15;) (type 4)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    i32.const 20
    i32.add
    i64.const 0
    i64.store align=4
    local.get 0
    i32.const 1
    i32.store offset=12
    local.get 0
    i32.const 1048668
    i32.store offset=8
    local.get 0
    i32.const 1049112
    i32.store offset=16
    local.get 0
    i32.const 8
    i32.add
    i32.const 1048676
    call 16
    unreachable)
  (func (;16;) (type 2) (param i32 i32)
    (local i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 2
    global.set 0
    local.get 2
    i32.const 1049112
    call 23
    block  ;; label = @1
      local.get 2
      i64.load
      i64.const -4493808902380553279
      i64.xor
      local.get 2
      i32.const 8
      i32.add
      i64.load
      i64.const -163230743173927068
      i64.xor
      i64.or
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      local.get 2
      local.get 2
      call 2
    end
    i32.const -71
    call 3
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func (;17;) (type 1) (param i32 i32) (result i32)
    (local i32 i32 i64 i64 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 2
    global.set 0
    i32.const 39
    local.set 3
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i64.load32_u
        local.tee 4
        i64.const 10000
        i64.ge_u
        br_if 0 (;@2;)
        local.get 4
        local.set 5
        br 1 (;@1;)
      end
      i32.const 39
      local.set 3
      loop  ;; label = @2
        local.get 2
        i32.const 9
        i32.add
        local.get 3
        i32.add
        local.tee 0
        i32.const -4
        i32.add
        local.get 4
        i64.const 10000
        i64.div_u
        local.tee 5
        i64.const 55536
        i64.mul
        local.get 4
        i64.add
        i32.wrap_i64
        local.tee 6
        i32.const 65535
        i32.and
        i32.const 100
        i32.div_u
        local.tee 7
        i32.const 1
        i32.shl
        i32.const 1048812
        i32.add
        i32.load16_u align=1
        i32.store16 align=1
        local.get 0
        i32.const -2
        i32.add
        local.get 7
        i32.const -100
        i32.mul
        local.get 6
        i32.add
        i32.const 65535
        i32.and
        i32.const 1
        i32.shl
        i32.const 1048812
        i32.add
        i32.load16_u align=1
        i32.store16 align=1
        local.get 3
        i32.const -4
        i32.add
        local.set 3
        local.get 4
        i64.const 99999999
        i64.gt_u
        local.set 0
        local.get 5
        local.set 4
        local.get 0
        br_if 0 (;@2;)
      end
    end
    block  ;; label = @1
      local.get 5
      i32.wrap_i64
      local.tee 0
      i32.const 99
      i32.le_u
      br_if 0 (;@1;)
      local.get 2
      i32.const 9
      i32.add
      local.get 3
      i32.const -2
      i32.add
      local.tee 3
      i32.add
      local.get 5
      i32.wrap_i64
      local.tee 6
      i32.const 65535
      i32.and
      i32.const 100
      i32.div_u
      local.tee 0
      i32.const -100
      i32.mul
      local.get 6
      i32.add
      i32.const 65535
      i32.and
      i32.const 1
      i32.shl
      i32.const 1048812
      i32.add
      i32.load16_u align=1
      i32.store16 align=1
    end
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i32.const 10
        i32.lt_u
        br_if 0 (;@2;)
        local.get 2
        i32.const 9
        i32.add
        local.get 3
        i32.const -2
        i32.add
        local.tee 3
        i32.add
        local.get 0
        i32.const 1
        i32.shl
        i32.const 1048812
        i32.add
        i32.load16_u align=1
        i32.store16 align=1
        br 1 (;@1;)
      end
      local.get 2
      i32.const 9
      i32.add
      local.get 3
      i32.const -1
      i32.add
      local.tee 3
      i32.add
      local.get 0
      i32.const 48
      i32.add
      i32.store8
    end
    i32.const 39
    local.get 3
    i32.sub
    local.set 8
    i32.const 1
    local.set 7
    i32.const 43
    i32.const 1114112
    local.get 1
    i32.load offset=28
    local.tee 0
    i32.const 1
    i32.and
    local.tee 6
    select
    local.set 9
    local.get 0
    i32.const 29
    i32.shl
    i32.const 31
    i32.shr_s
    i32.const 1049112
    i32.and
    local.set 10
    local.get 2
    i32.const 9
    i32.add
    local.get 3
    i32.add
    local.set 11
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        i32.load
        br_if 0 (;@2;)
        local.get 1
        i32.load offset=20
        local.tee 3
        local.get 1
        i32.load offset=24
        local.tee 0
        local.get 9
        local.get 10
        call 26
        br_if 1 (;@1;)
        local.get 3
        local.get 11
        local.get 8
        local.get 0
        i32.load offset=12
        call_indirect (type 0)
        local.set 7
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 1
        i32.load offset=4
        local.tee 12
        local.get 6
        local.get 8
        i32.add
        local.tee 7
        i32.gt_u
        br_if 0 (;@2;)
        i32.const 1
        local.set 7
        local.get 1
        i32.load offset=20
        local.tee 3
        local.get 1
        i32.load offset=24
        local.tee 0
        local.get 9
        local.get 10
        call 26
        br_if 1 (;@1;)
        local.get 3
        local.get 11
        local.get 8
        local.get 0
        i32.load offset=12
        call_indirect (type 0)
        local.set 7
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 0
        i32.const 8
        i32.and
        i32.eqz
        br_if 0 (;@2;)
        local.get 1
        i32.load offset=16
        local.set 13
        local.get 1
        i32.const 48
        i32.store offset=16
        local.get 1
        i32.load8_u offset=32
        local.set 14
        i32.const 1
        local.set 7
        local.get 1
        i32.const 1
        i32.store8 offset=32
        local.get 1
        i32.load offset=20
        local.tee 0
        local.get 1
        i32.load offset=24
        local.tee 15
        local.get 9
        local.get 10
        call 26
        br_if 1 (;@1;)
        local.get 3
        local.get 12
        i32.add
        local.get 6
        i32.sub
        i32.const -38
        i32.add
        local.set 3
        block  ;; label = @3
          loop  ;; label = @4
            local.get 3
            i32.const -1
            i32.add
            local.tee 3
            i32.eqz
            br_if 1 (;@3;)
            local.get 0
            i32.const 48
            local.get 15
            i32.load offset=16
            call_indirect (type 1)
            i32.eqz
            br_if 0 (;@4;)
            br 3 (;@1;)
          end
        end
        local.get 0
        local.get 11
        local.get 8
        local.get 15
        i32.load offset=12
        call_indirect (type 0)
        br_if 1 (;@1;)
        local.get 1
        local.get 14
        i32.store8 offset=32
        local.get 1
        local.get 13
        i32.store offset=16
        i32.const 0
        local.set 7
        br 1 (;@1;)
      end
      local.get 12
      local.get 7
      i32.sub
      local.set 12
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            i32.load8_u offset=32
            local.tee 3
            br_table 2 (;@2;) 0 (;@4;) 1 (;@3;) 0 (;@4;) 2 (;@2;)
          end
          local.get 12
          local.set 3
          i32.const 0
          local.set 12
          br 1 (;@2;)
        end
        local.get 12
        i32.const 1
        i32.shr_u
        local.set 3
        local.get 12
        i32.const 1
        i32.add
        i32.const 1
        i32.shr_u
        local.set 12
      end
      local.get 3
      i32.const 1
      i32.add
      local.set 3
      local.get 1
      i32.const 24
      i32.add
      i32.load
      local.set 0
      local.get 1
      i32.load offset=16
      local.set 15
      local.get 1
      i32.load offset=20
      local.set 6
      block  ;; label = @2
        loop  ;; label = @3
          local.get 3
          i32.const -1
          i32.add
          local.tee 3
          i32.eqz
          br_if 1 (;@2;)
          local.get 6
          local.get 15
          local.get 0
          i32.load offset=16
          call_indirect (type 1)
          i32.eqz
          br_if 0 (;@3;)
        end
        i32.const 1
        local.set 7
        br 1 (;@1;)
      end
      i32.const 1
      local.set 7
      local.get 6
      local.get 0
      local.get 9
      local.get 10
      call 26
      br_if 0 (;@1;)
      local.get 6
      local.get 11
      local.get 8
      local.get 0
      i32.load offset=12
      call_indirect (type 0)
      br_if 0 (;@1;)
      i32.const 0
      local.set 3
      loop  ;; label = @2
        block  ;; label = @3
          local.get 12
          local.get 3
          i32.ne
          br_if 0 (;@3;)
          local.get 12
          local.get 12
          i32.lt_u
          local.set 7
          br 2 (;@1;)
        end
        local.get 3
        i32.const 1
        i32.add
        local.set 3
        local.get 6
        local.get 15
        local.get 0
        i32.load offset=16
        call_indirect (type 1)
        i32.eqz
        br_if 0 (;@2;)
      end
      local.get 3
      i32.const -1
      i32.add
      local.get 12
      i32.lt_u
      local.set 7
    end
    local.get 2
    i32.const 48
    i32.add
    global.set 0
    local.get 7)
  (func (;18;) (type 3) (param i32)
    (local i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    i32.const 1049112
    call 23
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
      call 2
    end
    i32.const -71
    call 3
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func (;19;) (type 2) (param i32 i32)
    (local i32)
    i32.const 0
    i32.load8_u offset=1049316
    drop
    local.get 1
    call 20
    local.set 2
    local.get 0
    local.get 1
    i32.store offset=4
    local.get 0
    local.get 2
    i32.store)
  (func (;20;) (type 7) (param i32) (result i32)
    (local i32 i32 i32)
    block  ;; label = @1
      i32.const 0
      i32.load offset=1049320
      local.tee 1
      local.get 0
      i32.add
      local.tee 2
      i32.const 0
      i32.load offset=1049324
      i32.le_u
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 0
        i32.const 65535
        i32.add
        local.tee 2
        i32.const 16
        i32.shr_u
        memory.grow
        local.tee 1
        i32.const -1
        i32.ne
        br_if 0 (;@2;)
        i32.const 0
        return
      end
      i32.const 0
      i32.load offset=1049324
      local.set 3
      i32.const 0
      local.get 1
      i32.const 16
      i32.shl
      local.tee 1
      local.get 2
      i32.const -65536
      i32.and
      i32.add
      i32.store offset=1049324
      i32.const 0
      i32.load offset=1049320
      local.get 1
      local.get 1
      local.get 3
      i32.eq
      select
      local.tee 1
      local.get 0
      i32.add
      local.set 2
    end
    i32.const 0
    local.get 2
    i32.store offset=1049320
    local.get 1)
  (func (;21;) (type 2) (param i32 i32)
    (local i32 i32 i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 2
    global.set 0
    block  ;; label = @1
      local.get 0
      i32.load offset=4
      local.tee 3
      local.get 0
      i32.load offset=8
      local.tee 4
      i32.sub
      local.get 1
      i32.ge_u
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 4
        local.get 1
        i32.add
        local.tee 1
        local.get 4
        i32.lt_u
        br_if 0 (;@2;)
        local.get 3
        i32.const 1
        i32.shl
        local.tee 4
        local.get 1
        local.get 4
        local.get 1
        i32.gt_u
        select
        local.tee 1
        i32.const 8
        local.get 1
        i32.const 8
        i32.gt_u
        select
        local.tee 1
        i32.const -1
        i32.xor
        i32.const 31
        i32.shr_u
        local.set 4
        block  ;; label = @3
          block  ;; label = @4
            local.get 3
            br_if 0 (;@4;)
            local.get 2
            i32.const 0
            i32.store offset=24
            br 1 (;@3;)
          end
          local.get 2
          local.get 3
          i32.store offset=28
          local.get 2
          i32.const 1
          i32.store offset=24
          local.get 2
          local.get 0
          i32.load
          i32.store offset=20
        end
        local.get 2
        i32.const 8
        i32.add
        local.get 4
        local.get 1
        local.get 2
        i32.const 20
        i32.add
        call 22
        local.get 2
        i32.load offset=12
        local.set 3
        block  ;; label = @3
          local.get 2
          i32.load offset=8
          br_if 0 (;@3;)
          local.get 0
          local.get 1
          i32.store offset=4
          local.get 0
          local.get 3
          i32.store
          br 2 (;@1;)
        end
        local.get 3
        i32.const -2147483647
        i32.eq
        br_if 1 (;@1;)
        local.get 3
        i32.eqz
        br_if 0 (;@2;)
        local.get 2
        i32.const 16
        i32.add
        i32.load
        call 13
        unreachable
      end
      call 15
      unreachable
    end
    local.get 2
    i32.const 32
    i32.add
    global.set 0)
  (func (;22;) (type 6) (param i32 i32 i32 i32)
    (local i32 i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 4
    global.set 0
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          i32.eqz
          br_if 0 (;@3;)
          local.get 2
          i32.const -1
          i32.le_s
          br_if 1 (;@2;)
          block  ;; label = @4
            block  ;; label = @5
              local.get 3
              i32.load offset=4
              i32.eqz
              br_if 0 (;@5;)
              block  ;; label = @6
                local.get 3
                i32.const 8
                i32.add
                i32.load
                local.tee 5
                br_if 0 (;@6;)
                local.get 4
                i32.const 8
                i32.add
                local.get 2
                call 19
                local.get 4
                i32.load offset=12
                local.set 3
                local.get 4
                i32.load offset=8
                local.set 1
                br 2 (;@4;)
              end
              local.get 3
              i32.load
              local.set 3
              block  ;; label = @6
                block  ;; label = @7
                  local.get 2
                  call 20
                  local.tee 1
                  br_if 0 (;@7;)
                  i32.const 0
                  local.set 1
                  br 1 (;@6;)
                end
                local.get 1
                local.get 3
                local.get 5
                call 30
                drop
              end
              local.get 2
              local.set 3
              br 1 (;@4;)
            end
            local.get 4
            local.get 2
            call 19
            local.get 4
            i32.load offset=4
            local.set 3
            local.get 4
            i32.load
            local.set 1
          end
          block  ;; label = @4
            local.get 1
            i32.eqz
            br_if 0 (;@4;)
            local.get 0
            local.get 1
            i32.store offset=4
            local.get 0
            i32.const 8
            i32.add
            local.get 3
            i32.store
            i32.const 0
            local.set 1
            br 3 (;@1;)
          end
          i32.const 1
          local.set 1
          local.get 0
          i32.const 1
          i32.store offset=4
          local.get 0
          i32.const 8
          i32.add
          local.get 2
          i32.store
          br 2 (;@1;)
        end
        local.get 0
        i32.const 0
        i32.store offset=4
        local.get 0
        i32.const 8
        i32.add
        local.get 2
        i32.store
        i32.const 1
        local.set 1
        br 1 (;@1;)
      end
      local.get 0
      i32.const 0
      i32.store offset=4
      i32.const 1
      local.set 1
    end
    local.get 0
    local.get 1
    i32.store
    local.get 4
    i32.const 16
    i32.add
    global.set 0)
  (func (;23;) (type 2) (param i32 i32)
    local.get 0
    i64.const 568815540544143123
    i64.store offset=8
    local.get 0
    i64.const 5657071353825360256
    i64.store)
  (func (;24;) (type 2) (param i32 i32)
    (local i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 2
    global.set 0
    local.get 2
    local.get 0
    i32.store
    local.get 2
    local.get 1
    i32.store offset=4
    local.get 2
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 2
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 1
    i32.store
    local.get 2
    i32.const 2
    i32.store offset=12
    local.get 2
    i32.const 1049064
    i32.store offset=8
    local.get 2
    i32.const 1
    i32.store offset=36
    local.get 2
    local.get 2
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 2
    local.get 2
    i32.const 4
    i32.add
    i32.store offset=40
    local.get 2
    local.get 2
    i32.store offset=32
    local.get 2
    i32.const 8
    i32.add
    i32.const 1049300
    call 16
    unreachable)
  (func (;25;) (type 2) (param i32 i32)
    (local i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 2
    global.set 0
    local.get 2
    local.get 1
    i32.store offset=4
    local.get 2
    local.get 0
    i32.store
    local.get 2
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 2
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 1
    i32.store
    local.get 2
    i32.const 2
    i32.store offset=12
    local.get 2
    i32.const 1048796
    i32.store offset=8
    local.get 2
    i32.const 1
    i32.store offset=36
    local.get 2
    local.get 2
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 2
    local.get 2
    i32.store offset=40
    local.get 2
    local.get 2
    i32.const 4
    i32.add
    i32.store offset=32
    local.get 2
    i32.const 8
    i32.add
    i32.const 1049244
    call 16
    unreachable)
  (func (;26;) (type 8) (param i32 i32 i32 i32) (result i32)
    (local i32)
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 2
          i32.const 1114112
          i32.eq
          br_if 0 (;@3;)
          i32.const 1
          local.set 4
          local.get 0
          local.get 2
          local.get 1
          i32.load offset=16
          call_indirect (type 1)
          br_if 1 (;@2;)
        end
        local.get 3
        br_if 1 (;@1;)
        i32.const 0
        local.set 4
      end
      local.get 4
      return
    end
    local.get 0
    local.get 3
    i32.const 0
    local.get 1
    i32.load offset=12
    call_indirect (type 0))
  (func (;27;) (type 3) (param i32)
    (local i32)
    global.get 0
    i32.const 48
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    i32.const 4
    i32.store
    local.get 1
    local.get 0
    i32.store offset=4
    local.get 1
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 1
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 1
    i32.store
    local.get 1
    i32.const 2
    i32.store offset=12
    local.get 1
    i32.const 1049096
    i32.store offset=8
    local.get 1
    i32.const 1
    i32.store offset=36
    local.get 1
    local.get 1
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 1
    local.get 1
    i32.const 4
    i32.add
    i32.store offset=40
    local.get 1
    local.get 1
    i32.store offset=32
    local.get 1
    i32.const 8
    i32.add
    i32.const 1049208
    call 16
    unreachable)
  (func (;28;) (type 0) (param i32 i32 i32) (result i32)
    (local i32)
    global.get 0
    i32.const 32
    i32.sub
    local.tee 3
    global.set 0
    block  ;; label = @1
      local.get 2
      local.get 1
      i32.add
      local.get 0
      i32.gt_u
      br_if 0 (;@1;)
      local.get 3
      i32.const 32
      i32.add
      global.set 0
      local.get 1
      return
    end
    local.get 3
    i32.const 20
    i32.add
    i64.const 0
    i64.store align=4
    local.get 3
    i32.const 1
    i32.store offset=12
    local.get 3
    i32.const 1049276
    i32.store offset=8
    local.get 3
    i32.const 1049112
    i32.store offset=16
    local.get 3
    i32.const 8
    i32.add
    i32.const 1049284
    call 16
    unreachable)
  (func (;29;) (type 0) (param i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32)
    block  ;; label = @1
      block  ;; label = @2
        local.get 2
        i32.const 16
        i32.ge_u
        br_if 0 (;@2;)
        local.get 0
        local.set 3
        br 1 (;@1;)
      end
      local.get 0
      i32.const 0
      local.get 0
      i32.sub
      i32.const 3
      i32.and
      local.tee 4
      i32.add
      local.set 5
      block  ;; label = @2
        local.get 4
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        local.set 3
        local.get 1
        local.set 6
        loop  ;; label = @3
          local.get 3
          local.get 6
          i32.load8_u
          i32.store8
          local.get 6
          i32.const 1
          i32.add
          local.set 6
          local.get 3
          i32.const 1
          i32.add
          local.tee 3
          local.get 5
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 5
      local.get 2
      local.get 4
      i32.sub
      local.tee 7
      i32.const -4
      i32.and
      local.tee 8
      i32.add
      local.set 3
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          local.get 4
          i32.add
          local.tee 9
          i32.const 3
          i32.and
          i32.eqz
          br_if 0 (;@3;)
          local.get 8
          i32.const 1
          i32.lt_s
          br_if 1 (;@2;)
          local.get 9
          i32.const 3
          i32.shl
          local.tee 6
          i32.const 24
          i32.and
          local.set 2
          local.get 9
          i32.const -4
          i32.and
          local.tee 10
          i32.const 4
          i32.add
          local.set 1
          i32.const 0
          local.get 6
          i32.sub
          i32.const 24
          i32.and
          local.set 4
          local.get 10
          i32.load
          local.set 6
          loop  ;; label = @4
            local.get 5
            local.get 6
            local.get 2
            i32.shr_u
            local.get 1
            i32.load
            local.tee 6
            local.get 4
            i32.shl
            i32.or
            i32.store
            local.get 1
            i32.const 4
            i32.add
            local.set 1
            local.get 5
            i32.const 4
            i32.add
            local.tee 5
            local.get 3
            i32.lt_u
            br_if 0 (;@4;)
            br 2 (;@2;)
          end
        end
        local.get 8
        i32.const 1
        i32.lt_s
        br_if 0 (;@2;)
        local.get 9
        local.set 1
        loop  ;; label = @3
          local.get 5
          local.get 1
          i32.load
          i32.store
          local.get 1
          i32.const 4
          i32.add
          local.set 1
          local.get 5
          i32.const 4
          i32.add
          local.tee 5
          local.get 3
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 7
      i32.const 3
      i32.and
      local.set 2
      local.get 9
      local.get 8
      i32.add
      local.set 1
    end
    block  ;; label = @1
      local.get 2
      i32.eqz
      br_if 0 (;@1;)
      local.get 3
      local.get 2
      i32.add
      local.set 5
      loop  ;; label = @2
        local.get 3
        local.get 1
        i32.load8_u
        i32.store8
        local.get 1
        i32.const 1
        i32.add
        local.set 1
        local.get 3
        i32.const 1
        i32.add
        local.tee 3
        local.get 5
        i32.lt_u
        br_if 0 (;@2;)
      end
    end
    local.get 0)
  (func (;30;) (type 0) (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call 29)
  (table (;0;) 2 2 funcref)
  (memory (;0;) 17)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1049328))
  (global (;2;) i32 (i32.const 1049328))
  (export "memory" (memory 0))
  (export "deploy" (func 4))
  (export "main" (func 5))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (elem (;0;) (i32.const 1) func 17)
  (data (;0;) (i32.const 1048576) "Hello, Storage\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01library/alloc/src/raw_vec.rscapacity overflow\00J\00\10\00\11\00\00\00.\00\10\00\1c\00\00\00\17\02\00\00\05\00\00\00memory allocation of  bytes failed\00\00t\00\10\00\15\00\00\00\89\00\10\00\0d\00\00\00index out of bounds: the len is  but the index is \00\00\a8\00\10\00 \00\00\00\c8\00\10\00\12\00\00\0000010203040506070809101112131415161718192021222324252627282930313233343536373839404142434445464748495051525354555657585960616263646566676869707172737475767778798081828384858687888990919293949596979899range start index  out of range for slice of length \b4\01\10\00\12\00\00\00\c6\01\10\00\22\00\00\00range end index \f8\01\10\00\10\00\00\00\c6\01\10\00\22\00\00\00/Users/dmitry/.cargo/registry/src/index.crates.io-6f17d22bba15001f/byteorder-1.5.0/src/lib.rs\00\00\00\18\02\10\00]\00\00\00z\08\00\00\0c\00\00\00codec/src/buffer.rs\00\88\02\10\00\13\00\00\00%\00\00\00\14\00\00\00header overflow\00\ac\02\10\00\0f\00\00\00\88\02\10\00\13\00\00\00A\00\00\00\0d\00\00\00\88\02\10\00\13\00\00\00+\00\00\00\05\00\00\00"))
