(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (type (;2;) (func (param i32 i32)))
  (func $arithmetic_add (type 0)
    (local i32 i64 i32 i64 i64 i64)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h9812042dca91635cE
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h9812042dca91635cE
    local.get 0
    i32.const 64
    i32.add
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h61eaa12b08cc22fcE
    local.get 0
    i32.const 96
    i32.add
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h61eaa12b08cc22fcE
    i32.const 0
    i32.const 0
    i64.load offset=500
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 1
    i64.store offset=500
    i32.const 524
    local.get 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    local.get 0
    i64.load offset=96
    local.tee 1
    i64.const 4294967295
    i64.and
    local.get 0
    i64.load offset=64
    local.tee 3
    i64.const 4294967295
    i64.and
    i64.add
    local.tee 4
    i64.const 56
    i64.shl
    local.get 4
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 1
    i64.const 32
    i64.shr_u
    local.get 3
    i64.const 32
    i64.shr_u
    i64.add
    local.get 4
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 1
    i64.const 32
    i64.shl
    local.tee 3
    local.get 4
    i64.const 4294967295
    i64.and
    i64.or
    local.tee 4
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 4
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 1
    i64.const 24
    i64.shl
    i64.const 4278190080
    i64.and
    local.get 1
    i64.const 8
    i64.shl
    i64.const 16711680
    i64.and
    i64.or
    local.get 1
    i64.const 8
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 3
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 516
    local.get 2
    i32.sub
    local.get 0
    i64.load offset=104
    local.tee 3
    i64.const 4294967295
    i64.and
    local.get 0
    i64.load offset=72
    local.tee 5
    i64.const 4294967295
    i64.and
    i64.add
    local.get 1
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 4
    i64.const 56
    i64.shl
    local.get 4
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 3
    i64.const 32
    i64.shr_u
    local.get 5
    i64.const 32
    i64.shr_u
    i64.add
    local.get 4
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 1
    i64.const 32
    i64.shl
    local.tee 3
    local.get 4
    i64.const 4294967295
    i64.and
    i64.or
    local.tee 4
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 4
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 1
    i64.const 24
    i64.shl
    i64.const 4278190080
    i64.and
    local.get 1
    i64.const 8
    i64.shl
    i64.const 16711680
    i64.and
    i64.or
    local.get 1
    i64.const 8
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 3
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 508
    local.get 2
    i32.sub
    local.get 0
    i64.load offset=112
    local.tee 3
    i64.const 4294967295
    i64.and
    local.get 0
    i64.load offset=80
    local.tee 5
    i64.const 4294967295
    i64.and
    i64.add
    local.get 1
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 4
    i64.const 56
    i64.shl
    local.get 4
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 3
    i64.const 32
    i64.shr_u
    local.get 5
    i64.const 32
    i64.shr_u
    i64.add
    local.get 4
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 1
    i64.const 32
    i64.shl
    local.tee 3
    local.get 4
    i64.const 4294967295
    i64.and
    i64.or
    local.tee 4
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 4
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 1
    i64.const 24
    i64.shl
    i64.const 4278190080
    i64.and
    local.get 1
    i64.const 8
    i64.shl
    i64.const 16711680
    i64.and
    i64.or
    local.get 1
    i64.const 8
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 3
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 500
    local.get 2
    i32.sub
    local.get 0
    i64.load offset=120
    local.tee 4
    i64.const 4294967295
    i64.and
    local.get 0
    i64.load offset=88
    local.tee 3
    i64.const 4294967295
    i64.and
    i64.add
    local.get 4
    local.get 3
    i64.const -4294967296
    i64.and
    i64.add
    i64.const -4294967296
    i64.and
    i64.add
    local.get 1
    i64.const 32
    i64.shr_u
    i64.add
    local.tee 1
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
    i32.const 128
    i32.add
    global.set $__stack_pointer)
  (func $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h9812042dca91635cE (type 1) (param i32)
    (local i64 i32)
    local.get 0
    i32.const 500
    i32.const 0
    i64.load offset=500
    local.tee 1
    i32.wrap_i64
    local.tee 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    i32.const 508
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    i32.const 516
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 24
    i32.add
    i32.const 524
    local.get 2
    i32.sub
    i64.load align=1
    i64.store align=1
    i32.const 0
    local.get 1
    i64.const 32
    i64.shl
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    i64.store offset=500)
  (func $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h61eaa12b08cc22fcE (type 2) (param i32 i32)
    (local i64)
    local.get 0
    local.get 1
    i64.load align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=24
    local.get 0
    local.get 1
    i64.load offset=8 align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=16
    local.get 0
    local.get 1
    i64.load offset=16 align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store offset=8
    local.get 0
    local.get 1
    i64.load offset=24 align=1
    local.tee 2
    i64.const 56
    i64.shl
    local.get 2
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 2
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 2
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 2
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 2
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 2
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 2
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_add" (func $arithmetic_add))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
