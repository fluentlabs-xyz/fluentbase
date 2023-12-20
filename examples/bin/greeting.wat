(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (func $deploy (type 1))
  (func $main (type 1)
    i32.const 1048576
    i32.const 12
    call $_sys_write)
  (memory (;0;) 17)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048588))
  (global (;2;) i32 (i32.const 1048592))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data $.rodata (i32.const 1048576) "Hello, World"))
