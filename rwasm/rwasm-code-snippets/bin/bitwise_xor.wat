(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (func $bitwise_xor (type 0)
    (local i32 i32 i64 i32)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17haeae449af9724f8eE
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17haeae449af9724f8eE
    i32.const 0
    local.set 1
    loop  ;; label = @1
      block  ;; label = @2
        local.get 1
        i32.const 32
        i32.ne
        br_if 0 (;@2;)
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
        i32.const 524
        local.get 2
        i32.wrap_i64
        local.tee 1
        i32.sub
        local.get 0
        i32.const 56
        i32.add
        i64.load align=1
        i64.store align=1
        i32.const 516
        local.get 1
        i32.sub
        local.get 0
        i32.const 48
        i32.add
        i64.load align=1
        i64.store align=1
        i32.const 508
        local.get 1
        i32.sub
        local.get 0
        i32.const 40
        i32.add
        i64.load align=1
        i64.store align=1
        i32.const 500
        local.get 1
        i32.sub
        local.get 0
        i64.load offset=32 align=1
        i64.store align=1
        local.get 0
        i32.const 64
        i32.add
        global.set $__stack_pointer
        return
      end
      local.get 0
      i32.const 32
      i32.add
      local.get 1
      i32.add
      local.tee 3
      local.get 3
      i32.load8_u
      local.get 0
      local.get 1
      i32.add
      i32.load8_u
      i32.xor
      i32.store8
      local.get 1
      i32.const 1
      i32.add
      local.set 1
      br 0 (;@1;)
    end)
  (func $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17haeae449af9724f8eE (type 1) (param i32)
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
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "bitwise_xor" (func $bitwise_xor))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
