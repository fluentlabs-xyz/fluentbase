(module
  (type (;0;) (func (param i32 i32 i32) (result i32)))
  (type (;1;) (func (param i32 i32)))
  (type (;2;) (func))
  (import "env" "_sys_read" (func $_sys_read (type 0)))
  (import "env" "_evm_return" (func $_evm_return (type 1)))
  (func $main (type 2)
    (local i32 i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    i32.const 0
    local.set 1
    local.get 0
    i32.const 10
    i32.add
    i32.const 0
    i32.store8
    local.get 0
    i32.const 0
    i32.store16 offset=8
    local.get 0
    i32.const 8
    i32.add
    i32.const 0
    i32.const 3
    call $_sys_read
    drop
    i32.const 0
    local.set 2
    loop  ;; label = @1
      local.get 1
      local.get 0
      i32.const 8
      i32.add
      local.get 2
      i32.add
      i32.load8_u
      i32.add
      local.set 1
      local.get 2
      i32.const 1
      i32.add
      local.tee 2
      i32.const 3
      i32.ne
      br_if 0 (;@1;)
    end
    local.get 0
    local.get 1
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
    i32.store offset=12
    local.get 0
    i32.const 12
    i32.add
    i32.const 4
    call $_evm_return
    local.get 0
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
