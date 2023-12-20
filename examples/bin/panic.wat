(module
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func (param i32)))
  (type (;2;) (func))
  (import "env" "_sys_write" (func $_sys_write (type 0)))
  (import "env" "_sys_halt" (func $_sys_halt (type 1)))
  (func $deploy (type 2))
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
    i32.const 1048592
    i32.store offset=8
    local.get 0
    i32.const 1048600
    i32.store offset=16
    local.get 0
    i32.const 8
    i32.add
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE (type 1) (param i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    i32.const 1048600
    call $_ZN36_$LT$T$u20$as$u20$core..any..Any$GT$7type_id17hed637ffe26dba6a3E
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
      call $_sys_write
    end
    i32.const -71
    call $_sys_halt
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func $_ZN36_$LT$T$u20$as$u20$core..any..Any$GT$7type_id17hed637ffe26dba6a3E (type 0) (param i32 i32)
    local.get 0
    i64.const 568815540544143123
    i64.store offset=8
    local.get 0
    i64.const 5657071353825360256
    i64.store)
  (memory (;0;) 17)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048600))
  (global (;2;) i32 (i32.const 1048608))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data $.rodata (i32.const 1048576) "it is panic time\00\00\10\00\10\00\00\00"))
