(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func))
  (import "env" "_sys_read" (func $_sys_read (type 0)))
  (func $host_gaslimit (type 1)
    (local i32 i64 i64 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i64.const 0
    i64.store offset=8
    local.get 0
    i32.const 8
    i32.add
    i32.const 216
    i32.const 8
    call $_sys_read
    drop
    local.get 0
    i64.load offset=8
    local.set 1
    i32.const 0
    i32.const 0
    i64.load offset=500
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 2
    i64.store offset=500
    i32.const 516
    local.get 2
    i32.wrap_i64
    local.tee 3
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
    i32.const 524
    local.get 3
    i32.sub
    local.get 1
    i64.const 56
    i64.shl
    local.get 1
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 1
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 1
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 1
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 1
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 1
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 1
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "host_gaslimit" (func $host_gaslimit))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
