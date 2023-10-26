(module
  (type (;0;) (func))
  (func $main (type 0)
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func $dummy (type 0))
  (func $__wasm_call_dtors (type 0)
    call $dummy
    call $dummy)
  (func $main.command_export (type 0)
    call $main
    call $__wasm_call_dtors)
  (table (;0;) 1 1 funcref)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (export "memory" (memory 0))
  (export "main" (func $main.command_export)))
