(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32)))
  (import "env" "_sys_read" (func $_sys_read (type 0)))
  (func $system_caller (type 1) (param i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    i32.const 0
    local.set 2
    local.get 1
    i32.const 16
    i32.add
    i32.const 0
    i32.store
    local.get 1
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i64.const 0
    i64.store
    local.get 1
    i32.const 28
    i32.const 20
    call $_sys_read
    drop
    local.get 1
    i32.const 24
    i32.add
    i32.const 16
    i32.add
    i32.const 0
    i32.store
    local.get 1
    i32.const 24
    i32.add
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 1
    i64.const 0
    i64.store offset=24
    block  ;; label = @1
      loop  ;; label = @2
        local.get 2
        i32.const 20
        i32.eq
        br_if 1 (;@1;)
        local.get 1
        i32.const 24
        i32.add
        local.get 2
        i32.add
        local.get 1
        local.get 2
        i32.add
        i32.load8_u
        i32.store8
        local.get 2
        i32.const 1
        i32.add
        local.set 2
        br 0 (;@2;)
      end
    end
    local.get 0
    local.get 1
    i64.load offset=24
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    local.get 1
    i32.const 24
    i32.add
    i32.const 16
    i32.add
    i32.load
    i32.store align=1
    local.get 0
    i32.const 8
    i32.add
    local.get 1
    i32.const 24
    i32.add
    i32.const 8
    i32.add
    i64.load
    i64.store align=1
    local.get 1
    i32.const 48
    i32.add
    global.set $__stack_pointer)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "system_caller" (func $system_caller))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
