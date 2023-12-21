(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32)))
  (import "env" "_sys_read" (func $_sys_read (type 0)))
  (func $system_callvalue (type 1) (param i32)
    (local i32 i32 i32 i32 i32 i64 i64 i64 i64)
    global.get $__stack_pointer
    i32.const 96
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    i32.const 32
    i32.add
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i32.const 32
    i32.add
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i32.const 32
    i32.add
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i64.const 0
    i64.store offset=32
    local.get 1
    i32.const 32
    i32.add
    i32.const 108
    i32.const 32
    call $_sys_read
    drop
    local.get 1
    i32.const 64
    i32.add
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i32.const 64
    i32.add
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i32.const 64
    i32.add
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i64.const 0
    i64.store offset=64
    i32.const 0
    local.set 2
    block  ;; label = @1
      loop  ;; label = @2
        local.get 2
        i32.const 32
        i32.eq
        br_if 1 (;@1;)
        local.get 1
        i32.const 64
        i32.add
        local.get 2
        i32.add
        local.get 1
        i32.const 32
        i32.add
        local.get 2
        i32.add
        i64.load align=1
        i64.store
        local.get 2
        i32.const 8
        i32.add
        local.set 2
        br 0 (;@2;)
      end
    end
    local.get 1
    i32.const 24
    i32.add
    local.get 1
    i32.const 64
    i32.add
    i32.const 24
    i32.add
    i64.load
    i64.store
    local.get 1
    i32.const 16
    i32.add
    local.get 1
    i32.const 64
    i32.add
    i32.const 16
    i32.add
    i64.load
    i64.store
    local.get 1
    i32.const 8
    i32.add
    local.get 1
    i32.const 64
    i32.add
    i32.const 8
    i32.add
    i64.load
    i64.store
    local.get 1
    local.get 1
    i64.load offset=64
    i64.store
    local.get 1
    local.set 3
    i32.const 31
    local.set 2
    block  ;; label = @1
      loop  ;; label = @2
        local.get 2
        i32.const 15
        i32.eq
        br_if 1 (;@1;)
        local.get 3
        i32.load8_u
        local.set 4
        local.get 3
        local.get 1
        local.get 2
        i32.add
        local.tee 5
        i32.load8_u
        i32.store8
        local.get 5
        local.get 4
        i32.store8
        local.get 2
        i32.const -1
        i32.add
        local.set 2
        local.get 3
        i32.const 1
        i32.add
        local.set 3
        br 0 (;@2;)
      end
    end
    local.get 1
    i32.const 64
    i32.add
    i32.const 24
    i32.add
    local.get 1
    i32.const 24
    i32.add
    i64.load
    local.tee 6
    i64.store
    local.get 1
    i32.const 64
    i32.add
    i32.const 16
    i32.add
    local.get 1
    i32.const 16
    i32.add
    i64.load
    local.tee 7
    i64.store
    local.get 1
    i32.const 64
    i32.add
    i32.const 8
    i32.add
    local.get 1
    i32.const 8
    i32.add
    i64.load
    local.tee 8
    i64.store
    local.get 1
    local.get 1
    i64.load
    local.tee 9
    i64.store offset=64
    local.get 0
    i32.const 24
    i32.add
    local.get 6
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    local.get 7
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    local.get 8
    i64.store align=1
    local.get 0
    local.get 9
    i64.store align=1
    local.get 1
    i32.const 96
    i32.add
    global.set $__stack_pointer)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "system_callvalue" (func $system_callvalue))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
