(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func (param i32)))
  (type (;4;) (func))
  (import "env" "_sys_read" (func (;0;) (type 0)))
  (import "env" "_ecc_secp256k1_verify" (func (;1;) (type 1)))
  (import "env" "_sys_write" (func (;2;) (type 2)))
  (import "env" "_sys_halt" (func (;3;) (type 3)))
  (func (;4;) (type 4))
  (func (;5;) (type 4)
    (local i32)
    global.get 0
    i32.const 160
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
    local.get 0
    i32.const 0
    i32.const 32
    call 0
    drop
    local.get 0
    i32.const 38
    i32.add
    i32.const 0
    i32.const 64
    call 9
    drop
    local.get 0
    i32.const 38
    i32.add
    i32.const 32
    i32.const 64
    call 0
    drop
    local.get 0
    i32.const 0
    i32.store8 offset=102
    local.get 0
    i32.const 102
    i32.add
    i32.const 96
    i32.const 1
    call 0
    drop
    local.get 0
    i32.const 103
    i32.add
    i32.const 0
    i32.const 33
    call 9
    drop
    local.get 0
    i32.const 103
    i32.add
    i32.const 97
    i32.const 33
    call 0
    drop
    block  ;; label = @1
      local.get 0
      i32.const 32
      local.get 0
      i32.const 38
      i32.add
      i32.const 64
      local.get 0
      i32.const 103
      i32.add
      i32.const 33
      local.get 0
      i32.load8_u offset=102
      call 1
      br_if 0 (;@1;)
      local.get 0
      i32.const 148
      i32.add
      i64.const 0
      i64.store align=4
      local.get 0
      i32.const 1
      i32.store offset=140
      local.get 0
      i32.const 1048596
      i32.store offset=136
      local.get 0
      i32.const 1048604
      i32.store offset=144
      local.get 0
      i32.const 136
      i32.add
      call 6
      unreachable
    end
    local.get 0
    i32.const 160
    i32.add
    global.set 0)
  (func (;6;) (type 3) (param i32)
    (local i32)
    global.get 0
    i32.const 16
    i32.sub
    local.tee 1
    global.set 0
    local.get 1
    i32.const 1048604
    call 7
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
    i32.const -1
    call 3
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func (;7;) (type 2) (param i32 i32)
    local.get 0
    i64.const 568815540544143123
    i64.store offset=8
    local.get 0
    i64.const 5657071353825360256
    i64.store)
  (func (;8;) (type 0) (param i32 i32 i32) (result i32)
    (local i32 i32 i32)
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
        loop  ;; label = @3
          local.get 3
          local.get 1
          i32.store8
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
      local.tee 4
      i32.const -4
      i32.and
      local.tee 2
      i32.add
      local.set 3
      block  ;; label = @2
        local.get 2
        i32.const 1
        i32.lt_s
        br_if 0 (;@2;)
        local.get 1
        i32.const 255
        i32.and
        i32.const 16843009
        i32.mul
        local.set 2
        loop  ;; label = @3
          local.get 5
          local.get 2
          i32.store
          local.get 5
          i32.const 4
          i32.add
          local.tee 5
          local.get 3
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 4
      i32.const 3
      i32.and
      local.set 2
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
        i32.store8
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
  (func (;9;) (type 0) (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call 8)
  (memory (;0;) 17)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048604))
  (global (;2;) i32 (i32.const 1048608))
  (export "memory" (memory 0))
  (export "deploy" (func 4))
  (export "main" (func 5))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data (;0;) (i32.const 1048576) "verification failed\00\00\00\10\00\13\00\00\00"))
