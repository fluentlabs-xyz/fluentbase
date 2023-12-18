(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (func $deploy (type 1))
  (func $main (type 1)
    i32.const 65536
    i32.const 12
    call $_sys_write)
  (memory (;0;) 2)
  (global $__stack_pointer (mut i32) (i32.const 65536))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy))
  (export "main" (func $main))
  (data $.rodata (i32.const 65536) "Hello, World"))
