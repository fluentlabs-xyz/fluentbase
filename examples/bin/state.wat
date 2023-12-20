(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32)))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (func $deploy (type 1)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 100
    i32.store8 offset=15
    local.get 0
    i32.const 15
    i32.add
    call $_ZN14fluentbase_sdk5rwasm3sys80_$LT$impl$u20$fluentbase_sdk..SysPlatformSDK$u20$for$u20$fluentbase_sdk..SDK$GT$9sys_write17hb5d672df77c35779E
    local.get 0
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN14fluentbase_sdk5rwasm3sys80_$LT$impl$u20$fluentbase_sdk..SysPlatformSDK$u20$for$u20$fluentbase_sdk..SDK$GT$9sys_write17hb5d672df77c35779E (type 2) (param i32)
    local.get 0
    i32.const 1
    call $_sys_write)
  (func $main (type 1)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 200
    i32.store8 offset=15
    local.get 0
    i32.const 15
    i32.add
    call $_ZN14fluentbase_sdk5rwasm3sys80_$LT$impl$u20$fluentbase_sdk..SysPlatformSDK$u20$for$u20$fluentbase_sdk..SDK$GT$9sys_write17hb5d672df77c35779E
    local.get 0
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
