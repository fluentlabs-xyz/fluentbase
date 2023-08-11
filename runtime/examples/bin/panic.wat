(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func (param i32)))
  (type (;2;) (func))
  (import "env" "_evm_return" (func $_evm_return (type 0)))
  (import "env" "_sys_halt" (func $_sys_halt (type 1)))
  (func $main (type 2)
    (local i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 20
    i32.add
    i64.const 0
    i64.store align=4
    local.get 0
    i32.const 1
    i32.store offset=12
    local.get 0
    i32.const 1048596
    i32.store offset=8
    local.get 0
    i32.const 1048604
    i32.store offset=16
    local.get 0
    i32.const 8
    i32.add
    call $_ZN4core9panicking9panic_fmt17h92e39e63ca4b2ca4E
    unreachable)
  (func $_ZN4core9panicking9panic_fmt17h92e39e63ca4b2ca4E (type 1) (param i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    i32.const 1048604
    call $_ZN36_$LT$T$u20$as$u20$core..any..Any$GT$7type_id17hfc845d9c30dd1909E
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
      call $_evm_return
    end
    i32.const 1
    call $_sys_halt
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func $_ZN36_$LT$T$u20$as$u20$core..any..Any$GT$7type_id17hfc845d9c30dd1909E (type 0) (param i32 i32)
    local.get 0
    i64.const -4375259500804815539
    i64.store offset=8
    local.get 0
    i64.const 1135650936325378667
    i64.store)
  (memory (;0;) 17)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048604))
  (global (;2;) i32 (i32.const 1048608))
  (export "memory" (memory 0))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data $.rodata (i32.const 1048576) "its time to panic\00\00\00\00\00\10\00\11\00\00\00"))
