(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (param i32 i32 i32) (result i32)))
  (type (;2;) (func (param i32 i32) (result i32)))
  (type (;3;) (func (param i32 i32 i32 i32)))
  (type (;4;) (func (param i32) (result i32)))
  (type (;5;) (func (param i32 i32)))
  (type (;6;) (func (param i32)))
  (type (;7;) (func))
  (type (;8;) (func (param i32 i32 i32 i32 i32)))
  (type (;9;) (func (param i32 i32 i32 i32) (result i32)))
  (type (;10;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (type (;11;) (func (param i32 i32 i32 i32 i32) (result i32)))
  (type (;12;) (func (param i64 i32) (result i32)))
  (type (;13;) (func (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (type (;14;) (func (param i32 i64 i64 i64 i64)))
  (import "env" "_crypto_keccak256" (func $_crypto_keccak256 (type 0)))
  (import "env" "_sys_write" (func $_sys_write (type 5)))
  (import "env" "_sys_halt" (func $_sys_halt (type 6)))
  (import "env" "_evm_sstore" (func $_evm_sstore (type 5)))
  (import "env" "_sys_read" (func $_sys_read (type 1)))
  (import "env" "_evm_sload" (func $_evm_sload (type 5)))
  (import "env" "_evm_caller" (func $_evm_caller (type 6)))
  (func $_ZN15alloy_sol_types3abi7encoder7Encoder10into_bytes17hfd7e1de4bb961730E (type 5) (param i32 i32)
    local.get 0
    local.get 1
    i32.load
    i32.store
    local.get 0
    local.get 1
    i32.load offset=8
    i32.const 5
    i32.shl
    i32.store offset=8
    local.get 0
    local.get 1
    i32.load offset=4
    i32.const 5
    i32.shl
    i32.store offset=4
    block  ;; label = @1
      local.get 1
      i32.const 16
      i32.add
      i32.load
      local.tee 0
      i32.eqz
      br_if 0 (;@1;)
      local.get 1
      i32.load offset=12
      local.get 0
      i32.const 2
      i32.shl
      call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$7dealloc17h96238585fd016669E
    end)
  (func $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$7dealloc17h96238585fd016669E (type 5) (param i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      local.get 0
      i32.eqz
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 1
        i32.const 3
        i32.add
        i32.const 2
        i32.shr_u
        i32.const -1
        i32.add
        local.tee 1
        i32.const 256
        i32.lt_u
        br_if 0 (;@2;)
        local.get 2
        i32.const 0
        i32.load offset=1054484
        i32.store offset=8
        local.get 0
        local.get 2
        i32.const 8
        i32.add
        i32.const 1049804
        i32.const 1
        call $_ZN9wee_alloc8WeeAlloc12dealloc_impl28_$u7b$$u7b$closure$u7d$$u7d$17h9341e162cd3d43dcE
        i32.const 0
        local.get 2
        i32.load offset=8
        i32.store offset=1054484
        br 1 (;@1;)
      end
      local.get 2
      i32.const 1054484
      i32.store offset=4
      local.get 2
      local.get 1
      i32.const 2
      i32.shl
      i32.const 1053460
      i32.add
      local.tee 1
      i32.load
      i32.store offset=12
      local.get 0
      local.get 2
      i32.const 12
      i32.add
      local.get 2
      i32.const 4
      i32.add
      i32.const 2
      call $_ZN9wee_alloc8WeeAlloc12dealloc_impl28_$u7b$$u7b$closure$u7d$$u7d$17h9341e162cd3d43dcE
      local.get 1
      local.get 2
      i32.load offset=12
      i32.store
    end
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN15alloy_sol_types3abi7encoder7Encoder13with_capacity17hbd52833ff1794d00E (type 5) (param i32 i32)
    (local i32 i32)
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          i32.const 67108863
          i32.gt_u
          br_if 0 (;@3;)
          local.get 1
          i32.const 5
          i32.shl
          local.tee 2
          i32.const -1
          i32.le_s
          br_if 0 (;@3;)
          i32.const 1
          local.get 2
          call $_ZN63_$LT$alloc..alloc..Global$u20$as$u20$core..alloc..Allocator$GT$8allocate17h15badb2e48cc057cE
          local.tee 3
          i32.eqz
          br_if 1 (;@2;)
          i32.const 4
          i32.const 16
          call $_ZN63_$LT$alloc..alloc..Global$u20$as$u20$core..alloc..Allocator$GT$8allocate17h15badb2e48cc057cE
          local.tee 2
          i32.eqz
          br_if 2 (;@1;)
          local.get 0
          local.get 2
          i32.store offset=12
          local.get 0
          i32.const 0
          i32.store offset=8
          local.get 0
          local.get 1
          i32.store offset=4
          local.get 0
          local.get 3
          i32.store
          local.get 0
          i32.const 16
          i32.add
          i64.const 4
          i64.store align=4
          return
        end
        call $_ZN5alloc7raw_vec17capacity_overflow17h26cdf55d7b744af0E
        unreachable
      end
      local.get 2
      call $__rust_alloc_error_handler
      unreachable
    end
    i32.const 16
    call $__rust_alloc_error_handler
    unreachable)
  (func $_ZN63_$LT$alloc..alloc..Global$u20$as$u20$core..alloc..Allocator$GT$8allocate17h15badb2e48cc057cE (type 2) (param i32 i32) (result i32)
    block  ;; label = @1
      local.get 1
      i32.eqz
      br_if 0 (;@1;)
      i32.const 0
      i32.load8_u offset=1053456
      drop
      local.get 0
      local.get 1
      call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$5alloc17h940356afe3ab03a7E
      local.set 0
    end
    local.get 0)
  (func $_ZN5alloc7raw_vec17capacity_overflow17h26cdf55d7b744af0E (type 7)
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
    i32.const 1049132
    i32.store offset=8
    local.get 0
    i32.const 1049804
    i32.store offset=16
    local.get 0
    i32.const 8
    i32.add
    i32.const 1049140
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $__rust_alloc_error_handler (type 6) (param i32)
    local.get 0
    call $__rdl_oom
    unreachable)
  (func $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17ha702285a765a087fE (type 5) (param i32 i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 2
    i32.const 32
    i32.add
    i32.const 8
    i32.add
    local.tee 3
    i32.const 0
    i32.store
    local.get 2
    i32.const 52
    i32.add
    local.get 1
    i32.const 8
    i32.add
    i64.load align=1
    i64.store align=4
    local.get 2
    i32.const 60
    i32.add
    local.get 1
    i32.const 16
    i32.add
    i32.load align=1
    i32.store
    local.get 2
    local.get 1
    i64.load align=1
    i64.store offset=44 align=4
    local.get 2
    i32.const 8
    i32.add
    local.get 3
    i64.load
    i64.store
    local.get 2
    i32.const 16
    i32.add
    local.get 2
    i32.const 32
    i32.add
    i32.const 16
    i32.add
    i64.load
    i64.store
    local.get 2
    i32.const 24
    i32.add
    local.get 2
    i32.const 32
    i32.add
    i32.const 24
    i32.add
    i64.load
    i64.store
    local.get 2
    i64.const 0
    i64.store
    local.get 2
    i32.const 32
    i32.add
    i32.const 1
    call $_ZN15alloy_sol_types3abi7encoder7Encoder13with_capacity17hbd52833ff1794d00E
    local.get 2
    local.get 2
    i32.const 32
    i32.add
    call $_ZN71_$LT$$LP$T1$C$$RP$$u20$as$u20$alloy_sol_types..abi..token..TokenSeq$GT$15encode_sequence17he3dde54bb68d491dE
    local.get 0
    local.get 2
    i32.const 32
    i32.add
    call $_ZN15alloy_sol_types3abi7encoder7Encoder10into_bytes17hfd7e1de4bb961730E
    local.get 2
    i32.const 64
    i32.add
    global.set $__stack_pointer)
  (func $_ZN71_$LT$$LP$T1$C$$RP$$u20$as$u20$alloy_sol_types..abi..token..TokenSeq$GT$15encode_sequence17he3dde54bb68d491dE (type 5) (param i32 i32)
    local.get 1
    call $_ZN15alloy_sol_types3abi7encoder7Encoder11push_offset17h567292d1e5ce7129E
    local.get 1
    local.get 0
    call $_ZN15alloy_sol_types3abi7encoder7Encoder11append_word17hfe306cdba9a9f0f3E
    block  ;; label = @1
      local.get 1
      i32.const 20
      i32.add
      i32.load
      local.tee 0
      i32.eqz
      br_if 0 (;@1;)
      local.get 1
      local.get 0
      i32.const -1
      i32.add
      i32.store offset=20
    end)
  (func $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17hdea3fda248de7d34E (type 5) (param i32 i32)
    (local i32 i32 i32 i32 i64)
    global.get $__stack_pointer
    i32.const 96
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 2
    i32.const 64
    i32.add
    i32.const 24
    i32.add
    local.get 1
    i32.const 24
    i32.add
    i64.load align=1
    i64.store
    local.get 2
    i32.const 64
    i32.add
    i32.const 16
    i32.add
    local.get 1
    i32.const 16
    i32.add
    i64.load align=1
    i64.store
    local.get 2
    i32.const 64
    i32.add
    i32.const 8
    i32.add
    local.get 1
    i32.const 8
    i32.add
    i64.load align=1
    i64.store
    local.get 2
    local.get 1
    i64.load align=1
    i64.store offset=64
    local.get 2
    i32.const 64
    i32.add
    local.set 3
    i32.const 31
    local.set 1
    block  ;; label = @1
      loop  ;; label = @2
        local.get 1
        i32.const 15
        i32.eq
        br_if 1 (;@1;)
        local.get 3
        i32.load8_u
        local.set 4
        local.get 3
        local.get 2
        i32.const 64
        i32.add
        local.get 1
        i32.add
        local.tee 5
        i32.load8_u
        i32.store8
        local.get 5
        local.get 4
        i32.store8
        local.get 1
        i32.const -1
        i32.add
        local.set 1
        local.get 3
        i32.const 1
        i32.add
        local.set 3
        br 0 (;@2;)
      end
    end
    local.get 2
    i32.const 32
    i32.add
    i32.const 24
    i32.add
    local.get 2
    i32.const 64
    i32.add
    i32.const 24
    i32.add
    i64.load
    local.tee 6
    i64.store
    local.get 2
    i32.const 8
    i32.add
    local.get 2
    i32.const 64
    i32.add
    i32.const 8
    i32.add
    i64.load
    i64.store
    local.get 2
    i32.const 16
    i32.add
    local.get 2
    i32.const 64
    i32.add
    i32.const 16
    i32.add
    i64.load
    i64.store
    local.get 2
    i32.const 24
    i32.add
    local.get 6
    i64.store
    local.get 2
    local.get 2
    i64.load offset=64
    i64.store
    local.get 2
    i32.const 64
    i32.add
    i32.const 1
    call $_ZN15alloy_sol_types3abi7encoder7Encoder13with_capacity17hbd52833ff1794d00E
    local.get 2
    local.get 2
    i32.const 64
    i32.add
    call $_ZN71_$LT$$LP$T1$C$$RP$$u20$as$u20$alloy_sol_types..abi..token..TokenSeq$GT$15encode_sequence17he3dde54bb68d491dE
    local.get 0
    local.get 2
    i32.const 64
    i32.add
    call $_ZN15alloy_sol_types3abi7encoder7Encoder10into_bytes17hfd7e1de4bb961730E
    local.get 2
    i32.const 96
    i32.add
    global.set $__stack_pointer)
  (func $_ZN16alloy_primitives4bits5fixed19FixedBytes$LT$_$GT$7is_zero17h91cdd6d0bac06c3eE (type 4) (param i32) (result i32)
    local.get 0
    i32.const 1048608
    i32.const 20
    call $memcmp
    i32.eqz)
  (func $_ZN4core3ptr50drop_in_place$LT$alloc..borrow..Cow$LT$str$GT$$GT$17h925d7eff05490538E (type 5) (param i32 i32)
    block  ;; label = @1
      local.get 0
      i32.eqz
      br_if 0 (;@1;)
      local.get 0
      local.get 1
      call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
    end)
  (func $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE (type 5) (param i32 i32)
    block  ;; label = @1
      local.get 1
      i32.eqz
      br_if 0 (;@1;)
      local.get 0
      local.get 1
      call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$7dealloc17h96238585fd016669E
    end)
  (func $_ZN4core3ptr51drop_in_place$LT$alloy_sol_types..errors..Error$GT$17h005c935592b50f1bE (type 6) (param i32)
    (local i32)
    block  ;; label = @1
      local.get 0
      i32.load8_u
      local.tee 1
      i32.const 7
      i32.gt_u
      br_if 0 (;@1;)
      block  ;; label = @2
        block  ;; label = @3
          i32.const 1
          local.get 1
          i32.shl
          i32.const 222
          i32.and
          br_if 0 (;@3;)
          local.get 1
          br_if 1 (;@2;)
          local.get 0
          i32.load offset=4
          local.get 0
          i32.const 8
          i32.add
          i32.load
          call $_ZN4core3ptr50drop_in_place$LT$alloc..borrow..Cow$LT$str$GT$$GT$17h925d7eff05490538E
          local.get 0
          i32.load offset=16
          local.get 0
          i32.const 20
          i32.add
          i32.load
          call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
        end
        return
      end
      local.get 0
      i32.load offset=12
      local.tee 0
      i32.load
      local.get 0
      i32.const 4
      i32.add
      i32.load
      call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h1b2b8d6979887e6eE
      local.get 0
      i32.const 12
      i32.add
      call $_ZN4core3ptr52drop_in_place$LT$alloy_primitives..bytes_..Bytes$GT$17h2dce723588d0292bE
      local.get 0
      i32.const 28
      call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$7dealloc17h96238585fd016669E
      return
    end
    local.get 0
    i32.load offset=4
    local.get 0
    i32.const 8
    i32.add
    i32.load
    call $_ZN4core3ptr50drop_in_place$LT$alloc..borrow..Cow$LT$str$GT$$GT$17h925d7eff05490538E)
  (func $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h1b2b8d6979887e6eE (type 5) (param i32 i32)
    block  ;; label = @1
      local.get 1
      i32.eqz
      br_if 0 (;@1;)
      local.get 0
      local.get 1
      i32.const 5
      i32.shl
      call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$7dealloc17h96238585fd016669E
    end)
  (func $_ZN4core3ptr52drop_in_place$LT$alloy_primitives..bytes_..Bytes$GT$17h2dce723588d0292bE (type 6) (param i32)
    local.get 0
    i32.const 12
    i32.add
    local.get 0
    i32.load offset=4
    local.get 0
    i32.load offset=8
    local.get 0
    i32.load
    i32.load offset=8
    call_indirect (type 0))
  (func $_ZN5bytes5bytes11static_drop17h78a6f2685218f0d1E (type 0) (param i32 i32 i32))
  (func $_ZN91_$LT$alloy_primitives..bytes_..Bytes$u20$as$u20$alloy_sol_types..types..value..SolValue$GT$10abi_encode17hfee42a1e9ae0550dE (type 0) (param i32 i32 i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 2
          i32.eqz
          br_if 0 (;@3;)
          local.get 3
          i32.const 8
          i32.add
          local.get 2
          i32.const 31
          i32.add
          local.tee 4
          i32.const 5
          i32.shr_u
          local.tee 5
          i32.const 3
          i32.add
          call $_ZN15alloy_sol_types3abi7encoder7Encoder13with_capacity17hbd52833ff1794d00E
          local.get 3
          i32.const 8
          i32.add
          call $_ZN15alloy_sol_types3abi7encoder7Encoder11push_offset17h567292d1e5ce7129E
          local.get 3
          i32.const 32
          i32.add
          i32.const 8
          i32.add
          local.tee 6
          i64.const 0
          i64.store
          local.get 3
          i32.const 48
          i32.add
          local.tee 7
          i64.const 0
          i64.store
          local.get 3
          i32.const 32
          i32.add
          i32.const 24
          i32.add
          local.tee 8
          i32.const 0
          i32.store
          local.get 3
          i64.const 0
          i64.store offset=32
          local.get 3
          local.get 3
          i32.const 28
          i32.add
          local.tee 9
          i32.load
          i32.const 2
          i32.shl
          local.get 3
          i32.load offset=20
          i32.add
          i32.const -4
          i32.add
          i32.load
          local.tee 10
          i32.const 24
          i32.shl
          local.get 10
          i32.const 65280
          i32.and
          i32.const 8
          i32.shl
          i32.or
          local.get 10
          i32.const 8
          i32.shr_u
          i32.const 65280
          i32.and
          local.get 10
          i32.const 24
          i32.shr_u
          i32.or
          i32.or
          i32.store offset=60
          local.get 3
          i32.const 8
          i32.add
          local.get 3
          i32.const 32
          i32.add
          call $_ZN15alloy_sol_types3abi7encoder7Encoder11append_word17hfe306cdba9a9f0f3E
          block  ;; label = @4
            local.get 9
            i32.load
            local.tee 10
            i32.eqz
            br_if 0 (;@4;)
            local.get 10
            i32.const 2
            i32.shl
            local.get 3
            i32.load offset=20
            i32.add
            i32.const -4
            i32.add
            local.tee 10
            local.get 4
            i32.const -32
            i32.and
            local.get 10
            i32.load
            i32.add
            i32.const 32
            i32.add
            i32.store
          end
          local.get 8
          i32.const 0
          i32.store
          local.get 7
          i64.const 0
          i64.store
          local.get 6
          i64.const 0
          i64.store
          local.get 3
          i64.const 0
          i64.store offset=32
          local.get 3
          local.get 2
          i32.const 24
          i32.shl
          local.get 2
          i32.const 65280
          i32.and
          i32.const 8
          i32.shl
          i32.or
          local.get 2
          i32.const 8
          i32.shr_u
          i32.const 65280
          i32.and
          local.get 2
          i32.const 24
          i32.shr_u
          i32.or
          i32.or
          i32.store offset=60
          local.get 3
          i32.const 8
          i32.add
          local.get 3
          i32.const 32
          i32.add
          call $_ZN15alloy_sol_types3abi7encoder7Encoder11append_word17hfe306cdba9a9f0f3E
          block  ;; label = @4
            local.get 3
            i32.load offset=12
            local.get 3
            i32.load offset=16
            local.tee 10
            i32.sub
            local.get 5
            i32.ge_u
            br_if 0 (;@4;)
            local.get 3
            local.get 3
            i32.const 8
            i32.add
            local.get 10
            local.get 5
            call $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$14grow_amortized17h7e52c94f5aba65a2E
            local.get 3
            i32.load
            local.get 3
            i32.load offset=4
            call $_ZN5alloc7raw_vec14handle_reserve17h7b8a866c605ec769E
            local.get 3
            i32.load offset=16
            local.set 10
          end
          local.get 3
          local.get 10
          local.get 5
          i32.add
          i32.store offset=16
          local.get 3
          i32.load offset=8
          local.get 10
          i32.const 5
          i32.shl
          i32.add
          local.get 1
          local.get 2
          call $memcpy
          local.set 10
          block  ;; label = @4
            local.get 2
            i32.const 31
            i32.and
            local.tee 5
            i32.eqz
            br_if 0 (;@4;)
            local.get 10
            local.get 2
            i32.add
            i32.const 0
            i32.const 32
            local.get 5
            i32.sub
            call $memset
            drop
          end
          block  ;; label = @4
            local.get 3
            i32.load offset=28
            local.tee 2
            i32.eqz
            br_if 0 (;@4;)
            local.get 3
            local.get 2
            i32.const -1
            i32.add
            i32.store offset=28
          end
          local.get 0
          local.get 3
          i32.const 8
          i32.add
          call $_ZN15alloy_sol_types3abi7encoder7Encoder10into_bytes17hfd7e1de4bb961730E
          br 1 (;@2;)
        end
        i32.const 0
        i32.load8_u offset=1053456
        drop
        i32.const 1
        i32.const 64
        call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$5alloc17h940356afe3ab03a7E
        local.tee 2
        i32.eqz
        br_if 1 (;@1;)
        local.get 2
        i32.const 1048716
        i32.const 64
        call $memcpy
        local.set 2
        local.get 0
        i64.const 274877907008
        i64.store offset=4 align=4
        local.get 0
        local.get 2
        i32.store
      end
      local.get 3
      i32.const 64
      i32.add
      global.set $__stack_pointer
      return
    end
    i32.const 64
    call $__rust_alloc_error_handler
    unreachable)
  (func $_ZN15alloy_sol_types3abi7encoder7Encoder11push_offset17h567292d1e5ce7129E (type 6) (param i32)
    (local i32)
    block  ;; label = @1
      local.get 0
      i32.const 20
      i32.add
      i32.load
      local.tee 1
      local.get 0
      i32.const 16
      i32.add
      i32.load
      i32.ne
      br_if 0 (;@1;)
      local.get 0
      i32.const 12
      i32.add
      local.get 1
      call $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$16reserve_for_push17hf6d581f7796c012dE
      local.get 0
      i32.load offset=20
      local.set 1
    end
    local.get 0
    local.get 1
    i32.const 1
    i32.add
    i32.store offset=20
    local.get 0
    i32.load offset=12
    local.get 1
    i32.const 2
    i32.shl
    i32.add
    i32.const 32
    i32.store)
  (func $_ZN15alloy_sol_types3abi7encoder7Encoder11append_word17hfe306cdba9a9f0f3E (type 5) (param i32 i32)
    (local i32)
    block  ;; label = @1
      local.get 0
      i32.load offset=8
      local.tee 2
      local.get 0
      i32.load offset=4
      i32.ne
      br_if 0 (;@1;)
      local.get 0
      local.get 2
      call $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$16reserve_for_push17h51cccd5bdd1e8d04E
      local.get 0
      i32.load offset=8
      local.set 2
    end
    local.get 0
    local.get 2
    i32.const 1
    i32.add
    i32.store offset=8
    local.get 0
    i32.load
    local.get 2
    i32.const 5
    i32.shl
    i32.add
    local.tee 0
    local.get 1
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    local.get 1
    i32.const 8
    i32.add
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    local.get 1
    i32.const 16
    i32.add
    i64.load align=1
    i64.store align=1
    local.get 0
    i32.const 24
    i32.add
    local.get 1
    i32.const 24
    i32.add
    i64.load align=1
    i64.store align=1)
  (func $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$14grow_amortized17h7e52c94f5aba65a2E (type 3) (param i32 i32 i32 i32)
    (local i32 i32 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 4
    global.set $__stack_pointer
    i32.const 0
    local.set 5
    block  ;; label = @1
      local.get 2
      local.get 3
      i32.add
      local.tee 3
      local.get 2
      i32.lt_u
      br_if 0 (;@1;)
      local.get 1
      i32.load offset=4
      local.tee 2
      i32.const 1
      i32.shl
      local.tee 5
      local.get 3
      local.get 5
      local.get 3
      i32.gt_u
      select
      local.tee 3
      i32.const 4
      local.get 3
      i32.const 4
      i32.gt_u
      select
      local.tee 3
      i32.const 67108864
      i32.lt_u
      local.set 5
      local.get 3
      i32.const 5
      i32.shl
      local.set 6
      block  ;; label = @2
        block  ;; label = @3
          local.get 2
          br_if 0 (;@3;)
          local.get 4
          i32.const 0
          i32.store offset=24
          br 1 (;@2;)
        end
        local.get 4
        i32.const 1
        i32.store offset=24
        local.get 4
        local.get 2
        i32.const 5
        i32.shl
        i32.store offset=28
        local.get 4
        local.get 1
        i32.load
        i32.store offset=20
      end
      local.get 4
      i32.const 8
      i32.add
      local.get 5
      local.get 6
      local.get 4
      i32.const 20
      i32.add
      call $_ZN5alloc7raw_vec11finish_grow17h976f30d369f21f6aE
      local.get 4
      i32.load offset=12
      local.set 5
      block  ;; label = @2
        local.get 4
        i32.load offset=8
        i32.eqz
        br_if 0 (;@2;)
        local.get 4
        i32.const 16
        i32.add
        i32.load
        local.set 3
        br 1 (;@1;)
      end
      local.get 1
      local.get 3
      i32.store offset=4
      local.get 1
      local.get 5
      i32.store
      i32.const -2147483647
      local.set 5
    end
    local.get 0
    local.get 3
    i32.store offset=4
    local.get 0
    local.get 5
    i32.store
    local.get 4
    i32.const 32
    i32.add
    global.set $__stack_pointer)
  (func $_ZN5alloc7raw_vec14handle_reserve17h7b8a866c605ec769E (type 5) (param i32 i32)
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i32.const -2147483647
        i32.eq
        br_if 0 (;@2;)
        local.get 0
        i32.eqz
        br_if 1 (;@1;)
        local.get 1
        call $__rust_alloc_error_handler
        unreachable
      end
      return
    end
    call $_ZN5alloc7raw_vec17capacity_overflow17h26cdf55d7b744af0E
    unreachable)
  (func $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$5alloc17h940356afe3ab03a7E (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        i32.const 3
        i32.add
        i32.const 2
        i32.shr_u
        local.tee 1
        i32.const -1
        i32.add
        local.tee 3
        i32.const 256
        i32.lt_u
        br_if 0 (;@2;)
        local.get 2
        i32.const 0
        i32.load offset=1054484
        i32.store offset=8
        local.get 1
        local.get 0
        local.get 2
        i32.const 8
        i32.add
        i32.const 1049804
        i32.const 1053432
        call $_ZN9wee_alloc17alloc_with_refill17h36ec3c7d762c0443E
        local.set 1
        i32.const 0
        local.get 2
        i32.load offset=8
        i32.store offset=1054484
        br 1 (;@1;)
      end
      local.get 2
      i32.const 1054484
      i32.store offset=4
      local.get 2
      local.get 3
      i32.const 2
      i32.shl
      i32.const 1053460
      i32.add
      local.tee 3
      i32.load
      i32.store offset=12
      local.get 1
      local.get 0
      local.get 2
      i32.const 12
      i32.add
      local.get 2
      i32.const 4
      i32.add
      i32.const 1053408
      call $_ZN9wee_alloc17alloc_with_refill17h36ec3c7d762c0443E
      local.set 1
      local.get 3
      local.get 2
      i32.load offset=12
      i32.store
    end
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 1)
  (func $_ZN93_$LT$alloy_sol_types..abi..token..WordToken$u20$as$u20$alloy_sol_types..abi..token..Token$GT$11decode_from17ha8441e60a93aef41E (type 5) (param i32 i32)
    (local i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 1
    i32.load offset=4
    local.set 3
    local.get 1
    i32.load
    local.set 4
    local.get 1
    i32.load offset=8
    local.set 5
    i32.const 1
    local.set 6
    local.get 2
    i32.const 1
    i32.store8 offset=4
    block  ;; label = @1
      block  ;; label = @2
        local.get 5
        i32.const -33
        i32.gt_u
        br_if 0 (;@2;)
        local.get 3
        local.get 5
        i32.const 32
        i32.add
        local.tee 7
        i32.lt_u
        br_if 0 (;@2;)
        local.get 2
        i32.const 4
        i32.add
        call $_ZN4core3ptr51drop_in_place$LT$alloy_sol_types..errors..Error$GT$17h005c935592b50f1bE
        local.get 1
        local.get 7
        i32.store offset=8
        local.get 0
        local.get 4
        local.get 5
        i32.add
        local.tee 1
        i32.load16_u align=1
        i32.store16 offset=1 align=1
        local.get 0
        i32.const 8
        i32.add
        local.get 1
        i32.load offset=7 align=1
        i32.store
        local.get 0
        i32.const 12
        i32.add
        local.get 1
        i64.load offset=11 align=1
        i64.store align=1
        local.get 0
        i32.const 32
        i32.add
        local.get 1
        i32.load8_u offset=31
        i32.store8
        local.get 0
        i32.const 3
        i32.add
        local.get 1
        i32.const 2
        i32.add
        i32.load align=1
        i32.store align=1
        local.get 0
        i32.const 7
        i32.add
        local.get 1
        i32.const 6
        i32.add
        i32.load8_u
        i32.store8
        local.get 0
        i32.const 20
        i32.add
        local.get 1
        i32.const 19
        i32.add
        i64.load align=1
        i64.store align=1
        local.get 0
        i32.const 28
        i32.add
        local.get 1
        i32.const 27
        i32.add
        i32.load align=1
        i32.store align=1
        i32.const 0
        local.set 6
        br 1 (;@1;)
      end
      local.get 0
      i32.const 1
      i32.store8 offset=4
    end
    local.get 0
    local.get 6
    i32.store8
    local.get 2
    i32.const 32
    i32.add
    global.set $__stack_pointer)
  (func $_ZN18fluentbase_example5erc2019storage_mapping_key17h34d86350df1f7d79E (type 0) (param i32 i32 i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 96
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    i32.const 0
    i32.const 64
    call $memset
    local.tee 3
    i32.const 32
    i32.const 1048576
    i32.const 32
    i32.const 1048804
    call $_ZN4core5slice29_$LT$impl$u20$$u5b$T$u5d$$GT$15copy_from_slice17he60b17699e5c5b18E
    local.get 3
    i32.const 32
    i32.add
    i32.const 32
    local.get 1
    local.get 2
    i32.const 1048820
    call $_ZN4core5slice29_$LT$impl$u20$$u5b$T$u5d$$GT$15copy_from_slice17he60b17699e5c5b18E
    local.get 3
    i32.const 64
    i32.add
    i32.const 24
    i32.add
    local.tee 2
    i64.const 0
    i64.store
    local.get 3
    i32.const 64
    i32.add
    i32.const 16
    i32.add
    local.tee 1
    i64.const 0
    i64.store
    local.get 3
    i32.const 64
    i32.add
    i32.const 8
    i32.add
    local.tee 4
    i64.const 0
    i64.store
    local.get 3
    i64.const 0
    i64.store offset=64
    local.get 3
    i32.const 64
    local.get 3
    i32.const 64
    i32.add
    call $_crypto_keccak256
    local.get 0
    i32.const 24
    i32.add
    local.get 2
    i64.load
    i64.store align=1
    local.get 0
    i32.const 16
    i32.add
    local.get 1
    i64.load
    i64.store align=1
    local.get 0
    i32.const 8
    i32.add
    local.get 4
    i64.load
    i64.store align=1
    local.get 0
    local.get 3
    i64.load offset=64
    i64.store align=1
    local.get 3
    i32.const 96
    i32.add
    global.set $__stack_pointer)
  (func $_ZN4core5slice29_$LT$impl$u20$$u5b$T$u5d$$GT$15copy_from_slice17he60b17699e5c5b18E (type 8) (param i32 i32 i32 i32 i32)
    block  ;; label = @1
      local.get 1
      local.get 3
      i32.ne
      br_if 0 (;@1;)
      local.get 0
      local.get 2
      local.get 1
      call $memcpy
      drop
      return
    end
    local.get 1
    local.get 3
    local.get 4
    call $_ZN4core5slice29_$LT$impl$u20$$u5b$T$u5d$$GT$15copy_from_slice17len_mismatch_fail17h93d5590d3e243978E
    unreachable)
  (func $__rdl_oom (type 6) (param i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    local.get 0
    i32.store offset=12
    local.get 1
    i32.const 28
    i32.add
    i64.const 1
    i64.store align=4
    local.get 1
    i32.const 2
    i32.store offset=20
    local.get 1
    i32.const 1049192
    i32.store offset=16
    local.get 1
    i32.const 3
    i32.store offset=44
    local.get 1
    local.get 1
    i32.const 40
    i32.add
    i32.store offset=24
    local.get 1
    local.get 1
    i32.const 12
    i32.add
    i32.store offset=40
    local.get 1
    i32.const 16
    i32.add
    call $_ZN4core9panicking18panic_nounwind_fmt17hc0791cac263d58dbE
    unreachable)
  (func $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE (type 5) (param i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 2
    i32.const 1049804
    call $_ZN36_$LT$T$u20$as$u20$core..any..Any$GT$7type_id17hed637ffe26dba6a3E
    block  ;; label = @1
      local.get 2
      i64.load
      i64.const -4493808902380553279
      i64.xor
      local.get 2
      i32.const 8
      i32.add
      i64.load
      i64.const -163230743173927068
      i64.xor
      i64.or
      i64.const 0
      i64.ne
      br_if 0 (;@1;)
      local.get 2
      local.get 2
      call $_sys_write
    end
    i32.const -71
    call $_sys_halt
    loop  ;; label = @1
      br 0 (;@1;)
    end)
  (func $_ZN4core3fmt3num3imp52_$LT$impl$u20$core..fmt..Display$u20$for$u20$u32$GT$3fmt17he696c0e431156bceE (type 2) (param i32 i32) (result i32)
    local.get 0
    i64.load32_u
    local.get 1
    call $_ZN4core3fmt3num3imp7fmt_u6417h416abf9443aa8afdE)
  (func $_ZN4core9panicking18panic_nounwind_fmt17hc0791cac263d58dbE (type 6) (param i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    i32.const 1049804
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
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17hecf5f15880712f7eE (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 0
    i32.load
    local.set 3
    i32.const 1
    local.set 0
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        i32.load offset=20
        local.tee 4
        i32.const 1049352
        i32.const 6
        local.get 1
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        local.tee 5
        call_indirect (type 1)
        br_if 0 (;@2;)
        i32.const 1
        local.set 0
        local.get 3
        i32.load offset=4
        local.set 6
        local.get 2
        i32.const 8
        i32.add
        local.get 3
        i32.load offset=8
        local.tee 7
        i32.const 1
        i32.shl
        i32.const 2
        i32.add
        local.tee 1
        i32.const 1
        call $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$11allocate_in17hf9f36dcc15e8fce1E
        local.get 2
        i32.load offset=12
        local.set 8
        local.get 2
        i32.load offset=8
        local.tee 3
        local.get 1
        i32.const 0
        i32.const 1049304
        call $_ZN84_$LT$alloc..vec..Vec$LT$T$C$A$GT$$u20$as$u20$core..ops..index..IndexMut$LT$I$GT$$GT$9index_mut17h4b293a527584354aE
        i32.const 48
        i32.store8
        local.get 3
        local.get 1
        i32.const 1
        i32.const 1049320
        call $_ZN84_$LT$alloc..vec..Vec$LT$T$C$A$GT$$u20$as$u20$core..ops..index..IndexMut$LT$I$GT$$GT$9index_mut17h4b293a527584354aE
        i32.const 120
        i32.store8
        local.get 1
        i32.eqz
        br_if 1 (;@1;)
        local.get 6
        local.get 7
        local.get 3
        i32.const 2
        i32.add
        call $_ZN9const_hex4arch7generic6encode17hcf24167470e5e158E
        local.get 4
        local.get 3
        local.get 1
        local.get 5
        call_indirect (type 1)
        local.set 1
        local.get 3
        local.get 8
        call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
        local.get 1
        br_if 0 (;@2;)
        local.get 4
        i32.const 1049847
        i32.const 1
        local.get 5
        call_indirect (type 1)
        local.set 0
      end
      local.get 2
      i32.const 16
      i32.add
      global.set $__stack_pointer
      local.get 0
      return
    end
    i32.const 2
    local.get 1
    i32.const 1049336
    call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
    unreachable)
  (func $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$11allocate_in17hf9f36dcc15e8fce1E (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            br_if 0 (;@4;)
            i32.const 1
            local.set 2
            br 1 (;@3;)
          end
          local.get 1
          i32.const -1
          i32.le_s
          br_if 1 (;@2;)
          block  ;; label = @4
            block  ;; label = @5
              local.get 2
              br_if 0 (;@5;)
              local.get 3
              i32.const 8
              i32.add
              i32.const 1
              local.get 1
              call $_ZN63_$LT$alloc..alloc..Global$u20$as$u20$core..alloc..Allocator$GT$8allocate17h15badb2e48cc057cE.llvm.17895898940423260041
              local.get 3
              i32.load offset=8
              local.set 2
              br 1 (;@4;)
            end
            local.get 3
            i32.const 1
            local.get 1
            i32.const 1
            call $_ZN5alloc5alloc6Global10alloc_impl17h2c7b164b60447d0fE.llvm.17895898940423260041
            local.get 3
            i32.load
            local.set 2
          end
          local.get 2
          i32.eqz
          br_if 2 (;@1;)
        end
        local.get 0
        local.get 1
        i32.store offset=4
        local.get 0
        local.get 2
        i32.store
        local.get 3
        i32.const 16
        i32.add
        global.set $__stack_pointer
        return
      end
      call $_ZN5alloc7raw_vec17capacity_overflow17h26cdf55d7b744af0E
      unreachable
    end
    local.get 1
    call $__rust_alloc_error_handler
    unreachable)
  (func $_ZN84_$LT$alloc..vec..Vec$LT$T$C$A$GT$$u20$as$u20$core..ops..index..IndexMut$LT$I$GT$$GT$9index_mut17h4b293a527584354aE (type 9) (param i32 i32 i32 i32) (result i32)
    block  ;; label = @1
      local.get 2
      local.get 1
      i32.lt_u
      br_if 0 (;@1;)
      local.get 2
      local.get 1
      local.get 3
      call $_ZN4core9panicking18panic_bounds_check17hcafbc5434e2cd720E
      unreachable
    end
    local.get 0
    local.get 2
    i32.add)
  (func $_ZN9const_hex4arch7generic6encode17hcf24167470e5e158E (type 0) (param i32 i32 i32)
    (local i32)
    loop  ;; label = @1
      block  ;; label = @2
        local.get 1
        br_if 0 (;@2;)
        return
      end
      local.get 2
      i32.const 1
      i32.add
      local.get 0
      i32.load8_u
      local.tee 3
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8
      local.get 2
      local.get 3
      i32.const 4
      i32.shr_u
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8
      local.get 2
      i32.const 2
      i32.add
      local.set 2
      local.get 1
      i32.const -1
      i32.add
      local.set 1
      local.get 0
      i32.const 1
      i32.add
      local.set 0
      br 0 (;@1;)
    end)
  (func $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    local.get 0
    i32.store
    local.get 3
    local.get 1
    i32.store offset=4
    local.get 3
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 3
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 3
    i32.store
    local.get 3
    i32.const 2
    i32.store offset=12
    local.get 3
    i32.const 1050372
    i32.store offset=8
    local.get 3
    i32.const 3
    i32.store offset=36
    local.get 3
    local.get 3
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 3
    local.get 3
    i32.const 4
    i32.add
    i32.store offset=40
    local.get 3
    local.get 3
    i32.store offset=32
    local.get 3
    i32.const 8
    i32.add
    local.get 2
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN4core9panicking18panic_bounds_check17hcafbc5434e2cd720E (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    local.get 1
    i32.store offset=4
    local.get 3
    local.get 0
    i32.store
    local.get 3
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 3
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 3
    i32.store
    local.get 3
    i32.const 2
    i32.store offset=12
    local.get 3
    i32.const 1049956
    i32.store offset=8
    local.get 3
    i32.const 3
    i32.store offset=36
    local.get 3
    local.get 3
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 3
    local.get 3
    i32.store offset=40
    local.get 3
    local.get 3
    i32.const 4
    i32.add
    i32.store offset=32
    local.get 3
    i32.const 8
    i32.add
    local.get 2
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h4dc7eaf7cdd9e9ecE (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 144
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    i32.const 0
    local.set 4
    block  ;; label = @1
      loop  ;; label = @2
        local.get 4
        i32.const 64
        i32.eq
        br_if 1 (;@1;)
        local.get 3
        i32.const 80
        i32.add
        local.get 4
        i32.add
        i32.const 0
        i32.store16 align=1
        local.get 4
        i32.const 2
        i32.add
        local.set 4
        br 0 (;@2;)
      end
    end
    local.get 3
    i32.const 30768
    i32.store16 offset=14
    local.get 0
    i32.const 32
    local.get 3
    i32.const 16
    i32.add
    local.get 3
    i32.const 80
    i32.add
    i32.const 64
    call $memcpy
    call $_ZN9const_hex4arch7generic6encode17hcf24167470e5e158E
    local.get 1
    local.get 3
    i32.const 14
    i32.add
    i32.const 66
    local.get 2
    i32.load offset=12
    call_indirect (type 1)
    local.set 4
    local.get 3
    i32.const 144
    i32.add
    global.set $__stack_pointer
    local.get 4)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h994d0780e9319ec1E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    local.tee 0
    i32.load
    local.get 0
    i32.load offset=8
    local.get 1
    i32.load offset=20
    local.get 1
    i32.load offset=24
    call $_ZN40_$LT$str$u20$as$u20$core..fmt..Debug$GT$3fmt17hee1aab9b1929e229E)
  (func $_ZN40_$LT$str$u20$as$u20$core..fmt..Debug$GT$3fmt17hee1aab9b1929e229E (type 9) (param i32 i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i64 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 4
    global.set $__stack_pointer
    i32.const 1
    local.set 5
    block  ;; label = @1
      block  ;; label = @2
        local.get 2
        i32.const 34
        local.get 3
        i32.load offset=16
        local.tee 6
        call_indirect (type 2)
        br_if 0 (;@2;)
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            br_if 0 (;@4;)
            i32.const 0
            local.set 7
            i32.const 0
            local.set 1
            br 1 (;@3;)
          end
          local.get 0
          local.get 1
          i32.add
          local.set 8
          i32.const 0
          local.set 7
          local.get 0
          local.set 9
          i32.const 0
          local.set 10
          block  ;; label = @4
            block  ;; label = @5
              loop  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 9
                    local.tee 11
                    i32.load8_s
                    local.tee 12
                    i32.const -1
                    i32.le_s
                    br_if 0 (;@8;)
                    local.get 11
                    i32.const 1
                    i32.add
                    local.set 9
                    local.get 12
                    i32.const 255
                    i32.and
                    local.set 13
                    br 1 (;@7;)
                  end
                  local.get 11
                  i32.load8_u offset=1
                  i32.const 63
                  i32.and
                  local.set 14
                  local.get 12
                  i32.const 31
                  i32.and
                  local.set 15
                  block  ;; label = @8
                    local.get 12
                    i32.const -33
                    i32.gt_u
                    br_if 0 (;@8;)
                    local.get 15
                    i32.const 6
                    i32.shl
                    local.get 14
                    i32.or
                    local.set 13
                    local.get 11
                    i32.const 2
                    i32.add
                    local.set 9
                    br 1 (;@7;)
                  end
                  local.get 14
                  i32.const 6
                  i32.shl
                  local.get 11
                  i32.load8_u offset=2
                  i32.const 63
                  i32.and
                  i32.or
                  local.set 14
                  local.get 11
                  i32.const 3
                  i32.add
                  local.set 9
                  block  ;; label = @8
                    local.get 12
                    i32.const -16
                    i32.ge_u
                    br_if 0 (;@8;)
                    local.get 14
                    local.get 15
                    i32.const 12
                    i32.shl
                    i32.or
                    local.set 13
                    br 1 (;@7;)
                  end
                  local.get 14
                  i32.const 6
                  i32.shl
                  local.get 9
                  i32.load8_u
                  i32.const 63
                  i32.and
                  i32.or
                  local.get 15
                  i32.const 18
                  i32.shl
                  i32.const 1835008
                  i32.and
                  i32.or
                  local.tee 13
                  i32.const 1114112
                  i32.eq
                  br_if 3 (;@4;)
                  local.get 11
                  i32.const 4
                  i32.add
                  local.set 9
                end
                local.get 4
                i32.const 4
                i32.add
                local.get 13
                i32.const 65537
                call $_ZN4core4char7methods22_$LT$impl$u20$char$GT$16escape_debug_ext17hccd8a8eff60d1e48E
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 4
                    i32.load8_u offset=4
                    i32.const 128
                    i32.eq
                    br_if 0 (;@8;)
                    local.get 4
                    i32.load8_u offset=15
                    local.get 4
                    i32.load8_u offset=14
                    i32.sub
                    i32.const 255
                    i32.and
                    i32.const 1
                    i32.eq
                    br_if 0 (;@8;)
                    local.get 10
                    local.get 7
                    i32.lt_u
                    br_if 3 (;@5;)
                    block  ;; label = @9
                      local.get 7
                      i32.eqz
                      br_if 0 (;@9;)
                      block  ;; label = @10
                        local.get 7
                        local.get 1
                        i32.lt_u
                        br_if 0 (;@10;)
                        local.get 7
                        local.get 1
                        i32.eq
                        br_if 1 (;@9;)
                        br 5 (;@5;)
                      end
                      local.get 0
                      local.get 7
                      i32.add
                      i32.load8_s
                      i32.const -64
                      i32.lt_s
                      br_if 4 (;@5;)
                    end
                    block  ;; label = @9
                      local.get 10
                      i32.eqz
                      br_if 0 (;@9;)
                      block  ;; label = @10
                        local.get 10
                        local.get 1
                        i32.lt_u
                        br_if 0 (;@10;)
                        local.get 10
                        local.get 1
                        i32.eq
                        br_if 1 (;@9;)
                        br 5 (;@5;)
                      end
                      local.get 0
                      local.get 10
                      i32.add
                      i32.load8_s
                      i32.const -65
                      i32.le_s
                      br_if 4 (;@5;)
                    end
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 2
                        local.get 0
                        local.get 7
                        i32.add
                        local.get 10
                        local.get 7
                        i32.sub
                        local.get 3
                        i32.load offset=12
                        call_indirect (type 1)
                        br_if 0 (;@10;)
                        local.get 4
                        i32.const 16
                        i32.add
                        i32.const 8
                        i32.add
                        local.tee 15
                        local.get 4
                        i32.const 4
                        i32.add
                        i32.const 8
                        i32.add
                        i32.load
                        i32.store
                        local.get 4
                        local.get 4
                        i64.load offset=4 align=4
                        local.tee 16
                        i64.store offset=16
                        block  ;; label = @11
                          local.get 16
                          i32.wrap_i64
                          i32.const 255
                          i32.and
                          i32.const 128
                          i32.ne
                          br_if 0 (;@11;)
                          i32.const 128
                          local.set 14
                          loop  ;; label = @12
                            block  ;; label = @13
                              block  ;; label = @14
                                local.get 14
                                i32.const 255
                                i32.and
                                i32.const 128
                                i32.eq
                                br_if 0 (;@14;)
                                local.get 4
                                i32.load8_u offset=26
                                local.tee 12
                                local.get 4
                                i32.load8_u offset=27
                                i32.ge_u
                                br_if 5 (;@9;)
                                local.get 4
                                local.get 12
                                i32.const 1
                                i32.add
                                i32.store8 offset=26
                                local.get 12
                                i32.const 10
                                i32.ge_u
                                br_if 7 (;@7;)
                                local.get 4
                                i32.const 16
                                i32.add
                                local.get 12
                                i32.add
                                i32.load8_u
                                local.set 7
                                br 1 (;@13;)
                              end
                              i32.const 0
                              local.set 14
                              local.get 15
                              i32.const 0
                              i32.store
                              local.get 4
                              i32.load offset=20
                              local.set 7
                              local.get 4
                              i64.const 0
                              i64.store offset=16
                            end
                            local.get 2
                            local.get 7
                            local.get 6
                            call_indirect (type 2)
                            i32.eqz
                            br_if 0 (;@12;)
                            br 2 (;@10;)
                          end
                        end
                        local.get 4
                        i32.load8_u offset=26
                        local.tee 7
                        i32.const 10
                        local.get 7
                        i32.const 10
                        i32.gt_u
                        select
                        local.set 12
                        local.get 4
                        i32.load8_u offset=27
                        local.tee 14
                        local.get 7
                        local.get 14
                        local.get 7
                        i32.gt_u
                        select
                        local.set 17
                        loop  ;; label = @11
                          local.get 17
                          local.get 7
                          i32.eq
                          br_if 2 (;@9;)
                          local.get 4
                          local.get 7
                          i32.const 1
                          i32.add
                          local.tee 14
                          i32.store8 offset=26
                          local.get 12
                          local.get 7
                          i32.eq
                          br_if 4 (;@7;)
                          local.get 4
                          i32.const 16
                          i32.add
                          local.get 7
                          i32.add
                          local.set 15
                          local.get 14
                          local.set 7
                          local.get 2
                          local.get 15
                          i32.load8_u
                          local.get 6
                          call_indirect (type 2)
                          i32.eqz
                          br_if 0 (;@11;)
                        end
                      end
                      i32.const 1
                      local.set 5
                      br 7 (;@2;)
                    end
                    i32.const 1
                    local.set 7
                    block  ;; label = @9
                      local.get 13
                      i32.const 128
                      i32.lt_u
                      br_if 0 (;@9;)
                      i32.const 2
                      local.set 7
                      local.get 13
                      i32.const 2048
                      i32.lt_u
                      br_if 0 (;@9;)
                      i32.const 3
                      i32.const 4
                      local.get 13
                      i32.const 65536
                      i32.lt_u
                      select
                      local.set 7
                    end
                    local.get 7
                    local.get 10
                    i32.add
                    local.set 7
                  end
                  local.get 10
                  local.get 11
                  i32.sub
                  local.get 9
                  i32.add
                  local.set 10
                  local.get 9
                  local.get 8
                  i32.ne
                  br_if 1 (;@6;)
                  br 3 (;@4;)
                end
              end
              local.get 12
              i32.const 10
              i32.const 1052408
              call $_ZN4core9panicking18panic_bounds_check17hcafbc5434e2cd720E
              unreachable
            end
            local.get 0
            local.get 1
            local.get 7
            local.get 10
            i32.const 1050304
            call $_ZN4core3str16slice_error_fail17h5761aa5418ad8f04E
            unreachable
          end
          block  ;; label = @4
            local.get 7
            br_if 0 (;@4;)
            i32.const 0
            local.set 7
            br 1 (;@3;)
          end
          block  ;; label = @4
            local.get 1
            local.get 7
            i32.gt_u
            br_if 0 (;@4;)
            local.get 1
            local.get 7
            i32.ne
            br_if 3 (;@1;)
            local.get 1
            local.get 7
            i32.sub
            local.set 12
            local.get 1
            local.set 7
            local.get 12
            local.set 1
            br 1 (;@3;)
          end
          local.get 0
          local.get 7
          i32.add
          i32.load8_s
          i32.const -65
          i32.le_s
          br_if 2 (;@1;)
          local.get 1
          local.get 7
          i32.sub
          local.set 1
        end
        local.get 2
        local.get 0
        local.get 7
        i32.add
        local.get 1
        local.get 3
        i32.load offset=12
        call_indirect (type 1)
        br_if 0 (;@2;)
        local.get 2
        i32.const 34
        local.get 6
        call_indirect (type 2)
        local.set 5
      end
      local.get 4
      i32.const 32
      i32.add
      global.set $__stack_pointer
      local.get 5
      return
    end
    local.get 0
    local.get 1
    local.get 7
    local.get 1
    i32.const 1050288
    call $_ZN4core3str16slice_error_fail17h5761aa5418ad8f04E
    unreachable)
  (func $_ZN65_$LT$alloc..vec..Vec$LT$T$C$A$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17he4e6f5a62f3f6b24E (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 0
    i32.load offset=8
    i32.const 5
    i32.shl
    local.set 3
    local.get 0
    i32.load
    local.set 0
    local.get 1
    i32.load offset=20
    local.tee 4
    i32.const 1049904
    i32.const 1
    local.get 1
    i32.const 24
    i32.add
    i32.load
    local.tee 5
    i32.load offset=12
    local.tee 6
    call_indirect (type 1)
    local.set 7
    local.get 1
    i32.load offset=28
    i32.const 4
    i32.and
    local.set 8
    i32.const 1
    local.set 1
    loop (result i32)  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 3
            i32.eqz
            br_if 0 (;@4;)
            block  ;; label = @5
              local.get 7
              i32.const 1
              i32.and
              br_if 0 (;@5;)
              block  ;; label = @6
                local.get 8
                br_if 0 (;@6;)
                local.get 1
                i32.const 1
                i32.and
                br_if 3 (;@3;)
                i32.const 1
                local.set 7
                local.get 4
                i32.const 1050023
                i32.const 2
                local.get 6
                call_indirect (type 1)
                i32.eqz
                br_if 3 (;@3;)
                br 4 (;@2;)
              end
              block  ;; label = @6
                local.get 1
                i32.const 1
                i32.and
                i32.eqz
                br_if 0 (;@6;)
                i32.const 1
                local.set 7
                local.get 4
                i32.const 1050037
                i32.const 1
                local.get 6
                call_indirect (type 1)
                br_if 4 (;@2;)
              end
              local.get 2
              local.get 5
              i32.store offset=4
              local.get 2
              local.get 4
              i32.store
              local.get 2
              i32.const 1
              i32.store8 offset=15
              local.get 2
              local.get 2
              i32.const 15
              i32.add
              i32.store offset=8
              local.get 0
              local.get 2
              i32.const 1049992
              call $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h4dc7eaf7cdd9e9ecE
              br_if 0 (;@5;)
              local.get 2
              i32.const 1050028
              i32.const 2
              call $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$9write_str17h85101c618d2f4861E
              local.set 7
              br 3 (;@2;)
            end
            i32.const 1
            local.set 7
            br 2 (;@2;)
          end
          i32.const 1
          local.set 3
          block  ;; label = @4
            local.get 7
            i32.const 1
            i32.and
            br_if 0 (;@4;)
            local.get 4
            i32.const 1050038
            i32.const 1
            local.get 6
            call_indirect (type 1)
            local.set 3
          end
          local.get 2
          i32.const 16
          i32.add
          global.set $__stack_pointer
          local.get 3
          return
        end
        local.get 0
        local.get 4
        local.get 5
        call $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h4dc7eaf7cdd9e9ecE
        local.set 7
      end
      local.get 0
      i32.const 32
      i32.add
      local.set 0
      local.get 3
      i32.const -32
      i32.add
      local.set 3
      i32.const 0
      local.set 1
      br 0 (;@1;)
    end)
  (func $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$9write_str17h85101c618d2f4861E (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    local.get 0
    i32.load offset=4
    local.set 3
    local.get 0
    i32.load
    local.set 4
    local.get 0
    i32.load offset=8
    local.set 5
    i32.const 0
    local.set 6
    i32.const 0
    local.set 7
    i32.const 0
    local.set 8
    i32.const 0
    local.set 9
    block  ;; label = @1
      loop  ;; label = @2
        local.get 9
        i32.const 255
        i32.and
        br_if 1 (;@1;)
        block  ;; label = @3
          block  ;; label = @4
            local.get 8
            local.get 2
            i32.gt_u
            br_if 0 (;@4;)
            loop  ;; label = @5
              local.get 1
              local.get 8
              i32.add
              local.set 10
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 2
                        local.get 8
                        i32.sub
                        local.tee 9
                        i32.const 8
                        i32.lt_u
                        br_if 0 (;@10;)
                        local.get 10
                        i32.const 3
                        i32.add
                        i32.const -4
                        i32.and
                        local.tee 0
                        local.get 10
                        i32.eq
                        br_if 1 (;@9;)
                        local.get 0
                        local.get 10
                        i32.sub
                        local.tee 11
                        i32.eqz
                        br_if 1 (;@9;)
                        i32.const 0
                        local.set 0
                        loop  ;; label = @11
                          local.get 10
                          local.get 0
                          i32.add
                          i32.load8_u
                          i32.const 10
                          i32.eq
                          br_if 5 (;@6;)
                          local.get 11
                          local.get 0
                          i32.const 1
                          i32.add
                          local.tee 0
                          i32.ne
                          br_if 0 (;@11;)
                        end
                        local.get 11
                        local.get 9
                        i32.const -8
                        i32.add
                        local.tee 12
                        i32.gt_u
                        br_if 3 (;@7;)
                        br 2 (;@8;)
                      end
                      block  ;; label = @10
                        local.get 2
                        local.get 8
                        i32.ne
                        br_if 0 (;@10;)
                        local.get 2
                        local.set 8
                        br 6 (;@4;)
                      end
                      i32.const 0
                      local.set 0
                      loop  ;; label = @10
                        local.get 10
                        local.get 0
                        i32.add
                        i32.load8_u
                        i32.const 10
                        i32.eq
                        br_if 4 (;@6;)
                        local.get 9
                        local.get 0
                        i32.const 1
                        i32.add
                        local.tee 0
                        i32.ne
                        br_if 0 (;@10;)
                      end
                      local.get 2
                      local.set 8
                      br 5 (;@4;)
                    end
                    local.get 9
                    i32.const -8
                    i32.add
                    local.set 12
                    i32.const 0
                    local.set 11
                  end
                  loop  ;; label = @8
                    local.get 10
                    local.get 11
                    i32.add
                    local.tee 0
                    i32.const 4
                    i32.add
                    i32.load
                    local.tee 13
                    i32.const 168430090
                    i32.xor
                    i32.const -16843009
                    i32.add
                    local.get 13
                    i32.const -1
                    i32.xor
                    i32.and
                    local.get 0
                    i32.load
                    local.tee 0
                    i32.const 168430090
                    i32.xor
                    i32.const -16843009
                    i32.add
                    local.get 0
                    i32.const -1
                    i32.xor
                    i32.and
                    i32.or
                    i32.const -2139062144
                    i32.and
                    br_if 1 (;@7;)
                    local.get 11
                    i32.const 8
                    i32.add
                    local.tee 11
                    local.get 12
                    i32.le_u
                    br_if 0 (;@8;)
                  end
                end
                block  ;; label = @7
                  local.get 11
                  local.get 9
                  i32.ne
                  br_if 0 (;@7;)
                  local.get 2
                  local.set 8
                  br 3 (;@4;)
                end
                local.get 10
                local.get 11
                i32.add
                local.set 10
                local.get 2
                local.get 11
                i32.sub
                local.get 8
                i32.sub
                local.set 13
                i32.const 0
                local.set 0
                block  ;; label = @7
                  loop  ;; label = @8
                    local.get 10
                    local.get 0
                    i32.add
                    i32.load8_u
                    i32.const 10
                    i32.eq
                    br_if 1 (;@7;)
                    local.get 13
                    local.get 0
                    i32.const 1
                    i32.add
                    local.tee 0
                    i32.ne
                    br_if 0 (;@8;)
                  end
                  local.get 2
                  local.set 8
                  br 3 (;@4;)
                end
                local.get 0
                local.get 11
                i32.add
                local.set 0
              end
              local.get 8
              local.get 0
              i32.add
              local.tee 0
              i32.const 1
              i32.add
              local.set 8
              block  ;; label = @6
                local.get 0
                local.get 2
                i32.ge_u
                br_if 0 (;@6;)
                local.get 1
                local.get 0
                i32.add
                i32.load8_u
                i32.const 10
                i32.ne
                br_if 0 (;@6;)
                i32.const 0
                local.set 9
                local.get 8
                local.set 12
                local.get 8
                local.set 0
                br 3 (;@3;)
              end
              local.get 8
              local.get 2
              i32.le_u
              br_if 0 (;@5;)
            end
          end
          i32.const 1
          local.set 9
          local.get 7
          local.set 12
          local.get 2
          local.set 0
          local.get 7
          local.get 2
          i32.eq
          br_if 2 (;@1;)
        end
        block  ;; label = @3
          block  ;; label = @4
            local.get 5
            i32.load8_u
            i32.eqz
            br_if 0 (;@4;)
            local.get 4
            i32.const 1050016
            i32.const 4
            local.get 3
            i32.load offset=12
            call_indirect (type 1)
            br_if 1 (;@3;)
          end
          local.get 1
          local.get 7
          i32.add
          local.set 11
          local.get 0
          local.get 7
          i32.sub
          local.set 10
          i32.const 0
          local.set 13
          block  ;; label = @4
            local.get 0
            local.get 7
            i32.eq
            br_if 0 (;@4;)
            local.get 10
            local.get 11
            i32.add
            i32.const -1
            i32.add
            i32.load8_u
            i32.const 10
            i32.eq
            local.set 13
          end
          local.get 5
          local.get 13
          i32.store8
          local.get 12
          local.set 7
          local.get 4
          local.get 11
          local.get 10
          local.get 3
          i32.load offset=12
          call_indirect (type 1)
          i32.eqz
          br_if 1 (;@2;)
        end
      end
      i32.const 1
      local.set 6
    end
    local.get 6)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h0006516aa0b41931E (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 0
            i32.load
            local.tee 3
            i32.load
            local.tee 0
            i32.const -1114111
            i32.add
            i32.const 0
            local.get 0
            i32.const 2097150
            i32.and
            i32.const 1114112
            i32.eq
            select
            br_table 0 (;@4;) 1 (;@3;) 2 (;@2;) 0 (;@4;)
          end
          local.get 2
          local.get 3
          i32.const 4
          i32.add
          i32.store offset=12
          local.get 1
          i32.const 1049716
          i32.const 19
          i32.const 1049735
          i32.const 1
          local.get 3
          i32.const 1049736
          i32.const 1049752
          i32.const 5
          local.get 2
          i32.const 12
          i32.add
          i32.const 1049760
          call $_ZN4core3fmt9Formatter26debug_struct_field2_finish17h5c77ab8d3e21ede3E
          local.set 1
          br 2 (;@1;)
        end
        local.get 1
        i32.load offset=20
        i32.const 1049776
        i32.const 9
        local.get 1
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        local.set 1
        br 1 (;@1;)
      end
      local.get 1
      i32.load offset=20
      i32.const 1049785
      i32.const 19
      local.get 1
      i32.const 24
      i32.add
      i32.load
      i32.load offset=12
      call_indirect (type 1)
      local.set 1
    end
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 1)
  (func $_ZN4core3fmt9Formatter26debug_struct_field2_finish17h5c77ab8d3e21ede3E (type 10) (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 11
    global.set $__stack_pointer
    local.get 0
    i32.load offset=20
    local.get 1
    local.get 2
    local.get 0
    i32.const 24
    i32.add
    i32.load
    i32.load offset=12
    call_indirect (type 1)
    local.set 2
    local.get 11
    i32.const 0
    i32.store8 offset=13
    local.get 11
    local.get 2
    i32.store8 offset=12
    local.get 11
    local.get 0
    i32.store offset=8
    local.get 11
    i32.const 8
    i32.add
    local.get 3
    local.get 4
    local.get 5
    local.get 6
    call $_ZN4core3fmt8builders11DebugStruct5field17h553effddb6af86d3E
    local.get 7
    local.get 8
    local.get 9
    local.get 10
    call $_ZN4core3fmt8builders11DebugStruct5field17h553effddb6af86d3E
    local.set 1
    local.get 11
    i32.load8_u offset=12
    local.set 2
    block  ;; label = @1
      block  ;; label = @2
        local.get 11
        i32.load8_u offset=13
        br_if 0 (;@2;)
        local.get 2
        i32.const 255
        i32.and
        i32.const 0
        i32.ne
        local.set 0
        br 1 (;@1;)
      end
      i32.const 1
      local.set 0
      local.get 2
      i32.const 255
      i32.and
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 1
        i32.load
        local.tee 0
        i32.load8_u offset=28
        i32.const 4
        i32.and
        br_if 0 (;@2;)
        local.get 0
        i32.load offset=20
        i32.const 1050031
        i32.const 2
        local.get 0
        i32.load offset=24
        i32.load offset=12
        call_indirect (type 1)
        local.set 0
        br 1 (;@1;)
      end
      local.get 0
      i32.load offset=20
      i32.const 1050030
      i32.const 1
      local.get 0
      i32.load offset=24
      i32.load offset=12
      call_indirect (type 1)
      local.set 0
    end
    local.get 11
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN4core3ptr101drop_in_place$LT$alloc..vec..Vec$LT$alloy_primitives..bits..fixed..FixedBytes$LT$32_usize$GT$$GT$$GT$17hb9871b3d6a9bf65fE (type 6) (param i32)
    local.get 0
    i32.load
    local.get 0
    i32.load offset=4
    call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h1b2b8d6979887e6eE)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h54ff38d57ce564c9E (type 2) (param i32 i32) (result i32)
    (local i32)
    local.get 0
    i32.load
    local.set 0
    block  ;; label = @1
      local.get 1
      i32.load offset=28
      local.tee 2
      i32.const 16
      i32.and
      br_if 0 (;@1;)
      local.get 0
      i32.load8_u
      local.set 0
      block  ;; label = @2
        local.get 2
        i32.const 32
        i32.and
        br_if 0 (;@2;)
        local.get 0
        local.get 1
        call $_ZN4core3fmt3num3imp51_$LT$impl$u20$core..fmt..Display$u20$for$u20$u8$GT$3fmt17h2c41a8f626920984E
        return
      end
      local.get 0
      local.get 1
      call $_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..UpperHex$u20$for$u20$i8$GT$3fmt17h0b24ebddf20b35c3E
      return
    end
    local.get 0
    i32.load8_u
    local.get 1
    call $_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..LowerHex$u20$for$u20$i8$GT$3fmt17hbbac6e7052f4c4a1E)
  (func $_ZN4core3fmt3num3imp51_$LT$impl$u20$core..fmt..Display$u20$for$u20$u8$GT$3fmt17h2c41a8f626920984E (type 2) (param i32 i32) (result i32)
    local.get 0
    i64.extend_i32_u
    i64.const 255
    i64.and
    local.get 1
    call $_ZN4core3fmt3num3imp7fmt_u6417h416abf9443aa8afdE)
  (func $_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..UpperHex$u20$for$u20$i8$GT$3fmt17h0b24ebddf20b35c3E (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    i32.const 127
    local.set 3
    loop  ;; label = @1
      local.get 2
      local.get 3
      local.tee 4
      i32.add
      local.tee 5
      i32.const 48
      i32.const 55
      local.get 0
      i32.const 15
      i32.and
      local.tee 3
      i32.const 10
      i32.lt_u
      select
      local.get 3
      i32.add
      i32.store8
      local.get 4
      i32.const -1
      i32.add
      local.set 3
      local.get 0
      i32.const 255
      i32.and
      local.tee 6
      i32.const 4
      i32.shr_u
      local.set 0
      local.get 6
      i32.const 16
      i32.ge_u
      br_if 0 (;@1;)
    end
    block  ;; label = @1
      local.get 4
      i32.const 128
      i32.le_u
      br_if 0 (;@1;)
      local.get 4
      i32.const 128
      i32.const 1050068
      call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
      unreachable
    end
    local.get 1
    i32.const 1050084
    i32.const 2
    local.get 5
    i32.const 129
    local.get 4
    i32.const 1
    i32.add
    i32.sub
    call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
    local.set 0
    local.get 2
    i32.const 128
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..LowerHex$u20$for$u20$i8$GT$3fmt17hbbac6e7052f4c4a1E (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    i32.const 127
    local.set 3
    loop  ;; label = @1
      local.get 2
      local.get 3
      local.tee 4
      i32.add
      local.tee 5
      i32.const 48
      i32.const 87
      local.get 0
      i32.const 15
      i32.and
      local.tee 3
      i32.const 10
      i32.lt_u
      select
      local.get 3
      i32.add
      i32.store8
      local.get 4
      i32.const -1
      i32.add
      local.set 3
      local.get 0
      i32.const 255
      i32.and
      local.tee 6
      i32.const 4
      i32.shr_u
      local.set 0
      local.get 6
      i32.const 16
      i32.ge_u
      br_if 0 (;@1;)
    end
    block  ;; label = @1
      local.get 4
      i32.const 128
      i32.le_u
      br_if 0 (;@1;)
      local.get 4
      i32.const 128
      i32.const 1050068
      call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
      unreachable
    end
    local.get 1
    i32.const 1050084
    i32.const 2
    local.get 5
    i32.const 129
    local.get 4
    i32.const 1
    i32.add
    i32.sub
    call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
    local.set 0
    local.get 2
    i32.const 128
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h451fa72cb80b8158E (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 0
    i32.load
    local.set 3
    i32.const 0
    local.set 0
    block  ;; label = @1
      loop  ;; label = @2
        local.get 0
        i32.const 8
        i32.eq
        br_if 1 (;@1;)
        local.get 2
        i32.const 24
        i32.add
        local.get 0
        i32.add
        i32.const 0
        i32.store16
        local.get 0
        i32.const 2
        i32.add
        local.set 0
        br 0 (;@2;)
      end
    end
    local.get 2
    i32.const 30768
    i32.store16 offset=14
    local.get 2
    local.get 2
    i64.load offset=24
    i64.store offset=16 align=2
    local.get 3
    i32.const 4
    local.get 2
    i32.const 16
    i32.add
    call $_ZN9const_hex4arch7generic6encode17hcf24167470e5e158E
    local.get 1
    i32.load offset=20
    local.get 2
    i32.const 14
    i32.add
    i32.const 10
    local.get 1
    i32.const 24
    i32.add
    i32.load
    i32.load offset=12
    call_indirect (type 1)
    local.set 0
    local.get 2
    i32.const 32
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$16reserve_for_push17hf6d581f7796c012dE (type 5) (param i32 i32)
    (local i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    i32.const 0
    local.set 3
    block  ;; label = @1
      local.get 1
      i32.const 1
      i32.add
      local.tee 1
      i32.eqz
      br_if 0 (;@1;)
      local.get 0
      i32.load offset=4
      local.tee 3
      i32.const 1
      i32.shl
      local.tee 4
      local.get 1
      local.get 4
      local.get 1
      i32.gt_u
      select
      local.tee 1
      i32.const 4
      local.get 1
      i32.const 4
      i32.gt_u
      select
      local.tee 1
      i32.const 2
      i32.shl
      local.set 4
      local.get 1
      i32.const 536870912
      i32.lt_u
      i32.const 2
      i32.shl
      local.set 5
      block  ;; label = @2
        block  ;; label = @3
          local.get 3
          br_if 0 (;@3;)
          local.get 2
          i32.const 0
          i32.store offset=24
          br 1 (;@2;)
        end
        local.get 2
        i32.const 4
        i32.store offset=24
        local.get 2
        local.get 3
        i32.const 2
        i32.shl
        i32.store offset=28
        local.get 2
        local.get 0
        i32.load
        i32.store offset=20
      end
      local.get 2
      i32.const 8
      i32.add
      local.get 5
      local.get 4
      local.get 2
      i32.const 20
      i32.add
      call $_ZN5alloc7raw_vec11finish_grow17h976f30d369f21f6aE
      local.get 2
      i32.load offset=12
      local.set 3
      block  ;; label = @2
        local.get 2
        i32.load offset=8
        i32.eqz
        br_if 0 (;@2;)
        local.get 2
        i32.const 16
        i32.add
        i32.load
        local.set 1
        br 1 (;@1;)
      end
      local.get 0
      local.get 1
      i32.store offset=4
      local.get 0
      local.get 3
      i32.store
      i32.const -2147483647
      local.set 3
    end
    local.get 3
    local.get 1
    call $_ZN5alloc7raw_vec14handle_reserve17h7b8a866c605ec769E
    local.get 2
    i32.const 32
    i32.add
    global.set $__stack_pointer)
  (func $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$16reserve_for_push17h51cccd5bdd1e8d04E (type 5) (param i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 2
    i32.const 8
    i32.add
    local.get 0
    local.get 1
    i32.const 1
    call $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$14grow_amortized17h7e52c94f5aba65a2E
    local.get 2
    i32.load offset=8
    local.get 2
    i32.load offset=12
    call $_ZN5alloc7raw_vec14handle_reserve17h7b8a866c605ec769E
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h331c597912658dd7E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    local.get 0
    i32.load offset=4
    local.get 1
    i32.load offset=20
    local.get 1
    i32.load offset=24
    call $_ZN40_$LT$str$u20$as$u20$core..fmt..Debug$GT$3fmt17hee1aab9b1929e229E)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h2406429923ed46efE (type 2) (param i32 i32) (result i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 2
    local.get 0
    i32.load
    i32.load
    local.tee 0
    i32.const 12
    i32.add
    i32.store offset=12
    local.get 1
    i32.const 1049358
    i32.const 3
    i32.const 1049361
    i32.const 6
    local.get 0
    i32.const 1049368
    i32.const 1049444
    i32.const 4
    local.get 2
    i32.const 12
    i32.add
    i32.const 1049384
    call $_ZN4core3fmt9Formatter26debug_struct_field2_finish17h5c77ab8d3e21ede3E
    local.set 0
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h3b800cbb107d7d7bE (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    local.get 1
    call $_ZN64_$LT$alloc..borrow..Cow$LT$B$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17he20c672607de4bccE)
  (func $_ZN64_$LT$alloc..borrow..Cow$LT$B$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17he20c672607de4bccE (type 2) (param i32 i32) (result i32)
    (local i32)
    local.get 0
    i32.load
    local.tee 2
    local.get 0
    i32.load offset=4
    local.get 2
    select
    local.get 0
    i32.const 8
    i32.add
    i32.load
    local.get 1
    i32.load offset=20
    local.get 1
    i32.load offset=24
    call $_ZN40_$LT$str$u20$as$u20$core..fmt..Debug$GT$3fmt17hee1aab9b1929e229E)
  (func $_ZN67_$LT$alloy_sol_types..errors..Error$u20$as$u20$core..fmt..Debug$GT$3fmt17hae0106c69bd4ae0eE (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 0
                        i32.load8_u
                        br_table 0 (;@10;) 1 (;@9;) 2 (;@8;) 3 (;@7;) 4 (;@6;) 5 (;@5;) 6 (;@4;) 7 (;@3;) 8 (;@2;) 0 (;@10;)
                      end
                      local.get 2
                      local.get 0
                      i32.const 16
                      i32.add
                      i32.store offset=8
                      local.get 1
                      i32.const 1049400
                      i32.const 13
                      i32.const 1049413
                      i32.const 13
                      local.get 0
                      i32.const 4
                      i32.add
                      i32.const 1049428
                      i32.const 1049444
                      i32.const 4
                      local.get 2
                      i32.const 8
                      i32.add
                      i32.const 1049448
                      call $_ZN4core3fmt9Formatter26debug_struct_field2_finish17h5c77ab8d3e21ede3E
                      local.set 1
                      br 8 (;@1;)
                    end
                    local.get 1
                    i32.load offset=20
                    i32.const 1049464
                    i32.const 7
                    local.get 1
                    i32.const 24
                    i32.add
                    i32.load
                    i32.load offset=12
                    call_indirect (type 1)
                    local.set 1
                    br 7 (;@1;)
                  end
                  local.get 1
                  i32.load offset=20
                  i32.const 1049471
                  i32.const 14
                  local.get 1
                  i32.const 24
                  i32.add
                  i32.load
                  i32.load offset=12
                  call_indirect (type 1)
                  local.set 1
                  br 6 (;@1;)
                end
                local.get 1
                i32.load offset=20
                i32.const 1049485
                i32.const 13
                local.get 1
                i32.const 24
                i32.add
                i32.load
                i32.load offset=12
                call_indirect (type 1)
                local.set 1
                br 5 (;@1;)
              end
              local.get 2
              local.get 0
              i32.const 2
              i32.add
              i32.store offset=4
              local.get 1
              i32.load offset=20
              i32.const 1049498
              i32.const 16
              local.get 1
              i32.const 24
              i32.add
              i32.load
              i32.load offset=12
              call_indirect (type 1)
              local.set 3
              local.get 2
              i32.const 0
              i32.store8 offset=13
              local.get 2
              local.get 3
              i32.store8 offset=12
              local.get 2
              local.get 1
              i32.store offset=8
              i32.const 1
              local.set 1
              local.get 2
              i32.const 8
              i32.add
              i32.const 1049514
              i32.const 4
              local.get 0
              i32.const 4
              i32.add
              i32.const 1049520
              call $_ZN4core3fmt8builders11DebugStruct5field17h553effddb6af86d3E
              i32.const 1049536
              i32.const 5
              local.get 0
              i32.const 1
              i32.add
              i32.const 1049544
              call $_ZN4core3fmt8builders11DebugStruct5field17h553effddb6af86d3E
              i32.const 1049560
              i32.const 3
              local.get 2
              i32.const 4
              i32.add
              i32.const 1049564
              call $_ZN4core3fmt8builders11DebugStruct5field17h553effddb6af86d3E
              local.set 3
              local.get 2
              i32.load8_u offset=12
              local.set 0
              block  ;; label = @6
                local.get 2
                i32.load8_u offset=13
                br_if 0 (;@6;)
                local.get 0
                i32.const 255
                i32.and
                i32.const 0
                i32.ne
                local.set 1
                br 5 (;@1;)
              end
              local.get 0
              i32.const 255
              i32.and
              br_if 4 (;@1;)
              block  ;; label = @6
                local.get 3
                i32.load
                local.tee 0
                i32.load8_u offset=28
                i32.const 4
                i32.and
                br_if 0 (;@6;)
                local.get 0
                i32.load offset=20
                i32.const 1050031
                i32.const 2
                local.get 0
                i32.load offset=24
                i32.load offset=12
                call_indirect (type 1)
                local.set 1
                br 5 (;@1;)
              end
              local.get 0
              i32.load offset=20
              i32.const 1050030
              i32.const 1
              local.get 0
              i32.load offset=24
              i32.load offset=12
              call_indirect (type 1)
              local.set 1
              br 4 (;@1;)
            end
            local.get 2
            local.get 0
            i32.const 12
            i32.add
            i32.store offset=8
            local.get 1
            i32.const 1049580
            i32.const 10
            i32.const 1049514
            i32.const 4
            local.get 0
            i32.const 4
            i32.add
            i32.const 1049520
            i32.const 1049590
            i32.const 3
            local.get 2
            i32.const 8
            i32.add
            i32.const 1049596
            call $_ZN4core3fmt9Formatter26debug_struct_field2_finish17h5c77ab8d3e21ede3E
            local.set 1
            br 3 (;@1;)
          end
          local.get 2
          local.get 0
          i32.const 1
          i32.add
          i32.store offset=8
          local.get 1
          i32.const 1049612
          i32.const 15
          i32.const 1049514
          i32.const 4
          local.get 0
          i32.const 8
          i32.add
          i32.const 1049520
          i32.const 1049627
          i32.const 8
          local.get 2
          i32.const 8
          i32.add
          i32.const 1049636
          call $_ZN4core3fmt9Formatter26debug_struct_field2_finish17h5c77ab8d3e21ede3E
          local.set 1
          br 2 (;@1;)
        end
        local.get 2
        local.get 0
        i32.const 4
        i32.add
        i32.store offset=8
        local.get 1
        i32.const 1049652
        i32.const 12
        local.get 2
        i32.const 8
        i32.add
        i32.const 1049664
        call $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE
        local.set 1
        br 1 (;@1;)
      end
      local.get 2
      local.get 0
      i32.const 4
      i32.add
      i32.store offset=8
      local.get 1
      i32.const 1049680
      i32.const 5
      local.get 2
      i32.const 8
      i32.add
      i32.const 1049688
      call $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE
      local.set 1
    end
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 1)
  (func $_ZN4core3fmt8builders11DebugStruct5field17h553effddb6af86d3E (type 11) (param i32 i32 i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i64)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 5
    global.set $__stack_pointer
    i32.const 1
    local.set 6
    block  ;; label = @1
      local.get 0
      i32.load8_u offset=4
      br_if 0 (;@1;)
      local.get 0
      i32.load8_u offset=5
      local.set 7
      block  ;; label = @2
        local.get 0
        i32.load
        local.tee 8
        i32.load offset=28
        local.tee 9
        i32.const 4
        i32.and
        br_if 0 (;@2;)
        i32.const 1
        local.set 6
        local.get 8
        i32.load offset=20
        i32.const 1050023
        i32.const 1050020
        local.get 7
        i32.const 255
        i32.and
        local.tee 7
        select
        i32.const 2
        i32.const 3
        local.get 7
        select
        local.get 8
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        i32.const 1
        local.set 6
        local.get 8
        i32.load offset=20
        local.get 1
        local.get 2
        local.get 8
        i32.load offset=24
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        i32.const 1
        local.set 6
        local.get 8
        i32.load offset=20
        i32.const 1049972
        i32.const 2
        local.get 8
        i32.load offset=24
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        local.get 3
        local.get 8
        local.get 4
        i32.load offset=12
        call_indirect (type 2)
        local.set 6
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 7
        i32.const 255
        i32.and
        br_if 0 (;@2;)
        i32.const 1
        local.set 6
        local.get 8
        i32.load offset=20
        i32.const 1050025
        i32.const 3
        local.get 8
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        local.get 8
        i32.load offset=28
        local.set 9
      end
      i32.const 1
      local.set 6
      local.get 5
      i32.const 1
      i32.store8 offset=27
      local.get 5
      i32.const 52
      i32.add
      i32.const 1049992
      i32.store
      local.get 5
      local.get 8
      i64.load offset=20 align=4
      i64.store offset=12 align=4
      local.get 5
      local.get 5
      i32.const 27
      i32.add
      i32.store offset=20
      local.get 5
      local.get 8
      i64.load offset=8 align=4
      i64.store offset=36 align=4
      local.get 8
      i64.load align=4
      local.set 10
      local.get 5
      local.get 9
      i32.store offset=56
      local.get 5
      local.get 8
      i32.load offset=16
      i32.store offset=44
      local.get 5
      local.get 8
      i32.load8_u offset=32
      i32.store8 offset=60
      local.get 5
      local.get 10
      i64.store offset=28 align=4
      local.get 5
      local.get 5
      i32.const 12
      i32.add
      i32.store offset=48
      local.get 5
      i32.const 12
      i32.add
      local.get 1
      local.get 2
      call $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$9write_str17h85101c618d2f4861E
      br_if 0 (;@1;)
      local.get 5
      i32.const 12
      i32.add
      i32.const 1049972
      i32.const 2
      call $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$9write_str17h85101c618d2f4861E
      br_if 0 (;@1;)
      local.get 3
      local.get 5
      i32.const 28
      i32.add
      local.get 4
      i32.load offset=12
      call_indirect (type 2)
      br_if 0 (;@1;)
      local.get 5
      i32.load offset=48
      i32.const 1050028
      i32.const 2
      local.get 5
      i32.load offset=52
      i32.load offset=12
      call_indirect (type 1)
      local.set 6
    end
    local.get 0
    i32.const 1
    i32.store8 offset=5
    local.get 0
    local.get 6
    i32.store8 offset=4
    local.get 5
    i32.const 64
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE (type 11) (param i32 i32 i32 i32 i32) (result i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 5
    global.set $__stack_pointer
    local.get 5
    local.get 0
    i32.load offset=20
    local.get 1
    local.get 2
    local.get 0
    i32.const 24
    i32.add
    i32.load
    i32.load offset=12
    call_indirect (type 1)
    i32.store8 offset=12
    local.get 5
    local.get 0
    i32.store offset=8
    local.get 5
    i32.const 0
    i32.store8 offset=13
    local.get 5
    i32.const 0
    i32.store offset=4
    local.get 5
    i32.const 4
    i32.add
    local.get 3
    local.get 4
    call $_ZN4core3fmt8builders10DebugTuple5field17hbc9d984656bef3fdE
    local.set 0
    local.get 5
    i32.load8_u offset=12
    local.set 2
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i32.load
        local.tee 1
        br_if 0 (;@2;)
        local.get 2
        i32.const 255
        i32.and
        i32.const 0
        i32.ne
        local.set 0
        br 1 (;@1;)
      end
      i32.const 1
      local.set 0
      local.get 2
      i32.const 255
      i32.and
      br_if 0 (;@1;)
      local.get 5
      i32.load offset=8
      local.set 2
      block  ;; label = @2
        local.get 1
        i32.const 1
        i32.ne
        br_if 0 (;@2;)
        local.get 5
        i32.load8_u offset=13
        i32.const 255
        i32.and
        i32.eqz
        br_if 0 (;@2;)
        local.get 2
        i32.load8_u offset=28
        i32.const 4
        i32.and
        br_if 0 (;@2;)
        i32.const 1
        local.set 0
        local.get 2
        i32.load offset=20
        i32.const 1050036
        i32.const 1
        local.get 2
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
      end
      local.get 2
      i32.load offset=20
      i32.const 1049847
      i32.const 1
      local.get 2
      i32.const 24
      i32.add
      i32.load
      i32.load offset=12
      call_indirect (type 1)
      local.set 0
    end
    local.get 5
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN4core3ptr23drop_in_place$LT$u8$GT$17ha25c34eed54542e7E (type 6) (param i32))
  (func $_ZN4core3fmt3num49_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u8$GT$3fmt17h61affea28b29a946E (type 2) (param i32 i32) (result i32)
    (local i32)
    block  ;; label = @1
      local.get 1
      i32.load offset=28
      local.tee 2
      i32.const 16
      i32.and
      br_if 0 (;@1;)
      local.get 0
      i32.load8_u
      local.set 0
      block  ;; label = @2
        local.get 2
        i32.const 32
        i32.and
        br_if 0 (;@2;)
        local.get 0
        local.get 1
        call $_ZN4core3fmt3num3imp51_$LT$impl$u20$core..fmt..Display$u20$for$u20$u8$GT$3fmt17h2c41a8f626920984E
        return
      end
      local.get 0
      local.get 1
      call $_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..UpperHex$u20$for$u20$i8$GT$3fmt17h0b24ebddf20b35c3E
      return
    end
    local.get 0
    i32.load8_u
    local.get 1
    call $_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..LowerHex$u20$for$u20$i8$GT$3fmt17hbbac6e7052f4c4a1E)
  (func $_ZN4core3ptr50drop_in_place$LT$alloc..borrow..Cow$LT$str$GT$$GT$17h8819b2f7b427b7ceE.195 (type 6) (param i32)
    (local i32)
    block  ;; label = @1
      local.get 0
      i32.load
      local.tee 1
      i32.eqz
      br_if 0 (;@1;)
      local.get 1
      local.get 0
      i32.load offset=4
      call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
    end)
  (func $_ZN5alloc7raw_vec11finish_grow17h976f30d369f21f6aE (type 3) (param i32 i32 i32 i32)
    (local i32 i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 4
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 1
            i32.eqz
            br_if 0 (;@4;)
            local.get 2
            i32.const -1
            i32.le_s
            br_if 1 (;@3;)
            block  ;; label = @5
              block  ;; label = @6
                local.get 3
                i32.load offset=4
                i32.eqz
                br_if 0 (;@6;)
                block  ;; label = @7
                  local.get 3
                  i32.const 8
                  i32.add
                  i32.load
                  local.tee 5
                  br_if 0 (;@7;)
                  local.get 4
                  i32.const 8
                  i32.add
                  local.get 1
                  local.get 2
                  i32.const 0
                  call $_ZN5alloc5alloc6Global10alloc_impl17h2c7b164b60447d0fE.llvm.17895898940423260041
                  local.get 4
                  i32.load offset=12
                  local.set 5
                  local.get 4
                  i32.load offset=8
                  local.set 3
                  br 2 (;@5;)
                end
                local.get 3
                i32.load
                local.set 6
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 1
                    local.get 2
                    call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$5alloc17h940356afe3ab03a7E
                    local.tee 3
                    br_if 0 (;@8;)
                    i32.const 0
                    local.set 3
                    br 1 (;@7;)
                  end
                  local.get 3
                  local.get 6
                  local.get 5
                  call $memcpy
                  drop
                  local.get 6
                  local.get 5
                  call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$7dealloc17h96238585fd016669E
                end
                local.get 2
                local.set 5
                br 1 (;@5;)
              end
              local.get 4
              local.get 1
              local.get 2
              call $_ZN63_$LT$alloc..alloc..Global$u20$as$u20$core..alloc..Allocator$GT$8allocate17h15badb2e48cc057cE.llvm.17895898940423260041
              local.get 4
              i32.load offset=4
              local.set 5
              local.get 4
              i32.load
              local.set 3
            end
            block  ;; label = @5
              local.get 3
              i32.eqz
              br_if 0 (;@5;)
              local.get 0
              local.get 3
              i32.store offset=4
              local.get 0
              i32.const 8
              i32.add
              local.get 5
              i32.store
              i32.const 0
              local.set 2
              br 4 (;@1;)
            end
            local.get 0
            local.get 1
            i32.store offset=4
            local.get 0
            i32.const 8
            i32.add
            local.get 2
            i32.store
            br 2 (;@2;)
          end
          local.get 0
          i32.const 0
          i32.store offset=4
          local.get 0
          i32.const 8
          i32.add
          local.get 2
          i32.store
          br 1 (;@2;)
        end
        local.get 0
        i32.const 0
        i32.store offset=4
      end
      i32.const 1
      local.set 2
    end
    local.get 0
    local.get 2
    i32.store
    local.get 4
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN5bytes5bytes12static_clone17h796320c79f4bc4b4E (type 3) (param i32 i32 i32 i32)
    local.get 0
    i32.const 0
    i32.store offset=12
    local.get 0
    local.get 3
    i32.store offset=8
    local.get 0
    local.get 2
    i32.store offset=4
    local.get 0
    i32.const 1049704
    i32.store)
  (func $_ZN5bytes5bytes13static_to_vec17h3ffba84ffc73a880E (type 3) (param i32 i32 i32 i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 4
    global.set $__stack_pointer
    local.get 4
    i32.const 8
    i32.add
    local.get 3
    i32.const 0
    call $_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$11allocate_in17hf9f36dcc15e8fce1E
    local.get 4
    i32.load offset=12
    local.set 5
    local.get 4
    i32.load offset=8
    local.get 2
    local.get 3
    call $memcpy
    local.set 2
    local.get 0
    local.get 3
    i32.store offset=8
    local.get 0
    local.get 5
    i32.store offset=4
    local.get 0
    local.get 2
    i32.store
    local.get 4
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN5alloc5alloc6Global10alloc_impl17h2c7b164b60447d0fE.llvm.17895898940423260041 (type 3) (param i32 i32 i32 i32)
    block  ;; label = @1
      local.get 2
      i32.eqz
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 3
        br_if 0 (;@2;)
        i32.const 0
        i32.load8_u offset=1053456
        drop
        local.get 1
        local.get 2
        call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$5alloc17h940356afe3ab03a7E
        local.set 1
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 1
        local.get 2
        call $_ZN72_$LT$wee_alloc..WeeAlloc$u20$as$u20$core..alloc..global..GlobalAlloc$GT$5alloc17h940356afe3ab03a7E
        local.tee 1
        br_if 0 (;@2;)
        i32.const 0
        local.set 1
        br 1 (;@1;)
      end
      local.get 1
      i32.const 0
      local.get 2
      call $memset
      drop
    end
    local.get 0
    local.get 2
    i32.store offset=4
    local.get 0
    local.get 1
    i32.store)
  (func $_ZN63_$LT$alloc..alloc..Global$u20$as$u20$core..alloc..Allocator$GT$8allocate17h15badb2e48cc057cE.llvm.17895898940423260041 (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    i32.const 8
    i32.add
    local.get 1
    local.get 2
    i32.const 0
    call $_ZN5alloc5alloc6Global10alloc_impl17h2c7b164b60447d0fE.llvm.17895898940423260041
    local.get 3
    i32.load offset=12
    local.set 2
    local.get 0
    local.get 3
    i32.load offset=8
    i32.store
    local.get 0
    local.get 2
    i32.store offset=4
    local.get 3
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h3be23310966946e7E (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 0
    i32.load
    local.set 0
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 1
              i32.load offset=28
              local.tee 3
              i32.const 16
              i32.and
              br_if 0 (;@5;)
              local.get 3
              i32.const 32
              i32.and
              br_if 1 (;@4;)
              local.get 0
              local.get 1
              call $_ZN4core3fmt3num3imp52_$LT$impl$u20$core..fmt..Display$u20$for$u20$u32$GT$3fmt17he696c0e431156bceE
              local.set 0
              br 2 (;@3;)
            end
            local.get 0
            i32.load
            local.set 0
            i32.const 127
            local.set 4
            loop  ;; label = @5
              local.get 2
              local.get 4
              local.tee 3
              i32.add
              local.tee 5
              i32.const 48
              i32.const 87
              local.get 0
              i32.const 15
              i32.and
              local.tee 4
              i32.const 10
              i32.lt_u
              select
              local.get 4
              i32.add
              i32.store8
              local.get 3
              i32.const -1
              i32.add
              local.set 4
              local.get 0
              i32.const 16
              i32.lt_u
              local.set 6
              local.get 0
              i32.const 4
              i32.shr_u
              local.set 0
              local.get 6
              i32.eqz
              br_if 0 (;@5;)
            end
            local.get 3
            i32.const 128
            i32.gt_u
            br_if 2 (;@2;)
            local.get 1
            i32.const 1050084
            i32.const 2
            local.get 5
            i32.const 129
            local.get 3
            i32.const 1
            i32.add
            i32.sub
            call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
            local.set 0
            br 1 (;@3;)
          end
          local.get 0
          i32.load
          local.set 0
          i32.const 127
          local.set 4
          loop  ;; label = @4
            local.get 2
            local.get 4
            local.tee 3
            i32.add
            local.tee 5
            i32.const 48
            i32.const 55
            local.get 0
            i32.const 15
            i32.and
            local.tee 4
            i32.const 10
            i32.lt_u
            select
            local.get 4
            i32.add
            i32.store8
            local.get 3
            i32.const -1
            i32.add
            local.set 4
            local.get 0
            i32.const 16
            i32.lt_u
            local.set 6
            local.get 0
            i32.const 4
            i32.shr_u
            local.set 0
            local.get 6
            i32.eqz
            br_if 0 (;@4;)
          end
          local.get 3
          i32.const 128
          i32.gt_u
          br_if 2 (;@1;)
          local.get 1
          i32.const 1050084
          i32.const 2
          local.get 5
          i32.const 129
          local.get 3
          i32.const 1
          i32.add
          i32.sub
          call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
          local.set 0
        end
        local.get 2
        i32.const 128
        i32.add
        global.set $__stack_pointer
        local.get 0
        return
      end
      local.get 3
      i32.const 128
      i32.const 1050068
      call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
      unreachable
    end
    local.get 3
    i32.const 128
    i32.const 1050068
    call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
    unreachable)
  (func $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E (type 11) (param i32 i32 i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32)
    local.get 0
    i32.load offset=28
    local.tee 5
    i32.const 1
    i32.and
    local.tee 6
    local.get 4
    i32.add
    local.set 7
    block  ;; label = @1
      block  ;; label = @2
        local.get 5
        i32.const 4
        i32.and
        br_if 0 (;@2;)
        i32.const 0
        local.set 1
        br 1 (;@1;)
      end
      block  ;; label = @2
        block  ;; label = @3
          local.get 2
          br_if 0 (;@3;)
          i32.const 0
          local.set 8
          br 1 (;@2;)
        end
        block  ;; label = @3
          local.get 2
          i32.const 3
          i32.and
          local.tee 9
          br_if 0 (;@3;)
          br 1 (;@2;)
        end
        i32.const 0
        local.set 8
        local.get 1
        local.set 10
        loop  ;; label = @3
          local.get 8
          local.get 10
          i32.load8_s
          i32.const -65
          i32.gt_s
          i32.add
          local.set 8
          local.get 10
          i32.const 1
          i32.add
          local.set 10
          local.get 9
          i32.const -1
          i32.add
          local.tee 9
          br_if 0 (;@3;)
        end
      end
      local.get 8
      local.get 7
      i32.add
      local.set 7
    end
    i32.const 43
    i32.const 1114112
    local.get 6
    select
    local.set 6
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i32.load
        br_if 0 (;@2;)
        i32.const 1
        local.set 10
        local.get 0
        i32.load offset=20
        local.tee 8
        local.get 0
        i32.load offset=24
        local.tee 9
        local.get 6
        local.get 1
        local.get 2
        call $_ZN4core3fmt9Formatter12pad_integral12write_prefix17h43684999422d0638E
        br_if 1 (;@1;)
        local.get 8
        local.get 3
        local.get 4
        local.get 9
        i32.load offset=12
        call_indirect (type 1)
        return
      end
      block  ;; label = @2
        local.get 0
        i32.load offset=4
        local.tee 11
        local.get 7
        i32.gt_u
        br_if 0 (;@2;)
        i32.const 1
        local.set 10
        local.get 0
        i32.load offset=20
        local.tee 8
        local.get 0
        i32.load offset=24
        local.tee 9
        local.get 6
        local.get 1
        local.get 2
        call $_ZN4core3fmt9Formatter12pad_integral12write_prefix17h43684999422d0638E
        br_if 1 (;@1;)
        local.get 8
        local.get 3
        local.get 4
        local.get 9
        i32.load offset=12
        call_indirect (type 1)
        return
      end
      block  ;; label = @2
        local.get 5
        i32.const 8
        i32.and
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        i32.load offset=16
        local.set 5
        local.get 0
        i32.const 48
        i32.store offset=16
        local.get 0
        i32.load8_u offset=32
        local.set 12
        i32.const 1
        local.set 10
        local.get 0
        i32.const 1
        i32.store8 offset=32
        local.get 0
        i32.load offset=20
        local.tee 8
        local.get 0
        i32.load offset=24
        local.tee 9
        local.get 6
        local.get 1
        local.get 2
        call $_ZN4core3fmt9Formatter12pad_integral12write_prefix17h43684999422d0638E
        br_if 1 (;@1;)
        local.get 11
        local.get 7
        i32.sub
        i32.const 1
        i32.add
        local.set 10
        block  ;; label = @3
          loop  ;; label = @4
            local.get 10
            i32.const -1
            i32.add
            local.tee 10
            i32.eqz
            br_if 1 (;@3;)
            local.get 8
            i32.const 48
            local.get 9
            i32.load offset=16
            call_indirect (type 2)
            i32.eqz
            br_if 0 (;@4;)
          end
          i32.const 1
          return
        end
        i32.const 1
        local.set 10
        local.get 8
        local.get 3
        local.get 4
        local.get 9
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        local.get 0
        local.get 12
        i32.store8 offset=32
        local.get 0
        local.get 5
        i32.store offset=16
        i32.const 0
        local.set 10
        br 1 (;@1;)
      end
      local.get 11
      local.get 7
      i32.sub
      local.set 5
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 0
            i32.load8_u offset=32
            local.tee 10
            br_table 2 (;@2;) 0 (;@4;) 1 (;@3;) 0 (;@4;) 2 (;@2;)
          end
          local.get 5
          local.set 10
          i32.const 0
          local.set 5
          br 1 (;@2;)
        end
        local.get 5
        i32.const 1
        i32.shr_u
        local.set 10
        local.get 5
        i32.const 1
        i32.add
        i32.const 1
        i32.shr_u
        local.set 5
      end
      local.get 10
      i32.const 1
      i32.add
      local.set 10
      local.get 0
      i32.const 24
      i32.add
      i32.load
      local.set 8
      local.get 0
      i32.load offset=16
      local.set 7
      local.get 0
      i32.load offset=20
      local.set 9
      block  ;; label = @2
        loop  ;; label = @3
          local.get 10
          i32.const -1
          i32.add
          local.tee 10
          i32.eqz
          br_if 1 (;@2;)
          local.get 9
          local.get 7
          local.get 8
          i32.load offset=16
          call_indirect (type 2)
          i32.eqz
          br_if 0 (;@3;)
        end
        i32.const 1
        return
      end
      i32.const 1
      local.set 10
      local.get 9
      local.get 8
      local.get 6
      local.get 1
      local.get 2
      call $_ZN4core3fmt9Formatter12pad_integral12write_prefix17h43684999422d0638E
      br_if 0 (;@1;)
      local.get 9
      local.get 3
      local.get 4
      local.get 8
      i32.load offset=12
      call_indirect (type 1)
      br_if 0 (;@1;)
      i32.const 0
      local.set 10
      loop  ;; label = @2
        block  ;; label = @3
          local.get 5
          local.get 10
          i32.ne
          br_if 0 (;@3;)
          local.get 5
          local.get 5
          i32.lt_u
          return
        end
        local.get 10
        i32.const 1
        i32.add
        local.set 10
        local.get 9
        local.get 7
        local.get 8
        i32.load offset=16
        call_indirect (type 2)
        i32.eqz
        br_if 0 (;@2;)
      end
      local.get 10
      i32.const -1
      i32.add
      local.get 5
      i32.lt_u
      return
    end
    local.get 10)
  (func $_ZN4core3ptr25drop_in_place$LT$char$GT$17hc0b8a240319b7b4fE (type 6) (param i32))
  (func $_ZN4core3ops8function6FnOnce9call_once17h3b98e4a5ab67f033E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    drop
    loop (result i32)  ;; label = @1
      br 0 (;@1;)
    end)
  (func $_ZN36_$LT$T$u20$as$u20$core..any..Any$GT$7type_id17hed637ffe26dba6a3E (type 5) (param i32 i32)
    local.get 0
    i64.const 568815540544143123
    i64.store offset=8
    local.get 0
    i64.const 5657071353825360256
    i64.store)
  (func $_ZN4core3fmt3num3imp7fmt_u6417h416abf9443aa8afdE (type 12) (param i64 i32) (result i32)
    (local i32 i32 i64 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    i32.const 39
    local.set 3
    block  ;; label = @1
      block  ;; label = @2
        local.get 0
        i64.const 10000
        i64.ge_u
        br_if 0 (;@2;)
        local.get 0
        local.set 4
        br 1 (;@1;)
      end
      i32.const 39
      local.set 3
      loop  ;; label = @2
        local.get 2
        i32.const 9
        i32.add
        local.get 3
        i32.add
        local.tee 5
        i32.const -4
        i32.add
        local.get 0
        i64.const 10000
        i64.div_u
        local.tee 4
        i64.const 55536
        i64.mul
        local.get 0
        i64.add
        i32.wrap_i64
        local.tee 6
        i32.const 65535
        i32.and
        i32.const 100
        i32.div_u
        local.tee 7
        i32.const 1
        i32.shl
        i32.const 1050086
        i32.add
        i32.load16_u align=1
        i32.store16 align=1
        local.get 5
        i32.const -2
        i32.add
        local.get 7
        i32.const -100
        i32.mul
        local.get 6
        i32.add
        i32.const 65535
        i32.and
        i32.const 1
        i32.shl
        i32.const 1050086
        i32.add
        i32.load16_u align=1
        i32.store16 align=1
        local.get 3
        i32.const -4
        i32.add
        local.set 3
        local.get 0
        i64.const 99999999
        i64.gt_u
        local.set 5
        local.get 4
        local.set 0
        local.get 5
        br_if 0 (;@2;)
      end
    end
    block  ;; label = @1
      local.get 4
      i32.wrap_i64
      local.tee 5
      i32.const 99
      i32.le_u
      br_if 0 (;@1;)
      local.get 2
      i32.const 9
      i32.add
      local.get 3
      i32.const -2
      i32.add
      local.tee 3
      i32.add
      local.get 4
      i32.wrap_i64
      local.tee 6
      i32.const 65535
      i32.and
      i32.const 100
      i32.div_u
      local.tee 5
      i32.const -100
      i32.mul
      local.get 6
      i32.add
      i32.const 65535
      i32.and
      i32.const 1
      i32.shl
      i32.const 1050086
      i32.add
      i32.load16_u align=1
      i32.store16 align=1
    end
    block  ;; label = @1
      block  ;; label = @2
        local.get 5
        i32.const 10
        i32.lt_u
        br_if 0 (;@2;)
        local.get 2
        i32.const 9
        i32.add
        local.get 3
        i32.const -2
        i32.add
        local.tee 3
        i32.add
        local.get 5
        i32.const 1
        i32.shl
        i32.const 1050086
        i32.add
        i32.load16_u align=1
        i32.store16 align=1
        br 1 (;@1;)
      end
      local.get 2
      i32.const 9
      i32.add
      local.get 3
      i32.const -1
      i32.add
      local.tee 3
      i32.add
      local.get 5
      i32.const 48
      i32.add
      i32.store8
    end
    local.get 1
    i32.const 1049804
    i32.const 0
    local.get 2
    i32.const 9
    i32.add
    local.get 3
    i32.add
    i32.const 39
    local.get 3
    i32.sub
    call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
    local.set 3
    local.get 2
    i32.const 48
    i32.add
    global.set $__stack_pointer
    local.get 3)
  (func $_ZN4core3fmt9Formatter12pad_integral12write_prefix17h43684999422d0638E (type 11) (param i32 i32 i32 i32 i32) (result i32)
    (local i32)
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 2
          i32.const 1114112
          i32.eq
          br_if 0 (;@3;)
          i32.const 1
          local.set 5
          local.get 0
          local.get 2
          local.get 1
          i32.load offset=16
          call_indirect (type 2)
          br_if 1 (;@2;)
        end
        local.get 3
        br_if 1 (;@1;)
        i32.const 0
        local.set 5
      end
      local.get 5
      return
    end
    local.get 0
    local.get 3
    local.get 4
    local.get 1
    i32.load offset=12
    call_indirect (type 1))
  (func $_ZN4core5slice5index24slice_end_index_len_fail17h6372e465cf26b33aE (type 5) (param i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    local.get 2
    local.get 0
    i32.store
    local.get 2
    local.get 1
    i32.store offset=4
    local.get 2
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 2
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 3
    i32.store
    local.get 2
    i32.const 2
    i32.store offset=12
    local.get 2
    i32.const 1050404
    i32.store offset=8
    local.get 2
    i32.const 3
    i32.store offset=36
    local.get 2
    local.get 2
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 2
    local.get 2
    i32.const 4
    i32.add
    i32.store offset=40
    local.get 2
    local.get 2
    i32.store offset=32
    local.get 2
    i32.const 8
    i32.add
    i32.const 1050884
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN4core9panicking5panic17hdd77bb12897b1389E (type 6) (param i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    i32.const 12
    i32.add
    i64.const 0
    i64.store align=4
    local.get 1
    i32.const 1
    i32.store offset=4
    local.get 1
    i32.const 1049804
    i32.store offset=8
    local.get 1
    i32.const 43
    i32.store offset=28
    local.get 1
    i32.const 1049804
    i32.store offset=24
    local.get 1
    local.get 1
    i32.const 24
    i32.add
    i32.store
    local.get 1
    local.get 0
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN44_$LT$$RF$T$u20$as$u20$core..fmt..Display$GT$3fmt17h743d7417ec5e8687E (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    local.get 0
    i32.load offset=4
    local.set 2
    local.get 0
    i32.load
    local.set 3
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          i32.load
          local.tee 4
          local.get 1
          i32.load offset=8
          local.tee 0
          i32.or
          i32.eqz
          br_if 0 (;@3;)
          block  ;; label = @4
            local.get 0
            i32.eqz
            br_if 0 (;@4;)
            local.get 3
            local.get 2
            i32.add
            local.set 5
            local.get 1
            i32.const 12
            i32.add
            i32.load
            i32.const 1
            i32.add
            local.set 6
            i32.const 0
            local.set 7
            local.get 3
            local.set 8
            block  ;; label = @5
              loop  ;; label = @6
                local.get 8
                local.set 0
                local.get 6
                i32.const -1
                i32.add
                local.tee 6
                i32.eqz
                br_if 1 (;@5;)
                local.get 0
                local.get 5
                i32.eq
                br_if 2 (;@4;)
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 0
                    i32.load8_s
                    local.tee 9
                    i32.const -1
                    i32.le_s
                    br_if 0 (;@8;)
                    local.get 0
                    i32.const 1
                    i32.add
                    local.set 8
                    local.get 9
                    i32.const 255
                    i32.and
                    local.set 9
                    br 1 (;@7;)
                  end
                  local.get 0
                  i32.load8_u offset=1
                  i32.const 63
                  i32.and
                  local.set 10
                  local.get 9
                  i32.const 31
                  i32.and
                  local.set 8
                  block  ;; label = @8
                    local.get 9
                    i32.const -33
                    i32.gt_u
                    br_if 0 (;@8;)
                    local.get 8
                    i32.const 6
                    i32.shl
                    local.get 10
                    i32.or
                    local.set 9
                    local.get 0
                    i32.const 2
                    i32.add
                    local.set 8
                    br 1 (;@7;)
                  end
                  local.get 10
                  i32.const 6
                  i32.shl
                  local.get 0
                  i32.load8_u offset=2
                  i32.const 63
                  i32.and
                  i32.or
                  local.set 10
                  block  ;; label = @8
                    local.get 9
                    i32.const -16
                    i32.ge_u
                    br_if 0 (;@8;)
                    local.get 10
                    local.get 8
                    i32.const 12
                    i32.shl
                    i32.or
                    local.set 9
                    local.get 0
                    i32.const 3
                    i32.add
                    local.set 8
                    br 1 (;@7;)
                  end
                  local.get 10
                  i32.const 6
                  i32.shl
                  local.get 0
                  i32.load8_u offset=3
                  i32.const 63
                  i32.and
                  i32.or
                  local.get 8
                  i32.const 18
                  i32.shl
                  i32.const 1835008
                  i32.and
                  i32.or
                  local.tee 9
                  i32.const 1114112
                  i32.eq
                  br_if 3 (;@4;)
                  local.get 0
                  i32.const 4
                  i32.add
                  local.set 8
                end
                local.get 7
                local.get 0
                i32.sub
                local.get 8
                i32.add
                local.set 7
                local.get 9
                i32.const 1114112
                i32.ne
                br_if 0 (;@6;)
                br 2 (;@4;)
              end
            end
            local.get 0
            local.get 5
            i32.eq
            br_if 0 (;@4;)
            block  ;; label = @5
              local.get 0
              i32.load8_s
              local.tee 8
              i32.const -1
              i32.gt_s
              br_if 0 (;@5;)
              local.get 8
              i32.const -32
              i32.lt_u
              br_if 0 (;@5;)
              local.get 8
              i32.const -16
              i32.lt_u
              br_if 0 (;@5;)
              local.get 0
              i32.load8_u offset=2
              i32.const 63
              i32.and
              i32.const 6
              i32.shl
              local.get 0
              i32.load8_u offset=1
              i32.const 63
              i32.and
              i32.const 12
              i32.shl
              i32.or
              local.get 0
              i32.load8_u offset=3
              i32.const 63
              i32.and
              i32.or
              local.get 8
              i32.const 255
              i32.and
              i32.const 18
              i32.shl
              i32.const 1835008
              i32.and
              i32.or
              i32.const 1114112
              i32.eq
              br_if 1 (;@4;)
            end
            block  ;; label = @5
              block  ;; label = @6
                local.get 7
                i32.eqz
                br_if 0 (;@6;)
                block  ;; label = @7
                  local.get 7
                  local.get 2
                  i32.lt_u
                  br_if 0 (;@7;)
                  i32.const 0
                  local.set 0
                  local.get 7
                  local.get 2
                  i32.eq
                  br_if 1 (;@6;)
                  br 2 (;@5;)
                end
                i32.const 0
                local.set 0
                local.get 3
                local.get 7
                i32.add
                i32.load8_s
                i32.const -64
                i32.lt_s
                br_if 1 (;@5;)
              end
              local.get 3
              local.set 0
            end
            local.get 7
            local.get 2
            local.get 0
            select
            local.set 2
            local.get 0
            local.get 3
            local.get 0
            select
            local.set 3
          end
          block  ;; label = @4
            local.get 4
            br_if 0 (;@4;)
            local.get 1
            i32.load offset=20
            local.get 3
            local.get 2
            local.get 1
            i32.const 24
            i32.add
            i32.load
            i32.load offset=12
            call_indirect (type 1)
            return
          end
          local.get 1
          i32.load offset=4
          local.set 11
          block  ;; label = @4
            local.get 2
            i32.const 16
            i32.lt_u
            br_if 0 (;@4;)
            local.get 2
            local.get 3
            local.get 3
            i32.const 3
            i32.add
            i32.const -4
            i32.and
            local.tee 9
            i32.sub
            local.tee 6
            i32.add
            local.tee 4
            i32.const 3
            i32.and
            local.set 5
            i32.const 0
            local.set 10
            i32.const 0
            local.set 0
            block  ;; label = @5
              local.get 3
              local.get 9
              i32.eq
              br_if 0 (;@5;)
              i32.const 0
              local.set 0
              block  ;; label = @6
                local.get 9
                local.get 3
                i32.const -1
                i32.xor
                i32.add
                i32.const 3
                i32.lt_u
                br_if 0 (;@6;)
                i32.const 0
                local.set 0
                i32.const 0
                local.set 7
                loop  ;; label = @7
                  local.get 0
                  local.get 3
                  local.get 7
                  i32.add
                  local.tee 8
                  i32.load8_s
                  i32.const -65
                  i32.gt_s
                  i32.add
                  local.get 8
                  i32.const 1
                  i32.add
                  i32.load8_s
                  i32.const -65
                  i32.gt_s
                  i32.add
                  local.get 8
                  i32.const 2
                  i32.add
                  i32.load8_s
                  i32.const -65
                  i32.gt_s
                  i32.add
                  local.get 8
                  i32.const 3
                  i32.add
                  i32.load8_s
                  i32.const -65
                  i32.gt_s
                  i32.add
                  local.set 0
                  local.get 7
                  i32.const 4
                  i32.add
                  local.tee 7
                  br_if 0 (;@7;)
                end
              end
              local.get 3
              local.set 8
              loop  ;; label = @6
                local.get 0
                local.get 8
                i32.load8_s
                i32.const -65
                i32.gt_s
                i32.add
                local.set 0
                local.get 8
                i32.const 1
                i32.add
                local.set 8
                local.get 6
                i32.const 1
                i32.add
                local.tee 6
                br_if 0 (;@6;)
              end
            end
            block  ;; label = @5
              local.get 5
              i32.eqz
              br_if 0 (;@5;)
              local.get 9
              local.get 4
              i32.const -4
              i32.and
              i32.add
              local.tee 8
              i32.load8_s
              i32.const -65
              i32.gt_s
              local.set 10
              local.get 5
              i32.const 1
              i32.eq
              br_if 0 (;@5;)
              local.get 10
              local.get 8
              i32.load8_s offset=1
              i32.const -65
              i32.gt_s
              i32.add
              local.set 10
              local.get 5
              i32.const 2
              i32.eq
              br_if 0 (;@5;)
              local.get 10
              local.get 8
              i32.load8_s offset=2
              i32.const -65
              i32.gt_s
              i32.add
              local.set 10
            end
            local.get 4
            i32.const 2
            i32.shr_u
            local.set 5
            local.get 10
            local.get 0
            i32.add
            local.set 7
            loop  ;; label = @5
              local.get 9
              local.set 4
              local.get 5
              i32.eqz
              br_if 4 (;@1;)
              local.get 5
              i32.const 192
              local.get 5
              i32.const 192
              i32.lt_u
              select
              local.tee 10
              i32.const 3
              i32.and
              local.set 12
              local.get 10
              i32.const 2
              i32.shl
              local.set 13
              i32.const 0
              local.set 8
              block  ;; label = @6
                local.get 10
                i32.const 4
                i32.lt_u
                br_if 0 (;@6;)
                local.get 4
                local.get 13
                i32.const 1008
                i32.and
                i32.add
                local.set 6
                i32.const 0
                local.set 8
                local.get 4
                local.set 0
                loop  ;; label = @7
                  local.get 0
                  i32.const 12
                  i32.add
                  i32.load
                  local.tee 9
                  i32.const -1
                  i32.xor
                  i32.const 7
                  i32.shr_u
                  local.get 9
                  i32.const 6
                  i32.shr_u
                  i32.or
                  i32.const 16843009
                  i32.and
                  local.get 0
                  i32.const 8
                  i32.add
                  i32.load
                  local.tee 9
                  i32.const -1
                  i32.xor
                  i32.const 7
                  i32.shr_u
                  local.get 9
                  i32.const 6
                  i32.shr_u
                  i32.or
                  i32.const 16843009
                  i32.and
                  local.get 0
                  i32.const 4
                  i32.add
                  i32.load
                  local.tee 9
                  i32.const -1
                  i32.xor
                  i32.const 7
                  i32.shr_u
                  local.get 9
                  i32.const 6
                  i32.shr_u
                  i32.or
                  i32.const 16843009
                  i32.and
                  local.get 0
                  i32.load
                  local.tee 9
                  i32.const -1
                  i32.xor
                  i32.const 7
                  i32.shr_u
                  local.get 9
                  i32.const 6
                  i32.shr_u
                  i32.or
                  i32.const 16843009
                  i32.and
                  local.get 8
                  i32.add
                  i32.add
                  i32.add
                  i32.add
                  local.set 8
                  local.get 0
                  i32.const 16
                  i32.add
                  local.tee 0
                  local.get 6
                  i32.ne
                  br_if 0 (;@7;)
                end
              end
              local.get 5
              local.get 10
              i32.sub
              local.set 5
              local.get 4
              local.get 13
              i32.add
              local.set 9
              local.get 8
              i32.const 8
              i32.shr_u
              i32.const 16711935
              i32.and
              local.get 8
              i32.const 16711935
              i32.and
              i32.add
              i32.const 65537
              i32.mul
              i32.const 16
              i32.shr_u
              local.get 7
              i32.add
              local.set 7
              local.get 12
              i32.eqz
              br_if 0 (;@5;)
            end
            local.get 4
            local.get 10
            i32.const 252
            i32.and
            i32.const 2
            i32.shl
            i32.add
            local.tee 8
            i32.load
            local.tee 0
            i32.const -1
            i32.xor
            i32.const 7
            i32.shr_u
            local.get 0
            i32.const 6
            i32.shr_u
            i32.or
            i32.const 16843009
            i32.and
            local.set 0
            local.get 12
            i32.const 1
            i32.eq
            br_if 2 (;@2;)
            local.get 8
            i32.load offset=4
            local.tee 9
            i32.const -1
            i32.xor
            i32.const 7
            i32.shr_u
            local.get 9
            i32.const 6
            i32.shr_u
            i32.or
            i32.const 16843009
            i32.and
            local.get 0
            i32.add
            local.set 0
            local.get 12
            i32.const 2
            i32.eq
            br_if 2 (;@2;)
            local.get 8
            i32.load offset=8
            local.tee 8
            i32.const -1
            i32.xor
            i32.const 7
            i32.shr_u
            local.get 8
            i32.const 6
            i32.shr_u
            i32.or
            i32.const 16843009
            i32.and
            local.get 0
            i32.add
            local.set 0
            br 2 (;@2;)
          end
          block  ;; label = @4
            local.get 2
            br_if 0 (;@4;)
            i32.const 0
            local.set 7
            br 3 (;@1;)
          end
          local.get 2
          i32.const 3
          i32.and
          local.set 8
          block  ;; label = @4
            block  ;; label = @5
              local.get 2
              i32.const 4
              i32.ge_u
              br_if 0 (;@5;)
              i32.const 0
              local.set 7
              i32.const 0
              local.set 6
              br 1 (;@4;)
            end
            i32.const 0
            local.set 7
            local.get 3
            local.set 0
            local.get 2
            i32.const -4
            i32.and
            local.tee 6
            local.set 9
            loop  ;; label = @5
              local.get 7
              local.get 0
              i32.load8_s
              i32.const -65
              i32.gt_s
              i32.add
              local.get 0
              i32.const 1
              i32.add
              i32.load8_s
              i32.const -65
              i32.gt_s
              i32.add
              local.get 0
              i32.const 2
              i32.add
              i32.load8_s
              i32.const -65
              i32.gt_s
              i32.add
              local.get 0
              i32.const 3
              i32.add
              i32.load8_s
              i32.const -65
              i32.gt_s
              i32.add
              local.set 7
              local.get 0
              i32.const 4
              i32.add
              local.set 0
              local.get 9
              i32.const -4
              i32.add
              local.tee 9
              br_if 0 (;@5;)
            end
          end
          local.get 8
          i32.eqz
          br_if 2 (;@1;)
          local.get 3
          local.get 6
          i32.add
          local.set 0
          loop  ;; label = @4
            local.get 7
            local.get 0
            i32.load8_s
            i32.const -65
            i32.gt_s
            i32.add
            local.set 7
            local.get 0
            i32.const 1
            i32.add
            local.set 0
            local.get 8
            i32.const -1
            i32.add
            local.tee 8
            br_if 0 (;@4;)
            br 3 (;@1;)
          end
        end
        local.get 1
        i32.load offset=20
        local.get 3
        local.get 2
        local.get 1
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        return
      end
      local.get 0
      i32.const 8
      i32.shr_u
      i32.const 459007
      i32.and
      local.get 0
      i32.const 16711935
      i32.and
      i32.add
      i32.const 65537
      i32.mul
      i32.const 16
      i32.shr_u
      local.get 7
      i32.add
      local.set 7
    end
    block  ;; label = @1
      block  ;; label = @2
        local.get 11
        local.get 7
        i32.le_u
        br_if 0 (;@2;)
        local.get 11
        local.get 7
        i32.sub
        local.set 7
        i32.const 0
        local.set 0
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 1
              i32.load8_u offset=32
              br_table 2 (;@3;) 0 (;@5;) 1 (;@4;) 2 (;@3;) 2 (;@3;)
            end
            local.get 7
            local.set 0
            i32.const 0
            local.set 7
            br 1 (;@3;)
          end
          local.get 7
          i32.const 1
          i32.shr_u
          local.set 0
          local.get 7
          i32.const 1
          i32.add
          i32.const 1
          i32.shr_u
          local.set 7
        end
        local.get 0
        i32.const 1
        i32.add
        local.set 0
        local.get 1
        i32.const 24
        i32.add
        i32.load
        local.set 8
        local.get 1
        i32.load offset=16
        local.set 6
        local.get 1
        i32.load offset=20
        local.set 9
        loop  ;; label = @3
          local.get 0
          i32.const -1
          i32.add
          local.tee 0
          i32.eqz
          br_if 2 (;@1;)
          local.get 9
          local.get 6
          local.get 8
          i32.load offset=16
          call_indirect (type 2)
          i32.eqz
          br_if 0 (;@3;)
        end
        i32.const 1
        return
      end
      local.get 1
      i32.load offset=20
      local.get 3
      local.get 2
      local.get 1
      i32.const 24
      i32.add
      i32.load
      i32.load offset=12
      call_indirect (type 1)
      return
    end
    i32.const 1
    local.set 0
    block  ;; label = @1
      local.get 9
      local.get 3
      local.get 2
      local.get 8
      i32.load offset=12
      call_indirect (type 1)
      br_if 0 (;@1;)
      i32.const 0
      local.set 0
      block  ;; label = @2
        loop  ;; label = @3
          block  ;; label = @4
            local.get 7
            local.get 0
            i32.ne
            br_if 0 (;@4;)
            local.get 7
            local.set 0
            br 2 (;@2;)
          end
          local.get 0
          i32.const 1
          i32.add
          local.set 0
          local.get 9
          local.get 6
          local.get 8
          i32.load offset=16
          call_indirect (type 2)
          i32.eqz
          br_if 0 (;@3;)
        end
        local.get 0
        i32.const -1
        i32.add
        local.set 0
      end
      local.get 0
      local.get 7
      i32.lt_u
      local.set 0
    end
    local.get 0)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h02a298d5b218d667E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    local.get 1
    local.get 0
    i32.load offset=4
    i32.load offset=12
    call_indirect (type 2))
  (func $_ZN4core3fmt5write17h85c2d164d6b9d548E (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    i32.const 36
    i32.add
    local.get 1
    i32.store
    local.get 3
    i32.const 3
    i32.store8 offset=44
    local.get 3
    i32.const 32
    i32.store offset=28
    i32.const 0
    local.set 4
    local.get 3
    i32.const 0
    i32.store offset=40
    local.get 3
    local.get 0
    i32.store offset=32
    local.get 3
    i32.const 0
    i32.store offset=20
    local.get 3
    i32.const 0
    i32.store offset=12
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 2
              i32.load offset=16
              local.tee 5
              br_if 0 (;@5;)
              local.get 2
              i32.const 12
              i32.add
              i32.load
              local.tee 0
              i32.eqz
              br_if 1 (;@4;)
              local.get 2
              i32.load offset=8
              local.tee 1
              local.get 0
              i32.const 3
              i32.shl
              i32.add
              local.set 6
              local.get 0
              i32.const -1
              i32.add
              i32.const 536870911
              i32.and
              i32.const 1
              i32.add
              local.set 4
              local.get 2
              i32.load
              local.set 0
              loop  ;; label = @6
                block  ;; label = @7
                  local.get 0
                  i32.const 4
                  i32.add
                  i32.load
                  local.tee 7
                  i32.eqz
                  br_if 0 (;@7;)
                  local.get 3
                  i32.load offset=32
                  local.get 0
                  i32.load
                  local.get 7
                  local.get 3
                  i32.load offset=36
                  i32.load offset=12
                  call_indirect (type 1)
                  br_if 4 (;@3;)
                end
                local.get 1
                i32.load
                local.get 3
                i32.const 12
                i32.add
                local.get 1
                i32.const 4
                i32.add
                i32.load
                call_indirect (type 2)
                br_if 3 (;@3;)
                local.get 0
                i32.const 8
                i32.add
                local.set 0
                local.get 1
                i32.const 8
                i32.add
                local.tee 1
                local.get 6
                i32.ne
                br_if 0 (;@6;)
                br 2 (;@4;)
              end
            end
            local.get 2
            i32.const 20
            i32.add
            i32.load
            local.tee 1
            i32.eqz
            br_if 0 (;@4;)
            local.get 1
            i32.const 5
            i32.shl
            local.set 8
            local.get 1
            i32.const -1
            i32.add
            i32.const 134217727
            i32.and
            i32.const 1
            i32.add
            local.set 4
            local.get 2
            i32.load offset=8
            local.set 9
            local.get 2
            i32.load
            local.set 0
            i32.const 0
            local.set 7
            loop  ;; label = @5
              block  ;; label = @6
                local.get 0
                i32.const 4
                i32.add
                i32.load
                local.tee 1
                i32.eqz
                br_if 0 (;@6;)
                local.get 3
                i32.load offset=32
                local.get 0
                i32.load
                local.get 1
                local.get 3
                i32.load offset=36
                i32.load offset=12
                call_indirect (type 1)
                br_if 3 (;@3;)
              end
              local.get 3
              local.get 5
              local.get 7
              i32.add
              local.tee 1
              i32.const 16
              i32.add
              i32.load
              i32.store offset=28
              local.get 3
              local.get 1
              i32.const 28
              i32.add
              i32.load8_u
              i32.store8 offset=44
              local.get 3
              local.get 1
              i32.const 24
              i32.add
              i32.load
              i32.store offset=40
              local.get 1
              i32.const 12
              i32.add
              i32.load
              local.set 10
              i32.const 0
              local.set 11
              i32.const 0
              local.set 6
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 1
                    i32.const 8
                    i32.add
                    i32.load
                    br_table 1 (;@7;) 0 (;@8;) 2 (;@6;) 1 (;@7;)
                  end
                  local.get 10
                  i32.const 3
                  i32.shl
                  local.set 12
                  i32.const 0
                  local.set 6
                  local.get 9
                  local.get 12
                  i32.add
                  local.tee 12
                  i32.load offset=4
                  i32.const 4
                  i32.ne
                  br_if 1 (;@6;)
                  local.get 12
                  i32.load
                  i32.load
                  local.set 10
                end
                i32.const 1
                local.set 6
              end
              local.get 3
              local.get 10
              i32.store offset=16
              local.get 3
              local.get 6
              i32.store offset=12
              local.get 1
              i32.const 4
              i32.add
              i32.load
              local.set 6
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 1
                    i32.load
                    br_table 1 (;@7;) 0 (;@8;) 2 (;@6;) 1 (;@7;)
                  end
                  local.get 6
                  i32.const 3
                  i32.shl
                  local.set 10
                  local.get 9
                  local.get 10
                  i32.add
                  local.tee 10
                  i32.load offset=4
                  i32.const 4
                  i32.ne
                  br_if 1 (;@6;)
                  local.get 10
                  i32.load
                  i32.load
                  local.set 6
                end
                i32.const 1
                local.set 11
              end
              local.get 3
              local.get 6
              i32.store offset=24
              local.get 3
              local.get 11
              i32.store offset=20
              local.get 9
              local.get 1
              i32.const 20
              i32.add
              i32.load
              i32.const 3
              i32.shl
              i32.add
              local.tee 1
              i32.load
              local.get 3
              i32.const 12
              i32.add
              local.get 1
              i32.const 4
              i32.add
              i32.load
              call_indirect (type 2)
              br_if 2 (;@3;)
              local.get 0
              i32.const 8
              i32.add
              local.set 0
              local.get 8
              local.get 7
              i32.const 32
              i32.add
              local.tee 7
              i32.ne
              br_if 0 (;@5;)
            end
          end
          local.get 4
          local.get 2
          i32.load offset=4
          i32.ge_u
          br_if 1 (;@2;)
          local.get 3
          i32.load offset=32
          local.get 2
          i32.load
          local.get 4
          i32.const 3
          i32.shl
          i32.add
          local.tee 1
          i32.load
          local.get 1
          i32.load offset=4
          local.get 3
          i32.load offset=36
          i32.load offset=12
          call_indirect (type 1)
          i32.eqz
          br_if 1 (;@2;)
        end
        i32.const 1
        local.set 1
        br 1 (;@1;)
      end
      i32.const 0
      local.set 1
    end
    local.get 3
    i32.const 48
    i32.add
    global.set $__stack_pointer
    local.get 1)
  (func $_ZN4core3ptr102drop_in_place$LT$$RF$core..iter..adapters..copied..Copied$LT$core..slice..iter..Iter$LT$u8$GT$$GT$$GT$17h8916d5767b34df94E (type 6) (param i32))
  (func $_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u32$GT$3fmt17h021845e23eea6eaeE (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 1
              i32.load offset=28
              local.tee 3
              i32.const 16
              i32.and
              br_if 0 (;@5;)
              local.get 3
              i32.const 32
              i32.and
              br_if 1 (;@4;)
              local.get 0
              i64.extend_i32_u
              local.get 1
              call $_ZN4core3fmt3num3imp7fmt_u6417h416abf9443aa8afdE
              local.set 0
              br 2 (;@3;)
            end
            i32.const 127
            local.set 4
            loop  ;; label = @5
              local.get 2
              local.get 4
              local.tee 3
              i32.add
              local.tee 5
              i32.const 48
              i32.const 87
              local.get 0
              i32.const 15
              i32.and
              local.tee 4
              i32.const 10
              i32.lt_u
              select
              local.get 4
              i32.add
              i32.store8
              local.get 3
              i32.const -1
              i32.add
              local.set 4
              local.get 0
              i32.const 16
              i32.lt_u
              local.set 6
              local.get 0
              i32.const 4
              i32.shr_u
              local.set 0
              local.get 6
              i32.eqz
              br_if 0 (;@5;)
            end
            local.get 3
            i32.const 128
            i32.gt_u
            br_if 2 (;@2;)
            local.get 1
            i32.const 1050084
            i32.const 2
            local.get 5
            i32.const 129
            local.get 3
            i32.const 1
            i32.add
            i32.sub
            call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
            local.set 0
            br 1 (;@3;)
          end
          i32.const 127
          local.set 4
          loop  ;; label = @4
            local.get 2
            local.get 4
            local.tee 3
            i32.add
            local.tee 5
            i32.const 48
            i32.const 55
            local.get 0
            i32.const 15
            i32.and
            local.tee 4
            i32.const 10
            i32.lt_u
            select
            local.get 4
            i32.add
            i32.store8
            local.get 3
            i32.const -1
            i32.add
            local.set 4
            local.get 0
            i32.const 16
            i32.lt_u
            local.set 6
            local.get 0
            i32.const 4
            i32.shr_u
            local.set 0
            local.get 6
            i32.eqz
            br_if 0 (;@4;)
          end
          local.get 3
          i32.const 128
          i32.gt_u
          br_if 2 (;@1;)
          local.get 1
          i32.const 1050084
          i32.const 2
          local.get 5
          i32.const 129
          local.get 3
          i32.const 1
          i32.add
          i32.sub
          call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
          local.set 0
        end
        local.get 2
        i32.const 128
        i32.add
        global.set $__stack_pointer
        local.get 0
        return
      end
      local.get 3
      i32.const 128
      i32.const 1050068
      call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
      unreachable
    end
    local.get 3
    i32.const 128
    i32.const 1050068
    call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
    unreachable)
  (func $_ZN4core5slice5index22slice_index_order_fail17h5c05174755728e22E (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    local.get 0
    i32.store
    local.get 3
    local.get 1
    i32.store offset=4
    local.get 3
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 3
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 3
    i32.store
    local.get 3
    i32.const 2
    i32.store offset=12
    local.get 3
    i32.const 1050456
    i32.store offset=8
    local.get 3
    i32.const 3
    i32.store offset=36
    local.get 3
    local.get 3
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 3
    local.get 3
    i32.const 4
    i32.add
    i32.store offset=40
    local.get 3
    local.get 3
    i32.store offset=32
    local.get 3
    i32.const 8
    i32.add
    local.get 2
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN4core6result13unwrap_failed17h38a5f72e87633eadE (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    i32.const 43
    i32.store offset=12
    local.get 3
    i32.const 1048628
    i32.store offset=8
    local.get 3
    local.get 1
    i32.store offset=20
    local.get 3
    local.get 0
    i32.store offset=16
    local.get 3
    i32.const 24
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 3
    i32.const 48
    i32.add
    i32.const 12
    i32.add
    i32.const 5
    i32.store
    local.get 3
    i32.const 2
    i32.store offset=28
    local.get 3
    i32.const 1049976
    i32.store offset=24
    local.get 3
    i32.const 6
    i32.store offset=52
    local.get 3
    local.get 3
    i32.const 48
    i32.add
    i32.store offset=32
    local.get 3
    local.get 3
    i32.const 16
    i32.add
    i32.store offset=56
    local.get 3
    local.get 3
    i32.const 8
    i32.add
    i32.store offset=48
    local.get 3
    i32.const 24
    i32.add
    local.get 2
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$10write_char17h2f9f00c342b8af4cE (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    local.get 0
    i32.load offset=4
    local.set 2
    local.get 0
    i32.load
    local.set 3
    block  ;; label = @1
      local.get 0
      i32.load offset=8
      local.tee 0
      i32.load8_u
      i32.eqz
      br_if 0 (;@1;)
      local.get 3
      i32.const 1050016
      i32.const 4
      local.get 2
      i32.load offset=12
      call_indirect (type 1)
      i32.eqz
      br_if 0 (;@1;)
      i32.const 1
      return
    end
    local.get 0
    local.get 1
    i32.const 10
    i32.eq
    i32.store8
    local.get 3
    local.get 1
    local.get 2
    i32.load offset=16
    call_indirect (type 2))
  (func $_ZN4core3fmt5Write9write_fmt17he38a55ddba5b1872E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.const 1049992
    local.get 1
    call $_ZN4core3fmt5write17h85c2d164d6b9d548E)
  (func $_ZN4core3fmt8builders10DebugTuple5field17hbc9d984656bef3fdE (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i64)
    global.get $__stack_pointer
    i32.const 64
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 0
    i32.load
    local.set 4
    i32.const 1
    local.set 5
    block  ;; label = @1
      local.get 0
      i32.load8_u offset=8
      br_if 0 (;@1;)
      block  ;; label = @2
        local.get 0
        i32.load offset=4
        local.tee 6
        i32.load offset=28
        local.tee 7
        i32.const 4
        i32.and
        br_if 0 (;@2;)
        i32.const 1
        local.set 5
        local.get 6
        i32.load offset=20
        i32.const 1050023
        i32.const 1050033
        local.get 4
        select
        i32.const 2
        i32.const 1
        local.get 4
        select
        local.get 6
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        local.get 1
        local.get 6
        local.get 2
        i32.load offset=12
        call_indirect (type 2)
        local.set 5
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 4
        br_if 0 (;@2;)
        i32.const 1
        local.set 5
        local.get 6
        i32.load offset=20
        i32.const 1050034
        i32.const 2
        local.get 6
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
        local.get 6
        i32.load offset=28
        local.set 7
      end
      i32.const 1
      local.set 5
      local.get 3
      i32.const 1
      i32.store8 offset=27
      local.get 3
      i32.const 52
      i32.add
      i32.const 1049992
      i32.store
      local.get 3
      local.get 6
      i64.load offset=20 align=4
      i64.store offset=12 align=4
      local.get 3
      local.get 3
      i32.const 27
      i32.add
      i32.store offset=20
      local.get 3
      local.get 6
      i64.load offset=8 align=4
      i64.store offset=36 align=4
      local.get 6
      i64.load align=4
      local.set 8
      local.get 3
      local.get 7
      i32.store offset=56
      local.get 3
      local.get 6
      i32.load offset=16
      i32.store offset=44
      local.get 3
      local.get 6
      i32.load8_u offset=32
      i32.store8 offset=60
      local.get 3
      local.get 8
      i64.store offset=28 align=4
      local.get 3
      local.get 3
      i32.const 12
      i32.add
      i32.store offset=48
      local.get 1
      local.get 3
      i32.const 28
      i32.add
      local.get 2
      i32.load offset=12
      call_indirect (type 2)
      br_if 0 (;@1;)
      local.get 3
      i32.load offset=48
      i32.const 1050028
      i32.const 2
      local.get 3
      i32.load offset=52
      i32.load offset=12
      call_indirect (type 1)
      local.set 5
    end
    local.get 0
    local.get 5
    i32.store8 offset=8
    local.get 0
    local.get 4
    i32.const 1
    i32.add
    i32.store
    local.get 3
    i32.const 64
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN4core4char7methods22_$LT$impl$u20$char$GT$16escape_debug_ext17hccd8a8eff60d1e48E (type 0) (param i32 i32 i32)
    (local i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 1
                        br_table 5 (;@5;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 1 (;@9;) 3 (;@7;) 8 (;@2;) 8 (;@2;) 2 (;@8;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 6 (;@4;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 8 (;@2;) 7 (;@3;) 0 (;@10;)
                      end
                      local.get 1
                      i32.const 92
                      i32.eq
                      br_if 3 (;@6;)
                      br 7 (;@2;)
                    end
                    local.get 0
                    i32.const 512
                    i32.store16 offset=10
                    local.get 0
                    i64.const 0
                    i64.store offset=2 align=2
                    local.get 0
                    i32.const 29788
                    i32.store16
                    br 7 (;@1;)
                  end
                  local.get 0
                  i32.const 512
                  i32.store16 offset=10
                  local.get 0
                  i64.const 0
                  i64.store offset=2 align=2
                  local.get 0
                  i32.const 29276
                  i32.store16
                  br 6 (;@1;)
                end
                local.get 0
                i32.const 512
                i32.store16 offset=10
                local.get 0
                i64.const 0
                i64.store offset=2 align=2
                local.get 0
                i32.const 28252
                i32.store16
                br 5 (;@1;)
              end
              local.get 0
              i32.const 512
              i32.store16 offset=10
              local.get 0
              i64.const 0
              i64.store offset=2 align=2
              local.get 0
              i32.const 23644
              i32.store16
              br 4 (;@1;)
            end
            local.get 0
            i32.const 512
            i32.store16 offset=10
            local.get 0
            i64.const 0
            i64.store offset=2 align=2
            local.get 0
            i32.const 12380
            i32.store16
            br 3 (;@1;)
          end
          local.get 2
          i32.const 65536
          i32.and
          i32.eqz
          br_if 1 (;@2;)
          local.get 0
          i32.const 512
          i32.store16 offset=10
          local.get 0
          i64.const 0
          i64.store offset=2 align=2
          local.get 0
          i32.const 8796
          i32.store16
          br 2 (;@1;)
        end
        local.get 2
        i32.const 256
        i32.and
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        i32.const 512
        i32.store16 offset=10
        local.get 0
        i64.const 0
        i64.store offset=2 align=2
        local.get 0
        i32.const 10076
        i32.store16
        br 1 (;@1;)
      end
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 2
                    i32.const 1
                    i32.and
                    i32.eqz
                    br_if 0 (;@8;)
                    local.get 1
                    i32.const 11
                    i32.shl
                    local.set 4
                    i32.const 0
                    local.set 2
                    i32.const 33
                    local.set 5
                    i32.const 33
                    local.set 6
                    block  ;; label = @9
                      block  ;; label = @10
                        loop  ;; label = @11
                          block  ;; label = @12
                            block  ;; label = @13
                              i32.const -1
                              local.get 5
                              i32.const 1
                              i32.shr_u
                              local.get 2
                              i32.add
                              local.tee 7
                              i32.const 2
                              i32.shl
                              i32.const 1052424
                              i32.add
                              i32.load
                              i32.const 11
                              i32.shl
                              local.tee 5
                              local.get 4
                              i32.ne
                              local.get 5
                              local.get 4
                              i32.lt_u
                              select
                              local.tee 5
                              i32.const 1
                              i32.ne
                              br_if 0 (;@13;)
                              local.get 7
                              local.set 6
                              br 1 (;@12;)
                            end
                            local.get 5
                            i32.const 255
                            i32.and
                            i32.const 255
                            i32.ne
                            br_if 2 (;@10;)
                            local.get 7
                            i32.const 1
                            i32.add
                            local.set 2
                          end
                          local.get 6
                          local.get 2
                          i32.sub
                          local.set 5
                          local.get 6
                          local.get 2
                          i32.gt_u
                          br_if 0 (;@11;)
                          br 2 (;@9;)
                        end
                      end
                      local.get 7
                      i32.const 1
                      i32.add
                      local.set 2
                    end
                    block  ;; label = @9
                      block  ;; label = @10
                        block  ;; label = @11
                          block  ;; label = @12
                            local.get 2
                            i32.const 32
                            i32.gt_u
                            br_if 0 (;@12;)
                            local.get 2
                            i32.const 2
                            i32.shl
                            local.tee 4
                            i32.const 1052424
                            i32.add
                            i32.load
                            i32.const 21
                            i32.shr_u
                            local.set 6
                            local.get 2
                            i32.const 32
                            i32.ne
                            br_if 1 (;@11;)
                            i32.const 31
                            local.set 2
                            i32.const 727
                            local.set 7
                            br 2 (;@10;)
                          end
                          i32.const 33
                          i32.const 33
                          i32.const 1052344
                          call $_ZN4core9panicking18panic_bounds_check17hcafbc5434e2cd720E
                          unreachable
                        end
                        local.get 4
                        i32.const 1052428
                        i32.add
                        i32.load
                        i32.const 21
                        i32.shr_u
                        local.set 7
                        block  ;; label = @11
                          local.get 2
                          br_if 0 (;@11;)
                          i32.const 0
                          local.set 2
                          br 2 (;@9;)
                        end
                        local.get 2
                        i32.const -1
                        i32.add
                        local.set 2
                      end
                      local.get 2
                      i32.const 2
                      i32.shl
                      i32.const 1052424
                      i32.add
                      i32.load
                      i32.const 2097151
                      i32.and
                      local.set 2
                    end
                    block  ;; label = @9
                      local.get 7
                      local.get 6
                      i32.const -1
                      i32.xor
                      i32.add
                      i32.eqz
                      br_if 0 (;@9;)
                      local.get 1
                      local.get 2
                      i32.sub
                      local.set 5
                      local.get 6
                      i32.const 727
                      local.get 6
                      i32.const 727
                      i32.gt_u
                      select
                      local.set 4
                      local.get 7
                      i32.const -1
                      i32.add
                      local.set 7
                      i32.const 0
                      local.set 2
                      loop  ;; label = @10
                        local.get 4
                        local.get 6
                        i32.eq
                        br_if 7 (;@3;)
                        local.get 2
                        local.get 6
                        i32.const 1052556
                        i32.add
                        i32.load8_u
                        i32.add
                        local.tee 2
                        local.get 5
                        i32.gt_u
                        br_if 1 (;@9;)
                        local.get 7
                        local.get 6
                        i32.const 1
                        i32.add
                        local.tee 6
                        i32.ne
                        br_if 0 (;@10;)
                      end
                      local.get 7
                      local.set 6
                    end
                    local.get 6
                    i32.const 1
                    i32.and
                    br_if 1 (;@7;)
                  end
                  local.get 1
                  i32.const 32
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const 127
                  i32.lt_u
                  br_if 3 (;@4;)
                  local.get 1
                  i32.const 65536
                  i32.lt_u
                  br_if 2 (;@5;)
                  local.get 1
                  i32.const 131072
                  i32.lt_u
                  br_if 1 (;@6;)
                  local.get 1
                  i32.const -205744
                  i32.add
                  i32.const 712016
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -201547
                  i32.add
                  i32.const 5
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -195102
                  i32.add
                  i32.const 1506
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -191457
                  i32.add
                  i32.const 3103
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -183970
                  i32.add
                  i32.const 14
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -2
                  i32.and
                  i32.const 178206
                  i32.eq
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -32
                  i32.and
                  i32.const 173792
                  i32.eq
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -177978
                  i32.add
                  i32.const 6
                  i32.lt_u
                  br_if 5 (;@2;)
                  local.get 1
                  i32.const -918000
                  i32.add
                  i32.const 196112
                  i32.lt_u
                  br_if 5 (;@2;)
                  br 3 (;@4;)
                end
                local.get 3
                i32.const 6
                i32.add
                i32.const 2
                i32.add
                i32.const 0
                i32.store8
                local.get 3
                i32.const 0
                i32.store16 offset=6
                local.get 3
                local.get 1
                i32.const 8
                i32.shr_u
                i32.const 15
                i32.and
                i32.const 1049888
                i32.add
                i32.load8_u
                i32.store8 offset=12
                local.get 3
                local.get 1
                i32.const 12
                i32.shr_u
                i32.const 15
                i32.and
                i32.const 1049888
                i32.add
                i32.load8_u
                i32.store8 offset=11
                local.get 3
                local.get 1
                i32.const 16
                i32.shr_u
                i32.const 15
                i32.and
                i32.const 1049888
                i32.add
                i32.load8_u
                i32.store8 offset=10
                local.get 3
                local.get 1
                i32.const 20
                i32.shr_u
                i32.const 15
                i32.and
                i32.const 1049888
                i32.add
                i32.load8_u
                i32.store8 offset=9
                local.get 3
                i32.const 6
                i32.add
                local.get 1
                i32.const 1
                i32.or
                i32.clz
                i32.const 2
                i32.shr_u
                i32.const -2
                i32.add
                local.tee 2
                i32.add
                local.tee 6
                i32.const 0
                i32.load16_u offset=1052402 align=1
                i32.store16 align=1
                local.get 3
                local.get 1
                i32.const 4
                i32.shr_u
                i32.const 15
                i32.and
                i32.const 1049888
                i32.add
                i32.load8_u
                i32.store8 offset=13
                local.get 6
                i32.const 2
                i32.add
                i32.const 0
                i32.load8_u offset=1052404
                i32.store8
                local.get 3
                i32.const 6
                i32.add
                i32.const 8
                i32.add
                local.tee 6
                local.get 1
                i32.const 15
                i32.and
                i32.const 1049888
                i32.add
                i32.load8_u
                i32.store8
                local.get 0
                local.get 3
                i64.load offset=6 align=2
                i64.store align=1
                local.get 3
                i32.const 125
                i32.store8 offset=15
                local.get 0
                i32.const 8
                i32.add
                local.get 6
                i32.load16_u
                i32.store16 align=1
                local.get 0
                i32.const 10
                i32.store8 offset=11
                local.get 0
                local.get 2
                i32.store8 offset=10
                br 5 (;@1;)
              end
              local.get 1
              i32.const 1050900
              i32.const 44
              i32.const 1050988
              i32.const 196
              i32.const 1051184
              i32.const 450
              call $_ZN4core7unicode9printable5check17he77c4eb9e45510bcE
              br_if 1 (;@4;)
              br 3 (;@2;)
            end
            local.get 1
            i32.const 1051634
            i32.const 40
            i32.const 1051714
            i32.const 287
            i32.const 1052001
            i32.const 303
            call $_ZN4core7unicode9printable5check17he77c4eb9e45510bcE
            i32.eqz
            br_if 2 (;@2;)
          end
          local.get 0
          local.get 1
          i32.store offset=4
          local.get 0
          i32.const 128
          i32.store8
          br 2 (;@1;)
        end
        local.get 4
        i32.const 727
        i32.const 1052360
        call $_ZN4core9panicking18panic_bounds_check17hcafbc5434e2cd720E
        unreachable
      end
      local.get 3
      i32.const 6
      i32.add
      i32.const 2
      i32.add
      i32.const 0
      i32.store8
      local.get 3
      i32.const 0
      i32.store16 offset=6
      local.get 3
      local.get 1
      i32.const 8
      i32.shr_u
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8 offset=12
      local.get 3
      local.get 1
      i32.const 12
      i32.shr_u
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8 offset=11
      local.get 3
      local.get 1
      i32.const 16
      i32.shr_u
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8 offset=10
      local.get 3
      local.get 1
      i32.const 20
      i32.shr_u
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8 offset=9
      local.get 3
      i32.const 6
      i32.add
      local.get 1
      i32.const 1
      i32.or
      i32.clz
      i32.const 2
      i32.shr_u
      i32.const -2
      i32.add
      local.tee 2
      i32.add
      local.tee 6
      i32.const 0
      i32.load16_u offset=1052402 align=1
      i32.store16 align=1
      local.get 3
      local.get 1
      i32.const 4
      i32.shr_u
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8 offset=13
      local.get 6
      i32.const 2
      i32.add
      i32.const 0
      i32.load8_u offset=1052404
      i32.store8
      local.get 3
      i32.const 6
      i32.add
      i32.const 8
      i32.add
      local.tee 6
      local.get 1
      i32.const 15
      i32.and
      i32.const 1049888
      i32.add
      i32.load8_u
      i32.store8
      local.get 0
      local.get 3
      i64.load offset=6 align=2
      i64.store align=1
      local.get 3
      i32.const 125
      i32.store8 offset=15
      local.get 0
      i32.const 8
      i32.add
      local.get 6
      i32.load16_u
      i32.store16 align=1
      local.get 0
      i32.const 10
      i32.store8 offset=11
      local.get 0
      local.get 2
      i32.store8 offset=10
    end
    local.get 3
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN4core3str16slice_error_fail17h5761aa5418ad8f04E (type 8) (param i32 i32 i32 i32 i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    local.get 4
    call $_ZN4core3str19slice_error_fail_rt17h18c07e16af596d85E
    unreachable)
  (func $_ZN4core7unicode9printable5check17he77c4eb9e45510bcE (type 13) (param i32 i32 i32 i32 i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32)
    local.get 1
    local.get 2
    i32.const 1
    i32.shl
    i32.add
    local.set 7
    local.get 0
    i32.const 65280
    i32.and
    i32.const 8
    i32.shr_u
    local.set 8
    i32.const 0
    local.set 9
    local.get 0
    i32.const 255
    i32.and
    local.set 10
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            loop  ;; label = @5
              local.get 1
              i32.const 2
              i32.add
              local.set 11
              local.get 9
              local.get 1
              i32.load8_u offset=1
              local.tee 2
              i32.add
              local.set 12
              block  ;; label = @6
                local.get 1
                i32.load8_u
                local.tee 1
                local.get 8
                i32.eq
                br_if 0 (;@6;)
                local.get 1
                local.get 8
                i32.gt_u
                br_if 4 (;@2;)
                local.get 12
                local.set 9
                local.get 11
                local.set 1
                local.get 11
                local.get 7
                i32.ne
                br_if 1 (;@5;)
                br 4 (;@2;)
              end
              local.get 9
              local.get 12
              i32.gt_u
              br_if 1 (;@4;)
              local.get 12
              local.get 4
              i32.gt_u
              br_if 2 (;@3;)
              local.get 3
              local.get 9
              i32.add
              local.set 1
              loop  ;; label = @6
                block  ;; label = @7
                  local.get 2
                  br_if 0 (;@7;)
                  local.get 12
                  local.set 9
                  local.get 11
                  local.set 1
                  local.get 11
                  local.get 7
                  i32.ne
                  br_if 2 (;@5;)
                  br 5 (;@2;)
                end
                local.get 2
                i32.const -1
                i32.add
                local.set 2
                local.get 1
                i32.load8_u
                local.set 9
                local.get 1
                i32.const 1
                i32.add
                local.set 1
                local.get 9
                local.get 10
                i32.ne
                br_if 0 (;@6;)
              end
            end
            i32.const 0
            local.set 2
            br 3 (;@1;)
          end
          local.get 9
          local.get 12
          i32.const 1050884
          call $_ZN4core5slice5index22slice_index_order_fail17h5c05174755728e22E
          unreachable
        end
        local.get 12
        local.get 4
        call $_ZN4core5slice5index24slice_end_index_len_fail17h6372e465cf26b33aE
        unreachable
      end
      local.get 0
      i32.const 65535
      i32.and
      local.set 9
      local.get 5
      local.get 6
      i32.add
      local.set 12
      i32.const 1
      local.set 2
      loop  ;; label = @2
        local.get 5
        i32.const 1
        i32.add
        local.set 10
        block  ;; label = @3
          block  ;; label = @4
            local.get 5
            i32.load8_u
            local.tee 1
            i32.extend8_s
            local.tee 11
            i32.const 0
            i32.lt_s
            br_if 0 (;@4;)
            local.get 10
            local.set 5
            br 1 (;@3;)
          end
          block  ;; label = @4
            local.get 10
            local.get 12
            i32.eq
            br_if 0 (;@4;)
            local.get 11
            i32.const 127
            i32.and
            i32.const 8
            i32.shl
            local.get 5
            i32.load8_u offset=1
            i32.or
            local.set 1
            local.get 5
            i32.const 2
            i32.add
            local.set 5
            br 1 (;@3;)
          end
          i32.const 1050868
          call $_ZN4core9panicking5panic17hdd77bb12897b1389E
          unreachable
        end
        local.get 9
        local.get 1
        i32.sub
        local.tee 9
        i32.const 0
        i32.lt_s
        br_if 1 (;@1;)
        local.get 2
        i32.const 1
        i32.xor
        local.set 2
        local.get 5
        local.get 12
        i32.ne
        br_if 0 (;@2;)
      end
    end
    local.get 2
    i32.const 1
    i32.and)
  (func $_ZN4core3str19slice_error_fail_rt17h18c07e16af596d85E (type 8) (param i32 i32 i32 i32 i32)
    (local i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 112
    i32.sub
    local.tee 5
    global.set $__stack_pointer
    local.get 5
    local.get 3
    i32.store offset=12
    local.get 5
    local.get 2
    i32.store offset=8
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          i32.const 257
          i32.lt_u
          br_if 0 (;@3;)
          i32.const 256
          local.set 6
          block  ;; label = @4
            local.get 0
            i32.load8_s offset=256
            i32.const -65
            i32.gt_s
            br_if 0 (;@4;)
            i32.const 255
            local.set 6
            local.get 0
            i32.load8_s offset=255
            i32.const -65
            i32.gt_s
            br_if 0 (;@4;)
            i32.const 254
            local.set 6
            local.get 0
            i32.load8_s offset=254
            i32.const -65
            i32.gt_s
            br_if 0 (;@4;)
            i32.const 253
            local.set 6
            local.get 0
            i32.load8_s offset=253
            i32.const -65
            i32.le_s
            br_if 2 (;@2;)
          end
          local.get 5
          local.get 6
          i32.store offset=20
          local.get 5
          local.get 0
          i32.store offset=16
          i32.const 5
          local.set 6
          i32.const 1050560
          local.set 7
          br 2 (;@1;)
        end
        local.get 5
        local.get 1
        i32.store offset=20
        local.get 5
        local.get 0
        i32.store offset=16
        i32.const 0
        local.set 6
        i32.const 1049804
        local.set 7
        br 1 (;@1;)
      end
      local.get 0
      local.get 1
      i32.const 0
      i32.const 253
      local.get 4
      call $_ZN4core3str16slice_error_fail17h5761aa5418ad8f04E
      unreachable
    end
    local.get 5
    local.get 6
    i32.store offset=28
    local.get 5
    local.get 7
    i32.store offset=24
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 2
              local.get 1
              i32.gt_u
              local.tee 6
              br_if 0 (;@5;)
              local.get 3
              local.get 1
              i32.gt_u
              br_if 0 (;@5;)
              local.get 2
              local.get 3
              i32.gt_u
              br_if 1 (;@4;)
              block  ;; label = @6
                block  ;; label = @7
                  local.get 2
                  i32.eqz
                  br_if 0 (;@7;)
                  local.get 2
                  local.get 1
                  i32.ge_u
                  br_if 0 (;@7;)
                  local.get 0
                  local.get 2
                  i32.add
                  i32.load8_s
                  i32.const -64
                  i32.lt_s
                  br_if 1 (;@6;)
                end
                local.get 3
                local.set 2
              end
              local.get 5
              local.get 2
              i32.store offset=32
              local.get 1
              local.set 3
              block  ;; label = @6
                local.get 2
                local.get 1
                i32.ge_u
                br_if 0 (;@6;)
                i32.const 0
                local.get 2
                i32.const -3
                i32.add
                local.tee 3
                local.get 3
                local.get 2
                i32.gt_u
                select
                local.tee 3
                local.get 2
                i32.const 1
                i32.add
                local.tee 6
                i32.gt_u
                br_if 3 (;@3;)
                block  ;; label = @7
                  local.get 3
                  local.get 6
                  i32.eq
                  br_if 0 (;@7;)
                  local.get 0
                  local.get 6
                  i32.add
                  local.get 0
                  local.get 3
                  i32.add
                  local.tee 8
                  i32.sub
                  local.set 6
                  block  ;; label = @8
                    local.get 0
                    local.get 2
                    i32.add
                    local.tee 9
                    i32.load8_s
                    i32.const -65
                    i32.le_s
                    br_if 0 (;@8;)
                    local.get 6
                    i32.const -1
                    i32.add
                    local.set 7
                    br 1 (;@7;)
                  end
                  local.get 3
                  local.get 2
                  i32.eq
                  br_if 0 (;@7;)
                  block  ;; label = @8
                    local.get 9
                    i32.const -1
                    i32.add
                    local.tee 2
                    i32.load8_s
                    i32.const -65
                    i32.le_s
                    br_if 0 (;@8;)
                    local.get 6
                    i32.const -2
                    i32.add
                    local.set 7
                    br 1 (;@7;)
                  end
                  local.get 8
                  local.get 2
                  i32.eq
                  br_if 0 (;@7;)
                  block  ;; label = @8
                    local.get 9
                    i32.const -2
                    i32.add
                    local.tee 2
                    i32.load8_s
                    i32.const -65
                    i32.le_s
                    br_if 0 (;@8;)
                    local.get 6
                    i32.const -3
                    i32.add
                    local.set 7
                    br 1 (;@7;)
                  end
                  local.get 8
                  local.get 2
                  i32.eq
                  br_if 0 (;@7;)
                  block  ;; label = @8
                    local.get 9
                    i32.const -3
                    i32.add
                    local.tee 2
                    i32.load8_s
                    i32.const -65
                    i32.le_s
                    br_if 0 (;@8;)
                    local.get 6
                    i32.const -4
                    i32.add
                    local.set 7
                    br 1 (;@7;)
                  end
                  local.get 8
                  local.get 2
                  i32.eq
                  br_if 0 (;@7;)
                  local.get 6
                  i32.const -5
                  i32.add
                  local.set 7
                end
                local.get 7
                local.get 3
                i32.add
                local.set 3
              end
              block  ;; label = @6
                local.get 3
                i32.eqz
                br_if 0 (;@6;)
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 1
                    local.get 3
                    i32.gt_u
                    br_if 0 (;@8;)
                    local.get 1
                    local.get 3
                    i32.eq
                    br_if 1 (;@7;)
                    br 7 (;@1;)
                  end
                  local.get 0
                  local.get 3
                  i32.add
                  i32.load8_s
                  i32.const -65
                  i32.le_s
                  br_if 6 (;@1;)
                end
                local.get 1
                local.get 3
                i32.sub
                local.set 1
              end
              local.get 1
              i32.eqz
              br_if 3 (;@2;)
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      local.get 0
                      local.get 3
                      i32.add
                      local.tee 1
                      i32.load8_s
                      local.tee 2
                      i32.const -1
                      i32.gt_s
                      br_if 0 (;@9;)
                      local.get 1
                      i32.load8_u offset=1
                      i32.const 63
                      i32.and
                      local.set 0
                      local.get 2
                      i32.const 31
                      i32.and
                      local.set 6
                      local.get 2
                      i32.const -33
                      i32.gt_u
                      br_if 1 (;@8;)
                      local.get 6
                      i32.const 6
                      i32.shl
                      local.get 0
                      i32.or
                      local.set 1
                      br 2 (;@7;)
                    end
                    local.get 5
                    local.get 2
                    i32.const 255
                    i32.and
                    i32.store offset=36
                    i32.const 1
                    local.set 2
                    br 2 (;@6;)
                  end
                  local.get 0
                  i32.const 6
                  i32.shl
                  local.get 1
                  i32.load8_u offset=2
                  i32.const 63
                  i32.and
                  i32.or
                  local.set 0
                  block  ;; label = @8
                    local.get 2
                    i32.const -16
                    i32.ge_u
                    br_if 0 (;@8;)
                    local.get 0
                    local.get 6
                    i32.const 12
                    i32.shl
                    i32.or
                    local.set 1
                    br 1 (;@7;)
                  end
                  local.get 0
                  i32.const 6
                  i32.shl
                  local.get 1
                  i32.load8_u offset=3
                  i32.const 63
                  i32.and
                  i32.or
                  local.get 6
                  i32.const 18
                  i32.shl
                  i32.const 1835008
                  i32.and
                  i32.or
                  local.tee 1
                  i32.const 1114112
                  i32.eq
                  br_if 5 (;@2;)
                end
                local.get 5
                local.get 1
                i32.store offset=36
                i32.const 1
                local.set 2
                local.get 1
                i32.const 128
                i32.lt_u
                br_if 0 (;@6;)
                i32.const 2
                local.set 2
                local.get 1
                i32.const 2048
                i32.lt_u
                br_if 0 (;@6;)
                i32.const 3
                i32.const 4
                local.get 1
                i32.const 65536
                i32.lt_u
                select
                local.set 2
              end
              local.get 5
              local.get 3
              i32.store offset=40
              local.get 5
              local.get 2
              local.get 3
              i32.add
              i32.store offset=44
              local.get 5
              i32.const 48
              i32.add
              i32.const 12
              i32.add
              i64.const 5
              i64.store align=4
              local.get 5
              i32.const 108
              i32.add
              i32.const 6
              i32.store
              local.get 5
              i32.const 100
              i32.add
              i32.const 6
              i32.store
              local.get 5
              i32.const 92
              i32.add
              i32.const 7
              i32.store
              local.get 5
              i32.const 72
              i32.add
              i32.const 12
              i32.add
              i32.const 8
              i32.store
              local.get 5
              i32.const 5
              i32.store offset=52
              local.get 5
              i32.const 1050696
              i32.store offset=48
              local.get 5
              i32.const 3
              i32.store offset=76
              local.get 5
              local.get 5
              i32.const 72
              i32.add
              i32.store offset=56
              local.get 5
              local.get 5
              i32.const 24
              i32.add
              i32.store offset=104
              local.get 5
              local.get 5
              i32.const 16
              i32.add
              i32.store offset=96
              local.get 5
              local.get 5
              i32.const 40
              i32.add
              i32.store offset=88
              local.get 5
              local.get 5
              i32.const 36
              i32.add
              i32.store offset=80
              local.get 5
              local.get 5
              i32.const 32
              i32.add
              i32.store offset=72
              local.get 5
              i32.const 48
              i32.add
              local.get 4
              call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
              unreachable
            end
            local.get 5
            local.get 2
            local.get 3
            local.get 6
            select
            i32.store offset=40
            local.get 5
            i32.const 48
            i32.add
            i32.const 12
            i32.add
            i64.const 3
            i64.store align=4
            local.get 5
            i32.const 92
            i32.add
            i32.const 6
            i32.store
            local.get 5
            i32.const 72
            i32.add
            i32.const 12
            i32.add
            i32.const 6
            i32.store
            local.get 5
            i32.const 3
            i32.store offset=52
            local.get 5
            i32.const 1050760
            i32.store offset=48
            local.get 5
            i32.const 3
            i32.store offset=76
            local.get 5
            local.get 5
            i32.const 72
            i32.add
            i32.store offset=56
            local.get 5
            local.get 5
            i32.const 24
            i32.add
            i32.store offset=88
            local.get 5
            local.get 5
            i32.const 16
            i32.add
            i32.store offset=80
            local.get 5
            local.get 5
            i32.const 40
            i32.add
            i32.store offset=72
            local.get 5
            i32.const 48
            i32.add
            local.get 4
            call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
            unreachable
          end
          local.get 5
          i32.const 100
          i32.add
          i32.const 6
          i32.store
          local.get 5
          i32.const 92
          i32.add
          i32.const 6
          i32.store
          local.get 5
          i32.const 72
          i32.add
          i32.const 12
          i32.add
          i32.const 3
          i32.store
          local.get 5
          i32.const 48
          i32.add
          i32.const 12
          i32.add
          i64.const 4
          i64.store align=4
          local.get 5
          i32.const 4
          i32.store offset=52
          local.get 5
          i32.const 1050600
          i32.store offset=48
          local.get 5
          i32.const 3
          i32.store offset=76
          local.get 5
          local.get 5
          i32.const 72
          i32.add
          i32.store offset=56
          local.get 5
          local.get 5
          i32.const 24
          i32.add
          i32.store offset=96
          local.get 5
          local.get 5
          i32.const 16
          i32.add
          i32.store offset=88
          local.get 5
          local.get 5
          i32.const 12
          i32.add
          i32.store offset=80
          local.get 5
          local.get 5
          i32.const 8
          i32.add
          i32.store offset=72
          local.get 5
          i32.const 48
          i32.add
          local.get 4
          call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
          unreachable
        end
        local.get 3
        local.get 6
        i32.const 1050812
        call $_ZN4core5slice5index22slice_index_order_fail17h5c05174755728e22E
        unreachable
      end
      local.get 4
      call $_ZN4core9panicking5panic17hdd77bb12897b1389E
      unreachable
    end
    local.get 0
    local.get 1
    local.get 3
    local.get 1
    local.get 4
    call $_ZN4core3str16slice_error_fail17h5761aa5418ad8f04E
    unreachable)
  (func $_ZN71_$LT$core..ops..range..Range$LT$Idx$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17h938a7622bad5fb44E (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    i32.const 1
    local.set 3
    block  ;; label = @1
      local.get 0
      i32.load
      local.get 1
      call $_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u32$GT$3fmt17h021845e23eea6eaeE
      br_if 0 (;@1;)
      local.get 2
      i32.const 20
      i32.add
      i64.const 0
      i64.store align=4
      i32.const 1
      local.set 3
      local.get 2
      i32.const 1
      i32.store offset=12
      local.get 2
      i32.const 1049880
      i32.store offset=8
      local.get 2
      i32.const 1049804
      i32.store offset=16
      local.get 1
      i32.load offset=20
      local.get 1
      i32.const 24
      i32.add
      i32.load
      local.get 2
      i32.const 8
      i32.add
      call $_ZN4core3fmt5write17h85c2d164d6b9d548E
      br_if 0 (;@1;)
      local.get 0
      i32.load offset=4
      local.get 1
      call $_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u32$GT$3fmt17h021845e23eea6eaeE
      local.set 3
    end
    local.get 2
    i32.const 32
    i32.add
    global.set $__stack_pointer
    local.get 3)
  (func $_ZN41_$LT$char$u20$as$u20$core..fmt..Debug$GT$3fmt17hd37a4f87b17c40b9E (type 2) (param i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    i32.const 1
    local.set 3
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        i32.load offset=20
        local.tee 4
        i32.const 39
        local.get 1
        i32.const 24
        i32.add
        i32.load
        i32.load offset=16
        local.tee 5
        call_indirect (type 2)
        br_if 0 (;@2;)
        local.get 2
        local.get 0
        i32.load
        i32.const 257
        call $_ZN4core4char7methods22_$LT$impl$u20$char$GT$16escape_debug_ext17hccd8a8eff60d1e48E
        block  ;; label = @3
          block  ;; label = @4
            local.get 2
            i32.load8_u
            i32.const 128
            i32.ne
            br_if 0 (;@4;)
            local.get 2
            i32.const 8
            i32.add
            local.set 6
            i32.const 128
            local.set 7
            loop  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  local.get 7
                  i32.const 255
                  i32.and
                  i32.const 128
                  i32.eq
                  br_if 0 (;@7;)
                  local.get 2
                  i32.load8_u offset=10
                  local.tee 0
                  local.get 2
                  i32.load8_u offset=11
                  i32.ge_u
                  br_if 4 (;@3;)
                  local.get 2
                  local.get 0
                  i32.const 1
                  i32.add
                  i32.store8 offset=10
                  local.get 0
                  i32.const 10
                  i32.ge_u
                  br_if 6 (;@1;)
                  local.get 2
                  local.get 0
                  i32.add
                  i32.load8_u
                  local.set 1
                  br 1 (;@6;)
                end
                i32.const 0
                local.set 7
                local.get 6
                i32.const 0
                i32.store
                local.get 2
                i32.load offset=4
                local.set 1
                local.get 2
                i64.const 0
                i64.store
              end
              local.get 4
              local.get 1
              local.get 5
              call_indirect (type 2)
              i32.eqz
              br_if 0 (;@5;)
              br 3 (;@2;)
            end
          end
          local.get 2
          i32.load8_u offset=10
          local.tee 1
          i32.const 10
          local.get 1
          i32.const 10
          i32.gt_u
          select
          local.set 0
          local.get 2
          i32.load8_u offset=11
          local.tee 7
          local.get 1
          local.get 7
          local.get 1
          i32.gt_u
          select
          local.set 8
          loop  ;; label = @4
            local.get 8
            local.get 1
            i32.eq
            br_if 1 (;@3;)
            local.get 2
            local.get 1
            i32.const 1
            i32.add
            local.tee 7
            i32.store8 offset=10
            local.get 0
            local.get 1
            i32.eq
            br_if 3 (;@1;)
            local.get 2
            local.get 1
            i32.add
            local.set 6
            local.get 7
            local.set 1
            local.get 4
            local.get 6
            i32.load8_u
            local.get 5
            call_indirect (type 2)
            i32.eqz
            br_if 0 (;@4;)
            br 2 (;@2;)
          end
        end
        local.get 4
        i32.const 39
        local.get 5
        call_indirect (type 2)
        local.set 3
      end
      local.get 2
      i32.const 16
      i32.add
      global.set $__stack_pointer
      local.get 3
      return
    end
    local.get 0
    i32.const 10
    i32.const 1052408
    call $_ZN4core9panicking18panic_bounds_check17hcafbc5434e2cd720E
    unreachable)
  (func $_ZN4core5slice29_$LT$impl$u20$$u5b$T$u5d$$GT$15copy_from_slice17len_mismatch_fail17h93d5590d3e243978E (type 0) (param i32 i32 i32)
    (local i32)
    global.get $__stack_pointer
    i32.const 48
    i32.sub
    local.tee 3
    global.set $__stack_pointer
    local.get 3
    local.get 1
    i32.store offset=4
    local.get 3
    local.get 0
    i32.store
    local.get 3
    i32.const 8
    i32.add
    i32.const 12
    i32.add
    i64.const 2
    i64.store align=4
    local.get 3
    i32.const 32
    i32.add
    i32.const 12
    i32.add
    i32.const 3
    i32.store
    local.get 3
    i32.const 3
    i32.store offset=12
    local.get 3
    i32.const 1050536
    i32.store offset=8
    local.get 3
    i32.const 3
    i32.store offset=36
    local.get 3
    local.get 3
    i32.const 32
    i32.add
    i32.store offset=16
    local.get 3
    local.get 3
    i32.store offset=40
    local.get 3
    local.get 3
    i32.const 4
    i32.add
    i32.store offset=32
    local.get 3
    i32.const 8
    i32.add
    local.get 2
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $deploy (type 7)
    (local i32 i32 i64 i32 i32 i32 i64 i64 i64 i64)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 28
    i32.add
    call $_ZN14fluentbase_sdk5rwasm3evm80_$LT$impl$u20$fluentbase_sdk..EvmPlatformSDK$u20$for$u20$fluentbase_sdk..SDK$GT$10evm_caller17h25c0ee09dac6dd8aE
    local.get 0
    i32.const 120
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 112
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 96
    i32.add
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store offset=96
    i32.const 1048836
    local.set 1
    i64.const 5
    local.set 2
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          loop  ;; label = @4
            local.get 1
            i32.const 1048861
            i32.eq
            br_if 1 (;@3;)
            block  ;; label = @5
              block  ;; label = @6
                local.get 1
                i32.load8_s
                local.tee 3
                i32.const -1
                i32.le_s
                br_if 0 (;@6;)
                local.get 1
                i32.const 1
                i32.add
                local.set 1
                local.get 3
                i32.const 255
                i32.and
                local.set 4
                br 1 (;@5;)
              end
              local.get 1
              i32.load8_u offset=1
              i32.const 63
              i32.and
              local.set 4
              local.get 3
              i32.const 31
              i32.and
              local.set 5
              block  ;; label = @6
                local.get 3
                i32.const -33
                i32.gt_u
                br_if 0 (;@6;)
                local.get 5
                i32.const 6
                i32.shl
                local.get 4
                i32.or
                local.set 4
                local.get 1
                i32.const 2
                i32.add
                local.set 1
                br 1 (;@5;)
              end
              local.get 4
              i32.const 6
              i32.shl
              local.get 1
              i32.load8_u offset=2
              i32.const 63
              i32.and
              i32.or
              local.set 4
              block  ;; label = @6
                local.get 3
                i32.const -16
                i32.ge_u
                br_if 0 (;@6;)
                local.get 4
                local.get 5
                i32.const 12
                i32.shl
                i32.or
                local.set 4
                local.get 1
                i32.const 3
                i32.add
                local.set 1
                br 1 (;@5;)
              end
              local.get 4
              i32.const 6
              i32.shl
              local.get 1
              i32.load8_u offset=3
              i32.const 63
              i32.and
              i32.or
              local.get 5
              i32.const 18
              i32.shl
              i32.const 1835008
              i32.and
              i32.or
              local.tee 4
              i32.const 1114112
              i32.eq
              br_if 2 (;@3;)
              local.get 1
              i32.const 4
              i32.add
              local.set 1
            end
            local.get 2
            i64.const 5
            i64.ne
            br_if 0 (;@4;)
            block  ;; label = @5
              local.get 4
              i32.const -48
              i32.add
              i32.const 10
              i32.lt_u
              br_if 0 (;@5;)
              block  ;; label = @6
                local.get 4
                i32.const -97
                i32.add
                i32.const 26
                i32.ge_u
                br_if 0 (;@6;)
                i64.const -87
                local.set 2
                br 4 (;@2;)
              end
              block  ;; label = @6
                local.get 4
                i32.const -65
                i32.add
                i32.const 26
                i32.ge_u
                br_if 0 (;@6;)
                i64.const -55
                local.set 2
                br 4 (;@2;)
              end
              i64.const 5
              local.set 2
              local.get 4
              i32.const 95
              i32.eq
              br_if 1 (;@4;)
              local.get 6
              i64.const -4294967296
              i64.and
              local.get 4
              i64.extend_i32_u
              i64.or
              local.set 6
              i64.const 3
              local.set 2
              br 1 (;@4;)
            end
            i64.const 0
            local.set 7
            i32.const 0
            local.set 3
            local.get 4
            i64.extend_i32_u
            i64.const -48
            i64.add
            local.tee 8
            local.set 9
            loop  ;; label = @5
              block  ;; label = @6
                local.get 3
                i32.const 32
                i32.ne
                br_if 0 (;@6;)
                i64.const 5
                local.set 2
                local.get 9
                local.get 7
                i64.or
                i64.eqz
                br_if 2 (;@4;)
                i64.const 0
                local.set 2
                local.get 8
                local.set 6
                br 5 (;@1;)
              end
              local.get 0
              i32.const 8
              i32.add
              local.get 0
              i32.const 96
              i32.add
              local.get 3
              i32.add
              local.tee 4
              i64.load
              i64.const 0
              i64.const 10
              i64.const 0
              call $__multi3
              local.get 4
              local.get 0
              i64.load offset=8
              local.tee 2
              local.get 9
              i64.add
              local.tee 9
              i64.store
              local.get 0
              i32.const 8
              i32.add
              i32.const 8
              i32.add
              i64.load
              local.get 7
              i64.add
              local.get 9
              local.get 2
              i64.lt_u
              i64.extend_i32_u
              i64.add
              local.set 9
              local.get 3
              i32.const 8
              i32.add
              local.set 3
              i64.const 0
              local.set 7
              br 0 (;@5;)
            end
          end
        end
        local.get 2
        i64.const 5
        i64.ne
        br_if 1 (;@1;)
        local.get 0
        i64.load offset=112
        local.set 2
        local.get 0
        i64.load offset=104
        local.set 7
        local.get 0
        i64.load offset=96
        local.set 9
        local.get 0
        local.get 0
        i64.load offset=120
        i64.store offset=72
        local.get 0
        local.get 2
        i64.store offset=64
        local.get 0
        local.get 7
        i64.store offset=56
        local.get 0
        local.get 9
        i64.store offset=48
        local.get 0
        i32.const 84
        i32.add
        local.get 0
        i32.const 28
        i32.add
        call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17ha702285a765a087fE
        local.get 0
        i32.const 96
        i32.add
        local.get 0
        i32.load offset=84
        local.tee 1
        local.get 0
        i32.load offset=92
        call $_ZN18fluentbase_example5erc2019storage_mapping_key17h34d86350df1f7d79E
        local.get 1
        local.get 0
        i32.load offset=88
        call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
        local.get 0
        i32.const 96
        i32.add
        local.get 0
        i32.const 48
        i32.add
        call $_evm_sstore
        local.get 0
        i32.const 128
        i32.add
        global.set $__stack_pointer
        return
      end
      local.get 2
      local.get 4
      i64.extend_i32_u
      i64.add
      local.set 6
      i64.const 2
      local.set 2
    end
    local.get 0
    i64.const 10
    i64.store offset=112
    local.get 0
    local.get 6
    i64.store offset=104
    local.get 0
    local.get 2
    i64.store offset=96
    local.get 0
    i32.const 96
    i32.add
    i32.const 1048688
    i32.const 1048864
    call $_ZN4core6result13unwrap_failed17h38a5f72e87633eadE
    unreachable)
  (func $_ZN14fluentbase_sdk5rwasm3evm80_$LT$impl$u20$fluentbase_sdk..EvmPlatformSDK$u20$for$u20$fluentbase_sdk..SDK$GT$10evm_caller17h25c0ee09dac6dd8aE (type 6) (param i32)
    (local i32 i32 i32)
    global.get $__stack_pointer
    i32.const 32
    i32.sub
    local.tee 1
    global.set $__stack_pointer
    local.get 1
    i32.const 8
    i32.add
    i32.const 16
    i32.add
    local.tee 2
    i32.const 0
    i32.store
    local.get 1
    i32.const 8
    i32.add
    i32.const 8
    i32.add
    local.tee 3
    i64.const 0
    i64.store
    local.get 1
    i64.const 0
    i64.store offset=8
    local.get 1
    i32.const 8
    i32.add
    call $_evm_caller
    local.get 0
    i32.const 16
    i32.add
    local.get 2
    i32.load
    i32.store align=1
    local.get 0
    i32.const 8
    i32.add
    local.get 3
    i64.load
    i64.store align=1
    local.get 0
    local.get 1
    i64.load offset=8
    i64.store align=1
    local.get 1
    i32.const 32
    i32.add
    global.set $__stack_pointer)
  (func $main (type 7)
    (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i64 i64 i64 i32 i32 i32 i32 i32 i32 i32 i32 i64 i64)
    global.get $__stack_pointer
    i32.const 672
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    i32.const 0
    i32.store offset=12
    local.get 0
    i32.const 12
    i32.add
    i32.const 0
    i32.const 4
    call $_sys_read
    drop
    local.get 0
    i32.const 16
    i32.add
    i32.const 0
    i32.const 96
    call $memset
    drop
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      block  ;; label = @10
                        block  ;; label = @11
                          block  ;; label = @12
                            block  ;; label = @13
                              block  ;; label = @14
                                block  ;; label = @15
                                  block  ;; label = @16
                                    local.get 0
                                    i32.load8_u offset=12
                                    local.tee 1
                                    i32.const 6
                                    i32.eq
                                    br_if 0 (;@16;)
                                    local.get 1
                                    i32.const 24
                                    i32.eq
                                    br_if 3 (;@13;)
                                    local.get 1
                                    i32.const 49
                                    i32.eq
                                    br_if 2 (;@14;)
                                    local.get 1
                                    i32.const 112
                                    i32.eq
                                    br_if 4 (;@12;)
                                    local.get 1
                                    i32.const 149
                                    i32.eq
                                    br_if 1 (;@15;)
                                    local.get 1
                                    i32.const 169
                                    i32.eq
                                    br_if 5 (;@11;)
                                    br 15 (;@1;)
                                  end
                                  local.get 0
                                  i32.load8_u offset=13
                                  i32.const 253
                                  i32.ne
                                  br_if 14 (;@1;)
                                  local.get 0
                                  i32.load8_u offset=14
                                  i32.const 255
                                  i32.and
                                  i32.const 222
                                  i32.ne
                                  br_if 14 (;@1;)
                                  local.get 0
                                  i32.load8_u offset=15
                                  i32.const 255
                                  i32.and
                                  i32.const 3
                                  i32.ne
                                  br_if 14 (;@1;)
                                  local.get 0
                                  i64.const 5
                                  i64.store offset=176 align=4
                                  local.get 0
                                  i32.const 1048880
                                  i32.store offset=172
                                  local.get 0
                                  i32.const 1048704
                                  i32.store offset=168
                                  local.get 0
                                  i32.const 112
                                  i32.add
                                  i32.const 1048880
                                  i32.const 5
                                  call $_ZN91_$LT$alloy_primitives..bytes_..Bytes$u20$as$u20$alloy_sol_types..types..value..SolValue$GT$10abi_encode17hfee42a1e9ae0550dE
                                  local.get 0
                                  i32.load offset=112
                                  local.tee 1
                                  local.get 0
                                  i32.load offset=120
                                  call $_sys_write
                                  local.get 1
                                  local.get 0
                                  i32.load offset=116
                                  call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                                  local.get 0
                                  i32.const 168
                                  i32.add
                                  call $_ZN4core3ptr52drop_in_place$LT$alloy_primitives..bytes_..Bytes$GT$17h2dce723588d0292bE
                                  br 8 (;@7;)
                                end
                                local.get 0
                                i32.load8_u offset=13
                                i32.const 216
                                i32.ne
                                br_if 13 (;@1;)
                                local.get 0
                                i32.load8_u offset=14
                                i32.const 255
                                i32.and
                                i32.const 155
                                i32.ne
                                br_if 13 (;@1;)
                                local.get 0
                                i32.load8_u offset=15
                                i32.const 255
                                i32.and
                                i32.const 65
                                i32.ne
                                br_if 13 (;@1;)
                                local.get 0
                                i64.const 3
                                i64.store offset=176 align=4
                                local.get 0
                                i32.const 1048885
                                i32.store offset=172
                                local.get 0
                                i32.const 1048704
                                i32.store offset=168
                                local.get 0
                                i32.const 112
                                i32.add
                                i32.const 1048885
                                i32.const 3
                                call $_ZN91_$LT$alloy_primitives..bytes_..Bytes$u20$as$u20$alloy_sol_types..types..value..SolValue$GT$10abi_encode17hfee42a1e9ae0550dE
                                local.get 0
                                i32.load offset=112
                                local.tee 1
                                local.get 0
                                i32.load offset=120
                                call $_sys_write
                                local.get 1
                                local.get 0
                                i32.load offset=116
                                call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                                local.get 0
                                i32.const 168
                                i32.add
                                call $_ZN4core3ptr52drop_in_place$LT$alloy_primitives..bytes_..Bytes$GT$17h2dce723588d0292bE
                                br 7 (;@7;)
                              end
                              local.get 0
                              i32.load8_u offset=13
                              i32.const 60
                              i32.ne
                              br_if 12 (;@1;)
                              local.get 0
                              i32.load8_u offset=14
                              i32.const 255
                              i32.and
                              i32.const 229
                              i32.ne
                              br_if 12 (;@1;)
                              local.get 0
                              i32.load8_u offset=15
                              i32.const 255
                              i32.and
                              i32.const 103
                              i32.ne
                              br_if 12 (;@1;)
                              local.get 0
                              i32.const 192
                              i32.add
                              i64.const 0
                              i64.store
                              local.get 0
                              i32.const 184
                              i32.add
                              i64.const 0
                              i64.store
                              local.get 0
                              i64.const 0
                              i64.store offset=176
                              local.get 0
                              i64.const 18
                              i64.store offset=168
                              local.get 0
                              i32.const 112
                              i32.add
                              local.get 0
                              i32.const 168
                              i32.add
                              call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17hdea3fda248de7d34E
                              local.get 0
                              i32.load offset=112
                              local.tee 1
                              local.get 0
                              i32.load offset=120
                              call $_sys_write
                              local.get 1
                              local.get 0
                              i32.load offset=116
                              call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                              br 6 (;@7;)
                            end
                            local.get 0
                            i32.load8_u offset=13
                            i32.const 22
                            i32.ne
                            br_if 11 (;@1;)
                            local.get 0
                            i32.load8_u offset=14
                            i32.const 255
                            i32.and
                            i32.const 13
                            i32.ne
                            br_if 11 (;@1;)
                            local.get 0
                            i32.load8_u offset=15
                            i32.const 255
                            i32.and
                            i32.const 221
                            i32.ne
                            br_if 11 (;@1;)
                            local.get 0
                            i32.const 192
                            i32.add
                            i64.const 0
                            i64.store
                            local.get 0
                            i32.const 184
                            i32.add
                            i64.const 0
                            i64.store
                            local.get 0
                            i32.const 176
                            i32.add
                            i64.const 0
                            i64.store
                            local.get 0
                            i64.const 0
                            i64.store offset=168
                            local.get 0
                            i32.const 112
                            i32.add
                            local.get 0
                            i32.const 168
                            i32.add
                            call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17hdea3fda248de7d34E
                            local.get 0
                            i32.load offset=112
                            local.tee 1
                            local.get 0
                            i32.load offset=120
                            call $_sys_write
                            local.get 1
                            local.get 0
                            i32.load offset=116
                            call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                            br 5 (;@7;)
                          end
                          local.get 0
                          i32.load8_u offset=13
                          i32.const 160
                          i32.ne
                          br_if 10 (;@1;)
                          local.get 0
                          i32.load8_u offset=14
                          i32.const 255
                          i32.and
                          i32.const 130
                          i32.ne
                          br_if 10 (;@1;)
                          local.get 0
                          i32.load8_u offset=15
                          i32.const 255
                          i32.and
                          i32.const 49
                          i32.ne
                          br_if 10 (;@1;)
                          local.get 0
                          i32.const 16
                          i32.add
                          i32.const 4
                          i32.const 32
                          call $_sys_read
                          drop
                          local.get 0
                          i32.const 0
                          i32.store8 offset=124
                          local.get 0
                          i64.const 96
                          i64.store offset=116 align=4
                          local.get 0
                          local.get 0
                          i32.const 16
                          i32.add
                          i32.store offset=112
                          local.get 0
                          i32.const 168
                          i32.add
                          local.get 0
                          i32.const 112
                          i32.add
                          call $_ZN93_$LT$alloy_sol_types..abi..token..WordToken$u20$as$u20$alloy_sol_types..abi..token..Token$GT$11decode_from17ha8441e60a93aef41E
                          local.get 0
                          i32.load8_u offset=168
                          br_if 1 (;@10;)
                          local.get 0
                          local.get 0
                          i32.const 180
                          i32.add
                          i32.load align=1
                          i32.store offset=643 align=1
                          local.get 0
                          local.get 0
                          i32.const 177
                          i32.add
                          i32.load align=1
                          i32.store offset=640
                          local.get 0
                          local.get 0
                          i32.const 185
                          i32.add
                          i32.load align=1
                          i32.store offset=608
                          local.get 0
                          local.get 0
                          i32.const 188
                          i32.add
                          i32.load align=1
                          i32.store offset=611 align=1
                          local.get 0
                          local.get 0
                          i32.const 193
                          i32.add
                          i32.load align=1
                          i32.store offset=576
                          local.get 0
                          local.get 0
                          i32.const 196
                          i32.add
                          i32.load align=1
                          i32.store offset=579 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=640
                          i32.store offset=448
                          local.get 0
                          local.get 0
                          i32.load offset=643 align=1
                          i32.store offset=451 align=1
                          local.get 0
                          i32.const 184
                          i32.add
                          i32.load8_u
                          local.set 1
                          local.get 0
                          i32.const 192
                          i32.add
                          i32.load8_u
                          local.set 2
                          local.get 0
                          i32.const 168
                          i32.add
                          i32.const 32
                          i32.add
                          i32.load8_u
                          local.set 3
                          local.get 0
                          local.get 0
                          i32.load offset=611 align=1
                          i32.store offset=371 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=608
                          i32.store offset=368
                          local.get 0
                          local.get 0
                          i32.load offset=579 align=1
                          i32.store offset=307 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=576
                          i32.store offset=304
                          local.get 0
                          local.get 0
                          i32.load offset=451 align=1
                          i32.store offset=547 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=448
                          i32.store offset=544
                          local.get 0
                          local.get 0
                          i32.load offset=371 align=1
                          i32.store offset=515 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=368
                          i32.store offset=512
                          local.get 0
                          local.get 0
                          i32.load offset=307 align=1
                          i32.store offset=483 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=304
                          i32.store offset=480
                          local.get 0
                          local.get 0
                          i32.load offset=547 align=1
                          i32.store offset=419 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=544
                          i32.store offset=416
                          local.get 0
                          local.get 0
                          i32.load offset=515 align=1
                          i32.store offset=339 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=512
                          i32.store offset=336
                          local.get 0
                          local.get 0
                          i32.load offset=483 align=1
                          i32.store offset=275 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=480
                          i32.store offset=272
                          local.get 0
                          local.get 0
                          i32.load offset=419 align=1
                          i32.store offset=403 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=416
                          i32.store offset=400
                          local.get 0
                          local.get 0
                          i32.load offset=339 align=1
                          i32.store offset=247 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=336
                          i32.store offset=244
                          local.get 0
                          local.get 0
                          i32.load offset=275 align=1
                          i32.store offset=239 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=272
                          i32.store offset=236
                          local.get 0
                          local.get 0
                          i32.load offset=403 align=1
                          i32.store offset=171 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=400
                          i32.store offset=168
                          local.get 0
                          local.get 0
                          i32.load offset=247 align=1
                          i32.store offset=115 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=244
                          i32.store offset=112
                          local.get 0
                          local.get 0
                          i32.load offset=239 align=1
                          i32.store offset=643 align=1
                          local.get 0
                          local.get 0
                          i32.load offset=236
                          i32.store offset=640
                          local.get 0
                          i32.const 450
                          i32.add
                          local.get 0
                          i32.const 174
                          i32.add
                          i32.load8_u
                          i32.store8
                          local.get 0
                          local.get 0
                          i32.load16_u offset=172
                          i32.store16 offset=448
                          br 2 (;@9;)
                        end
                        local.get 0
                        i32.load8_u offset=13
                        i32.const 5
                        i32.ne
                        br_if 9 (;@1;)
                        local.get 0
                        i32.load8_u offset=14
                        i32.const 255
                        i32.and
                        i32.const 156
                        i32.ne
                        br_if 9 (;@1;)
                        local.get 0
                        i32.load8_u offset=15
                        i32.const 255
                        i32.and
                        i32.const 187
                        i32.ne
                        br_if 9 (;@1;)
                        local.get 0
                        i32.const 16
                        i32.add
                        i32.const 4
                        i32.const 64
                        call $_sys_read
                        drop
                        local.get 0
                        i32.const 0
                        i32.store8 offset=412
                        local.get 0
                        i64.const 96
                        i64.store offset=404 align=4
                        local.get 0
                        local.get 0
                        i32.const 16
                        i32.add
                        i32.store offset=400
                        local.get 0
                        i32.const 112
                        i32.add
                        local.get 0
                        i32.const 400
                        i32.add
                        call $_ZN93_$LT$alloy_sol_types..abi..token..WordToken$u20$as$u20$alloy_sol_types..abi..token..Token$GT$11decode_from17ha8441e60a93aef41E
                        local.get 0
                        i32.load8_u offset=112
                        br_if 5 (;@5;)
                        local.get 0
                        i32.const 168
                        i32.add
                        local.get 0
                        i32.const 400
                        i32.add
                        call $_ZN93_$LT$alloy_sol_types..abi..token..WordToken$u20$as$u20$alloy_sol_types..abi..token..Token$GT$11decode_from17ha8441e60a93aef41E
                        local.get 0
                        i32.load8_u offset=168
                        i32.eqz
                        br_if 2 (;@8;)
                        local.get 0
                        i32.const 635
                        i32.add
                        local.get 0
                        i32.const 196
                        i32.add
                        i32.load
                        i32.store align=1
                        local.get 0
                        i32.const 627
                        i32.add
                        local.get 0
                        i32.const 188
                        i32.add
                        i64.load align=4
                        i64.store align=1
                        local.get 0
                        i32.const 619
                        i32.add
                        local.get 0
                        i32.const 180
                        i32.add
                        i64.load align=4
                        i64.store align=1
                        local.get 0
                        local.get 0
                        i64.load offset=172 align=4
                        i64.store offset=611 align=1
                        br 6 (;@4;)
                      end
                      local.get 0
                      i32.const 268
                      i32.add
                      i32.const 2
                      i32.add
                      local.tee 4
                      local.get 0
                      i32.const 175
                      i32.add
                      i32.load8_u
                      i32.store8
                      local.get 0
                      local.get 0
                      i32.const 173
                      i32.add
                      i32.load16_u align=1
                      i32.store16 offset=268
                      local.get 0
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 9
                      i32.add
                      i32.load align=1
                      i32.store offset=640
                      local.get 0
                      local.get 0
                      i32.const 180
                      i32.add
                      i32.load align=1
                      i32.store offset=643 align=1
                      local.get 0
                      local.get 0
                      i32.const 185
                      i32.add
                      i32.load align=1
                      i32.store offset=608
                      local.get 0
                      local.get 0
                      i32.const 188
                      i32.add
                      i32.load align=1
                      i32.store offset=611 align=1
                      local.get 0
                      local.get 0
                      i32.const 196
                      i32.add
                      i32.load align=1
                      i32.store offset=579 align=1
                      local.get 0
                      local.get 0
                      i32.const 193
                      i32.add
                      i32.load align=1
                      i32.store offset=576
                      local.get 0
                      i32.const 176
                      i32.add
                      local.tee 5
                      i32.load8_u
                      local.set 1
                      local.get 0
                      i32.const 184
                      i32.add
                      local.tee 6
                      i32.load8_u
                      local.set 2
                      local.get 0
                      i32.const 192
                      i32.add
                      local.tee 7
                      i32.load8_u
                      local.set 3
                      local.get 0
                      i32.load8_u offset=172
                      local.set 8
                      local.get 0
                      i32.const 264
                      i32.add
                      i32.const 2
                      i32.add
                      local.tee 9
                      local.get 4
                      i32.load8_u
                      i32.store8
                      local.get 0
                      local.get 0
                      i32.load16_u offset=268
                      i32.store16 offset=264
                      local.get 0
                      local.get 0
                      i32.load offset=640
                      i32.store offset=448
                      local.get 0
                      local.get 0
                      i32.load offset=643 align=1
                      i32.store offset=451 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=608
                      i32.store offset=368
                      local.get 0
                      local.get 0
                      i32.load offset=611 align=1
                      i32.store offset=371 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=576
                      i32.store offset=304
                      local.get 0
                      local.get 0
                      i32.load offset=579 align=1
                      i32.store offset=307 align=1
                      local.get 0
                      i32.const 260
                      i32.add
                      i32.const 2
                      i32.add
                      local.tee 4
                      local.get 9
                      i32.load8_u
                      i32.store8
                      local.get 0
                      local.get 0
                      i32.load16_u offset=264
                      i32.store16 offset=260
                      local.get 0
                      local.get 0
                      i32.load offset=451 align=1
                      i32.store offset=547 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=448
                      i32.store offset=544
                      local.get 0
                      local.get 0
                      i32.load offset=371 align=1
                      i32.store offset=515 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=368
                      i32.store offset=512
                      local.get 0
                      local.get 0
                      i32.load offset=307 align=1
                      i32.store offset=483 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=304
                      i32.store offset=480
                      local.get 0
                      i32.const 256
                      i32.add
                      i32.const 2
                      i32.add
                      local.tee 9
                      local.get 4
                      i32.load8_u
                      i32.store8
                      local.get 0
                      local.get 0
                      i32.load16_u offset=260
                      i32.store16 offset=256
                      local.get 0
                      local.get 0
                      i32.load offset=547 align=1
                      i32.store offset=419 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=544
                      i32.store offset=416
                      local.get 0
                      local.get 0
                      i32.load offset=515 align=1
                      i32.store offset=339 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=512
                      i32.store offset=336
                      local.get 0
                      local.get 0
                      i32.load offset=483 align=1
                      i32.store offset=275 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=480
                      i32.store offset=272
                      local.get 0
                      i32.const 252
                      i32.add
                      i32.const 2
                      i32.add
                      local.tee 4
                      local.get 9
                      i32.load8_u
                      i32.store8
                      local.get 0
                      local.get 0
                      i32.load16_u offset=256
                      i32.store16 offset=252
                      local.get 0
                      local.get 0
                      i32.load offset=419 align=1
                      i32.store offset=403 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=416
                      i32.store offset=400
                      local.get 0
                      local.get 0
                      i32.load offset=339 align=1
                      i32.store offset=247 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=336
                      i32.store offset=244
                      local.get 0
                      local.get 0
                      i32.load offset=275 align=1
                      i32.store offset=239 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=272
                      i32.store offset=236
                      local.get 0
                      i32.const 448
                      i32.add
                      i32.const 2
                      i32.add
                      local.tee 9
                      local.get 4
                      i32.load8_u
                      i32.store8
                      local.get 0
                      local.get 0
                      i32.load16_u offset=252
                      i32.store16 offset=448
                      local.get 0
                      local.get 0
                      i32.load offset=403 align=1
                      i32.store offset=115 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=400
                      i32.store offset=112
                      local.get 0
                      local.get 0
                      i32.load offset=247 align=1
                      i32.store offset=643 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=244
                      i32.store offset=640
                      local.get 0
                      local.get 0
                      i32.load offset=239 align=1
                      i32.store offset=579 align=1
                      local.get 0
                      local.get 0
                      i32.load offset=236
                      i32.store offset=576
                      local.get 8
                      i32.const 9
                      i32.ne
                      br_if 3 (;@6;)
                    end
                    local.get 0
                    i32.const 615
                    i32.add
                    local.get 0
                    i32.load offset=115 align=1
                    i32.store align=1
                    local.get 0
                    i32.const 623
                    i32.add
                    local.get 0
                    i32.load offset=643 align=1
                    i32.store align=1
                    local.get 0
                    i32.const 608
                    i32.add
                    i32.const 2
                    i32.add
                    local.get 0
                    i32.const 448
                    i32.add
                    i32.const 2
                    i32.add
                    i32.load8_u
                    i32.store8
                    local.get 0
                    local.get 0
                    i32.load16_u offset=448
                    i32.store16 offset=608
                    local.get 0
                    local.get 1
                    i32.store8 offset=611
                    local.get 0
                    local.get 0
                    i32.load offset=112
                    i32.store offset=612 align=2
                    local.get 0
                    local.get 2
                    i32.store8 offset=619
                    local.get 0
                    local.get 0
                    i32.load offset=640
                    i32.store offset=620 align=2
                    local.get 0
                    local.get 3
                    i32.store8 offset=627
                    local.get 0
                    i32.const 112
                    i32.add
                    i32.const 24
                    i32.add
                    local.tee 1
                    i64.const 0
                    i64.store
                    local.get 0
                    i32.const 112
                    i32.add
                    i32.const 16
                    i32.add
                    local.tee 2
                    i64.const 0
                    i64.store
                    local.get 0
                    i32.const 112
                    i32.add
                    i32.const 8
                    i32.add
                    local.tee 3
                    i64.const 0
                    i64.store
                    local.get 0
                    i64.const 0
                    i64.store offset=112
                    local.get 0
                    i32.const 640
                    i32.add
                    local.get 0
                    i32.const 608
                    i32.add
                    call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17ha702285a765a087fE
                    local.get 0
                    i32.const 168
                    i32.add
                    local.get 0
                    i32.load offset=640
                    local.tee 8
                    local.get 0
                    i32.load offset=648
                    call $_ZN18fluentbase_example5erc2019storage_mapping_key17h34d86350df1f7d79E
                    local.get 8
                    local.get 0
                    i32.load offset=644
                    call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                    local.get 0
                    i32.const 168
                    i32.add
                    local.get 0
                    i32.const 112
                    i32.add
                    call $_evm_sload
                    local.get 0
                    i32.const 640
                    i32.add
                    i32.const 24
                    i32.add
                    local.get 1
                    i64.load
                    i64.store
                    local.get 0
                    i32.const 640
                    i32.add
                    i32.const 16
                    i32.add
                    local.get 2
                    i64.load
                    i64.store
                    local.get 0
                    i32.const 640
                    i32.add
                    i32.const 8
                    i32.add
                    local.get 3
                    i64.load
                    i64.store
                    local.get 0
                    local.get 0
                    i64.load offset=112
                    i64.store offset=640
                    local.get 0
                    i32.const 168
                    i32.add
                    local.get 0
                    i32.const 640
                    i32.add
                    call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17hdea3fda248de7d34E
                    local.get 0
                    i32.load offset=168
                    local.tee 1
                    local.get 0
                    i32.load offset=176
                    call $_sys_write
                    local.get 1
                    local.get 0
                    i32.load offset=172
                    call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                    br 1 (;@7;)
                  end
                  local.get 0
                  i32.const 544
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 2
                  local.get 0
                  i32.const 178
                  i32.add
                  i64.load align=2
                  i64.store
                  local.get 0
                  i32.const 544
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 3
                  local.get 0
                  i32.const 186
                  i32.add
                  i64.load align=2
                  i64.store
                  local.get 0
                  i32.const 544
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 5
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 25
                  i32.add
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 8
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 9
                  i32.add
                  i64.load align=1
                  local.tee 10
                  i64.store
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 16
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 17
                  i32.add
                  i64.load align=1
                  local.tee 11
                  i64.store
                  i32.const 24
                  local.set 1
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 7
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 25
                  i32.add
                  i64.load align=1
                  local.tee 12
                  i64.store
                  local.get 0
                  i32.const 608
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 8
                  local.get 12
                  i64.store
                  local.get 0
                  i32.const 608
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 4
                  local.get 11
                  i64.store
                  local.get 0
                  i32.const 608
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 9
                  local.get 10
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=170 align=2
                  i64.store offset=544
                  local.get 0
                  local.get 0
                  i64.load offset=113 align=1
                  local.tee 10
                  i64.store offset=640
                  local.get 0
                  local.get 10
                  i64.store offset=608
                  local.get 0
                  i32.load8_u offset=169
                  local.set 13
                  local.get 0
                  i32.const 512
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 6
                  local.get 5
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  i32.const 512
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 14
                  local.get 3
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 512
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 3
                  local.get 2
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 576
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 5
                  local.get 9
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 576
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 2
                  local.get 4
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 576
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 15
                  local.get 8
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=544
                  i64.store offset=512
                  local.get 0
                  local.get 0
                  i64.load offset=608
                  i64.store offset=576
                  local.get 0
                  i32.const 480
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 16
                  local.get 6
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  i32.const 480
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 17
                  local.get 14
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 480
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 14
                  local.get 3
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=512
                  i64.store offset=480
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 18
                  local.get 15
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 3
                  local.get 2
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 6
                  local.get 5
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=576
                  i64.store offset=448
                  local.get 0
                  i32.const 416
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 15
                  local.get 16
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  i32.const 416
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 16
                  local.get 17
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 416
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 17
                  local.get 14
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=480
                  i64.store offset=416
                  local.get 0
                  i32.const 368
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 14
                  local.get 18
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 368
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 18
                  local.get 3
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 368
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 19
                  local.get 6
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=448
                  i64.store offset=368
                  local.get 0
                  i32.const 336
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 20
                  local.get 15
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  i32.const 336
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 15
                  local.get 16
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 336
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 16
                  local.get 17
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=416
                  i64.store offset=336
                  local.get 0
                  i32.const 304
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 17
                  local.get 14
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 304
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 14
                  local.get 18
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 304
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 18
                  local.get 19
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=368
                  i64.store offset=304
                  local.get 0
                  i32.const 272
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 19
                  local.get 20
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  i32.const 272
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 20
                  local.get 15
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 272
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 15
                  local.get 16
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=336
                  i64.store offset=272
                  local.get 9
                  local.get 18
                  i64.load
                  i64.store
                  local.get 4
                  local.get 14
                  i64.load
                  i64.store
                  local.get 8
                  local.get 17
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=304
                  i64.store offset=608
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 8
                  local.get 19
                  i64.load align=1
                  i64.store align=1
                  local.get 3
                  local.get 20
                  i64.load
                  i64.store
                  local.get 6
                  local.get 15
                  i64.load
                  local.tee 10
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=272
                  local.tee 11
                  i64.store offset=448
                  local.get 0
                  i32.const 576
                  i32.add
                  i32.const 23
                  i32.add
                  local.tee 4
                  local.get 8
                  i64.load align=1
                  i64.store align=1
                  local.get 2
                  local.get 3
                  i64.load
                  i64.store
                  local.get 5
                  local.get 10
                  i64.store
                  local.get 0
                  local.get 11
                  i64.store offset=576
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 9
                  i32.add
                  local.get 10
                  i64.store align=1
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 17
                  i32.add
                  local.get 2
                  i64.load
                  i64.store align=1
                  local.get 7
                  local.get 4
                  i64.load align=1
                  i64.store align=1
                  local.get 0
                  local.get 13
                  i32.store8 offset=640
                  local.get 0
                  local.get 11
                  i64.store offset=641 align=1
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 24
                  i32.add
                  i64.const 0
                  i64.store
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 16
                  i32.add
                  i64.const 0
                  i64.store
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 8
                  i32.add
                  i64.const 0
                  i64.store
                  local.get 0
                  i64.const 0
                  i64.store offset=112
                  local.get 0
                  i32.const 112
                  i32.add
                  local.set 2
                  block  ;; label = @8
                    loop  ;; label = @9
                      local.get 1
                      i32.const -8
                      i32.eq
                      br_if 1 (;@8;)
                      local.get 2
                      local.get 0
                      i32.const 640
                      i32.add
                      local.get 1
                      i32.add
                      i64.load align=1
                      local.tee 10
                      i64.const 56
                      i64.shl
                      local.get 10
                      i64.const 65280
                      i64.and
                      i64.const 40
                      i64.shl
                      i64.or
                      local.get 10
                      i64.const 16711680
                      i64.and
                      i64.const 24
                      i64.shl
                      local.get 10
                      i64.const 4278190080
                      i64.and
                      i64.const 8
                      i64.shl
                      i64.or
                      i64.or
                      local.get 10
                      i64.const 8
                      i64.shr_u
                      i64.const 4278190080
                      i64.and
                      local.get 10
                      i64.const 24
                      i64.shr_u
                      i64.const 16711680
                      i64.and
                      i64.or
                      local.get 10
                      i64.const 40
                      i64.shr_u
                      i64.const 65280
                      i64.and
                      local.get 10
                      i64.const 56
                      i64.shr_u
                      i64.or
                      i64.or
                      i64.or
                      i64.store
                      local.get 1
                      i32.const -8
                      i32.add
                      local.set 1
                      local.get 2
                      i32.const 8
                      i32.add
                      local.set 2
                      br 0 (;@9;)
                    end
                  end
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 32
                  i32.add
                  local.get 0
                  i64.load offset=112
                  i64.store
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 56
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 24
                  i32.add
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 48
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 1
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 40
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 8
                  i32.add
                  local.tee 2
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 16
                  i32.add
                  local.get 0
                  i32.const 628
                  i32.add
                  i64.load align=4
                  i64.store
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 24
                  i32.add
                  local.get 0
                  i32.const 636
                  i32.add
                  i32.load
                  i32.store
                  local.get 0
                  local.get 0
                  i64.load offset=620 align=4
                  i64.store offset=176
                  local.get 0
                  i32.const 112
                  i32.add
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 8
                  i32.add
                  i32.const 56
                  call $memcpy
                  drop
                  local.get 0
                  i32.const 304
                  i32.add
                  i32.const 16
                  i32.add
                  local.get 1
                  i32.load
                  i32.store
                  local.get 0
                  i32.const 304
                  i32.add
                  i32.const 8
                  i32.add
                  local.get 2
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=112
                  i64.store offset=304
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 24
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 48
                  i32.add
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 16
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 40
                  i32.add
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 448
                  i32.add
                  i32.const 8
                  i32.add
                  local.get 0
                  i32.const 112
                  i32.add
                  i32.const 32
                  i32.add
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=136
                  i64.store offset=448
                  local.get 0
                  i32.const 368
                  i32.add
                  call $_ZN14fluentbase_sdk5rwasm3evm80_$LT$impl$u20$fluentbase_sdk..EvmPlatformSDK$u20$for$u20$fluentbase_sdk..SDK$GT$10evm_caller17h25c0ee09dac6dd8aE
                  local.get 0
                  i32.const 368
                  i32.add
                  call $_ZN16alloy_primitives4bits5fixed19FixedBytes$LT$_$GT$7is_zero17h91cdd6d0bac06c3eE
                  br_if 4 (;@3;)
                  local.get 0
                  i32.const 304
                  i32.add
                  call $_ZN16alloy_primitives4bits5fixed19FixedBytes$LT$_$GT$7is_zero17h91cdd6d0bac06c3eE
                  br_if 5 (;@2;)
                  i32.const 24
                  local.set 1
                  local.get 0
                  i32.const 576
                  i32.add
                  i32.const 24
                  i32.add
                  i64.const 0
                  i64.store
                  local.get 0
                  i32.const 592
                  i32.add
                  i64.const 0
                  i64.store
                  local.get 0
                  i32.const 584
                  i32.add
                  i64.const 0
                  i64.store
                  local.get 0
                  i64.const 0
                  i64.store offset=576
                  local.get 0
                  i32.const 168
                  i32.add
                  local.get 0
                  i32.const 368
                  i32.add
                  call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17ha702285a765a087fE
                  local.get 0
                  i32.const 608
                  i32.add
                  local.get 0
                  i32.load offset=168
                  local.tee 2
                  local.get 0
                  i32.load offset=176
                  call $_ZN18fluentbase_example5erc2019storage_mapping_key17h34d86350df1f7d79E
                  local.get 2
                  local.get 0
                  i32.load offset=172
                  call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                  local.get 0
                  i32.const 608
                  i32.add
                  local.get 0
                  i32.const 576
                  i32.add
                  call $_evm_sload
                  block  ;; label = @8
                    block  ;; label = @9
                      block  ;; label = @10
                        loop  ;; label = @11
                          local.get 1
                          i32.const -8
                          i32.add
                          local.tee 2
                          i32.const -16
                          i32.eq
                          br_if 1 (;@10;)
                          local.get 0
                          i32.const 448
                          i32.add
                          local.get 1
                          i32.add
                          local.set 3
                          local.get 0
                          i32.const 576
                          i32.add
                          local.get 1
                          i32.add
                          local.set 8
                          local.get 2
                          local.set 1
                          local.get 8
                          i64.load
                          local.tee 10
                          local.get 3
                          i64.load
                          local.tee 11
                          i64.gt_u
                          local.get 10
                          local.get 11
                          i64.lt_u
                          i32.sub
                          local.tee 2
                          i32.eqz
                          br_if 0 (;@11;)
                        end
                        local.get 2
                        i32.const 255
                        i32.and
                        i32.const 255
                        i32.eq
                        br_if 1 (;@9;)
                      end
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 24
                      i32.add
                      local.get 0
                      i32.const 576
                      i32.add
                      i32.const 24
                      i32.add
                      i64.load
                      i64.store
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 16
                      i32.add
                      local.get 0
                      i32.const 576
                      i32.add
                      i32.const 16
                      i32.add
                      i64.load
                      i64.store
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 8
                      i32.add
                      local.get 0
                      i32.const 576
                      i32.add
                      i32.const 8
                      i32.add
                      i64.load
                      i64.store
                      local.get 0
                      local.get 0
                      i64.load offset=576
                      i64.store offset=168
                      i64.const 0
                      local.set 10
                      i32.const 0
                      local.set 1
                      i64.const 0
                      local.set 11
                      block  ;; label = @10
                        loop  ;; label = @11
                          local.get 1
                          i32.const 32
                          i32.eq
                          br_if 1 (;@10;)
                          local.get 0
                          i32.const 168
                          i32.add
                          local.get 1
                          i32.add
                          local.tee 2
                          local.get 2
                          i64.load
                          local.tee 12
                          local.get 0
                          i32.const 448
                          i32.add
                          local.get 1
                          i32.add
                          i64.load
                          local.tee 21
                          i64.sub
                          local.tee 22
                          local.get 10
                          i64.add
                          local.tee 10
                          i64.store
                          local.get 11
                          local.get 12
                          local.get 21
                          i64.lt_u
                          i64.extend_i32_u
                          i64.sub
                          local.get 10
                          local.get 22
                          i64.lt_u
                          i64.extend_i32_u
                          i64.add
                          local.tee 10
                          i64.const 63
                          i64.shr_s
                          local.set 11
                          local.get 1
                          i32.const 8
                          i32.add
                          local.set 1
                          br 0 (;@11;)
                        end
                      end
                      local.get 0
                      i32.const 640
                      i32.add
                      i32.const 24
                      i32.add
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 24
                      i32.add
                      local.tee 1
                      i64.load
                      i64.store
                      local.get 0
                      i32.const 640
                      i32.add
                      i32.const 16
                      i32.add
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 16
                      i32.add
                      local.tee 2
                      i64.load
                      i64.store
                      local.get 0
                      i32.const 640
                      i32.add
                      i32.const 8
                      i32.add
                      local.get 0
                      i32.const 168
                      i32.add
                      i32.const 8
                      i32.add
                      local.tee 3
                      i64.load
                      i64.store
                      local.get 0
                      local.get 0
                      i64.load offset=168
                      i64.store offset=640
                      local.get 0
                      i32.const 608
                      i32.add
                      local.get 0
                      i32.const 640
                      i32.add
                      call $_evm_sstore
                      local.get 0
                      i32.const 576
                      i32.add
                      i32.const 24
                      i32.add
                      local.tee 8
                      i64.const 0
                      i64.store
                      local.get 0
                      i32.const 576
                      i32.add
                      i32.const 16
                      i32.add
                      local.tee 4
                      i64.const 0
                      i64.store
                      local.get 0
                      i32.const 576
                      i32.add
                      i32.const 8
                      i32.add
                      local.tee 9
                      i64.const 0
                      i64.store
                      local.get 0
                      i64.const 0
                      i64.store offset=576
                      local.get 0
                      i32.const 168
                      i32.add
                      local.get 0
                      i32.const 112
                      i32.add
                      call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17ha702285a765a087fE
                      local.get 0
                      i32.const 608
                      i32.add
                      local.get 0
                      i32.load offset=168
                      local.tee 5
                      local.get 0
                      i32.load offset=176
                      call $_ZN18fluentbase_example5erc2019storage_mapping_key17h34d86350df1f7d79E
                      local.get 5
                      local.get 0
                      i32.load offset=172
                      call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                      local.get 0
                      i32.const 608
                      i32.add
                      local.get 0
                      i32.const 576
                      i32.add
                      call $_evm_sload
                      local.get 1
                      local.get 8
                      i64.load
                      i64.store
                      local.get 2
                      local.get 4
                      i64.load
                      i64.store
                      local.get 3
                      local.get 9
                      i64.load
                      i64.store
                      local.get 0
                      local.get 0
                      i64.load offset=576
                      i64.store offset=168
                      i32.const 0
                      local.set 1
                      i64.const 0
                      local.set 10
                      loop  ;; label = @10
                        local.get 1
                        i32.const 32
                        i32.eq
                        br_if 2 (;@8;)
                        local.get 0
                        i32.const 168
                        i32.add
                        local.get 1
                        i32.add
                        local.tee 2
                        local.get 10
                        local.get 2
                        i64.load
                        i64.add
                        local.tee 11
                        local.get 0
                        i32.const 448
                        i32.add
                        local.get 1
                        i32.add
                        i64.load
                        i64.add
                        local.tee 12
                        i64.store
                        i64.const 0
                        local.get 11
                        local.get 10
                        i64.lt_u
                        i64.extend_i32_u
                        i64.add
                        local.get 12
                        local.get 11
                        i64.lt_u
                        i64.extend_i32_u
                        i64.add
                        local.set 10
                        local.get 1
                        i32.const 8
                        i32.add
                        local.set 1
                        br 0 (;@10;)
                      end
                    end
                    local.get 0
                    i32.const 180
                    i32.add
                    i64.const 0
                    i64.store align=4
                    local.get 0
                    i32.const 1
                    i32.store offset=172
                    local.get 0
                    i32.const 1048908
                    i32.store offset=168
                    local.get 0
                    i32.const 1049804
                    i32.store offset=176
                    local.get 0
                    i32.const 168
                    i32.add
                    i32.const 1048916
                    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
                    unreachable
                  end
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 24
                  i32.add
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 24
                  i32.add
                  local.tee 1
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 16
                  i32.add
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 16
                  i32.add
                  local.tee 2
                  i64.load
                  i64.store
                  local.get 0
                  i32.const 640
                  i32.add
                  i32.const 8
                  i32.add
                  local.get 0
                  i32.const 168
                  i32.add
                  i32.const 8
                  i32.add
                  i64.load
                  i64.store
                  local.get 0
                  local.get 0
                  i64.load offset=168
                  i64.store offset=640
                  local.get 0
                  i32.const 608
                  i32.add
                  local.get 0
                  i32.const 640
                  i32.add
                  call $_evm_sstore
                  local.get 2
                  i64.const 0
                  i64.store
                  local.get 1
                  i64.const 0
                  i64.store
                  local.get 0
                  i64.const 0
                  i64.store offset=176
                  local.get 0
                  i64.const 1
                  i64.store offset=168
                  local.get 0
                  i32.const 640
                  i32.add
                  local.get 0
                  i32.const 168
                  i32.add
                  call $_ZN15alloy_sol_types5types5value8SolValue10abi_encode17hdea3fda248de7d34E
                  local.get 0
                  i32.load offset=640
                  local.tee 1
                  local.get 0
                  i32.load offset=648
                  call $_sys_write
                  local.get 1
                  local.get 0
                  i32.load offset=644
                  call $_ZN77_$LT$alloc..raw_vec..RawVec$LT$T$C$A$GT$$u20$as$u20$core..ops..drop..Drop$GT$4drop17h326eb7bc1013df4cE
                end
                local.get 0
                i32.const 672
                i32.add
                global.set $__stack_pointer
                return
              end
              local.get 5
              local.get 0
              i32.load offset=115 align=1
              i32.store align=1
              local.get 6
              local.get 0
              i32.load offset=643 align=1
              i32.store align=1
              local.get 7
              local.get 0
              i32.load offset=579 align=1
              i32.store align=1
              local.get 0
              local.get 8
              i32.store8 offset=168
              local.get 0
              local.get 0
              i32.load16_u offset=448
              i32.store16 offset=169 align=1
              local.get 0
              local.get 1
              i32.store8 offset=172
              local.get 0
              local.get 0
              i32.load offset=112
              i32.store offset=173 align=1
              local.get 0
              local.get 2
              i32.store8 offset=180
              local.get 0
              local.get 0
              i32.load offset=640
              i32.store offset=181 align=1
              local.get 0
              local.get 3
              i32.store8 offset=188
              local.get 0
              local.get 0
              i32.load offset=576
              i32.store offset=189 align=1
              local.get 0
              local.get 9
              i32.load8_u
              i32.store8 offset=171
              local.get 0
              i32.const 168
              i32.add
              i32.const 1048672
              i32.const 1049012
              call $_ZN4core6result13unwrap_failed17h38a5f72e87633eadE
              unreachable
            end
            local.get 0
            i32.const 635
            i32.add
            local.get 0
            i32.const 140
            i32.add
            i32.load
            i32.store align=1
            local.get 0
            i32.const 627
            i32.add
            local.get 0
            i32.const 132
            i32.add
            i64.load align=4
            i64.store align=1
            local.get 0
            i32.const 619
            i32.add
            local.get 0
            i32.const 124
            i32.add
            i64.load align=4
            i64.store align=1
            local.get 0
            local.get 0
            i64.load offset=116 align=4
            i64.store offset=611 align=1
          end
          local.get 0
          i32.const 448
          i32.add
          i32.const 11
          i32.add
          local.get 0
          i32.const 608
          i32.add
          i32.const 11
          i32.add
          i64.load align=1
          local.tee 10
          i64.store align=1
          local.get 0
          i32.const 368
          i32.add
          i32.const 27
          i32.add
          local.tee 1
          local.get 0
          i32.const 608
          i32.add
          i32.const 27
          i32.add
          i32.load align=1
          i32.store align=1
          local.get 0
          i32.const 368
          i32.add
          i32.const 19
          i32.add
          local.tee 2
          local.get 0
          i32.const 608
          i32.add
          i32.const 19
          i32.add
          i64.load align=1
          i64.store align=1
          local.get 0
          i32.const 368
          i32.add
          i32.const 11
          i32.add
          local.tee 3
          local.get 10
          i64.store align=1
          local.get 0
          local.get 0
          i64.load offset=611 align=1
          local.tee 10
          i64.store offset=451 align=1
          local.get 0
          local.get 10
          i64.store offset=371 align=1
          local.get 0
          i32.const 304
          i32.add
          i32.const 27
          i32.add
          local.tee 8
          local.get 1
          i32.load align=1
          i32.store align=1
          local.get 0
          i32.const 304
          i32.add
          i32.const 19
          i32.add
          local.tee 1
          local.get 2
          i64.load align=1
          i64.store align=1
          local.get 0
          i32.const 304
          i32.add
          i32.const 11
          i32.add
          local.tee 2
          local.get 3
          i64.load align=1
          i64.store align=1
          local.get 0
          local.get 0
          i64.load offset=371 align=1
          i64.store offset=307 align=1
          local.get 0
          i32.const 196
          i32.add
          local.tee 3
          local.get 8
          i32.load align=1
          i32.store
          local.get 0
          i32.const 188
          i32.add
          local.tee 8
          local.get 1
          i64.load align=1
          i64.store align=4
          local.get 0
          i32.const 180
          i32.add
          local.tee 1
          local.get 2
          i64.load align=1
          i64.store align=4
          local.get 0
          local.get 0
          i64.load offset=307 align=1
          i64.store offset=172 align=4
          local.get 0
          i32.const 136
          i32.add
          local.get 3
          i32.load
          i32.store
          local.get 0
          i32.const 128
          i32.add
          local.get 8
          i64.load align=4
          i64.store
          local.get 0
          i32.const 120
          i32.add
          local.get 1
          i64.load align=4
          i64.store
          local.get 0
          local.get 0
          i64.load offset=172 align=4
          i64.store offset=112
          local.get 0
          i32.const 112
          i32.add
          i32.const 1048672
          i32.const 1049028
          call $_ZN4core6result13unwrap_failed17h38a5f72e87633eadE
          unreachable
        end
        local.get 0
        i32.const 180
        i32.add
        i64.const 0
        i64.store align=4
        local.get 0
        i32.const 1
        i32.store offset=172
        local.get 0
        i32.const 1048988
        i32.store offset=168
        local.get 0
        i32.const 1049804
        i32.store offset=176
        local.get 0
        i32.const 168
        i32.add
        i32.const 1048996
        call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
        unreachable
      end
      local.get 0
      i32.const 180
      i32.add
      i64.const 0
      i64.store align=4
      local.get 0
      i32.const 1
      i32.store offset=172
      local.get 0
      i32.const 1048948
      i32.store offset=168
      local.get 0
      i32.const 1049804
      i32.store offset=176
      local.get 0
      i32.const 168
      i32.add
      i32.const 1048956
      call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
      unreachable
    end
    local.get 0
    i32.const 180
    i32.add
    i64.const 0
    i64.store align=4
    local.get 0
    i32.const 1
    i32.store offset=172
    local.get 0
    i32.const 1049060
    i32.store offset=168
    local.get 0
    i32.const 1049804
    i32.store offset=176
    local.get 0
    i32.const 168
    i32.add
    i32.const 1049068
    call $_ZN4core9panicking9panic_fmt17h78607b33a29a727dE
    unreachable)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h289f8e0c551f4d07E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    local.get 1
    call $_ZN41_$LT$char$u20$as$u20$core..fmt..Debug$GT$3fmt17hd37a4f87b17c40b9E)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h6951c5766b812742E (type 2) (param i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 0
            i32.load
            local.tee 0
            i32.load
            br_table 0 (;@4;) 1 (;@3;) 2 (;@2;) 0 (;@4;)
          end
          local.get 1
          i32.load offset=20
          i32.const 1053300
          i32.const 8
          local.get 1
          i32.const 24
          i32.add
          i32.load
          i32.load offset=12
          call_indirect (type 1)
          local.set 1
          br 2 (;@1;)
        end
        local.get 2
        local.get 0
        i32.const 8
        i32.add
        i32.store offset=4
        local.get 1
        i32.const 1053308
        i32.const 11
        local.get 2
        i32.const 4
        i32.add
        i32.const 1053320
        call $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE
        local.set 1
        br 1 (;@1;)
      end
      local.get 2
      local.get 0
      i32.const 16
      i32.add
      i32.store
      local.get 2
      local.get 1
      i32.load offset=20
      i32.const 1053336
      i32.const 12
      local.get 1
      i32.const 24
      i32.add
      i32.load
      i32.load offset=12
      call_indirect (type 1)
      i32.store8 offset=12
      local.get 2
      local.get 1
      i32.store offset=8
      local.get 2
      i32.const 0
      i32.store8 offset=13
      local.get 2
      i32.const 0
      i32.store offset=4
      local.get 2
      i32.const 4
      i32.add
      local.get 0
      i32.const 8
      i32.add
      i32.const 1053284
      call $_ZN4core3fmt8builders10DebugTuple5field17hbc9d984656bef3fdE
      local.get 2
      i32.const 1053320
      call $_ZN4core3fmt8builders10DebugTuple5field17hbc9d984656bef3fdE
      local.set 1
      local.get 2
      i32.load8_u offset=12
      local.set 0
      block  ;; label = @2
        local.get 1
        i32.load
        local.tee 3
        br_if 0 (;@2;)
        local.get 0
        i32.const 255
        i32.and
        i32.const 0
        i32.ne
        local.set 1
        br 1 (;@1;)
      end
      i32.const 1
      local.set 1
      local.get 0
      i32.const 255
      i32.and
      br_if 0 (;@1;)
      local.get 2
      i32.load offset=8
      local.set 0
      block  ;; label = @2
        local.get 3
        i32.const 1
        i32.ne
        br_if 0 (;@2;)
        local.get 2
        i32.load8_u offset=13
        i32.const 255
        i32.and
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        i32.load8_u offset=28
        i32.const 4
        i32.and
        br_if 0 (;@2;)
        i32.const 1
        local.set 1
        local.get 0
        i32.load offset=20
        i32.const 1050036
        i32.const 1
        local.get 0
        i32.const 24
        i32.add
        i32.load
        i32.load offset=12
        call_indirect (type 1)
        br_if 1 (;@1;)
      end
      local.get 0
      i32.load offset=20
      i32.const 1049847
      i32.const 1
      local.get 0
      i32.const 24
      i32.add
      i32.load
      i32.load offset=12
      call_indirect (type 1)
      local.set 1
    end
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 1)
  (func $_ZN4core3ptr24drop_in_place$LT$u64$GT$17h08a8e06d40a6cf17E (type 6) (param i32))
  (func $_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u64$GT$3fmt17h50bbec3a2579ee20E.475 (type 2) (param i32 i32) (result i32)
    (local i32 i32 i64 i32 i32)
    global.get $__stack_pointer
    i32.const 128
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 1
              i32.load offset=28
              local.tee 3
              i32.const 16
              i32.and
              br_if 0 (;@5;)
              local.get 3
              i32.const 32
              i32.and
              br_if 1 (;@4;)
              local.get 0
              i64.load
              local.get 1
              call $_ZN4core3fmt3num3imp7fmt_u6417h416abf9443aa8afdE
              local.set 0
              br 2 (;@3;)
            end
            local.get 0
            i64.load
            local.set 4
            i32.const 127
            local.set 3
            loop  ;; label = @5
              local.get 2
              local.get 3
              local.tee 0
              i32.add
              local.tee 5
              i32.const 48
              i32.const 87
              local.get 4
              i32.wrap_i64
              i32.const 15
              i32.and
              local.tee 3
              i32.const 10
              i32.lt_u
              select
              local.get 3
              i32.add
              i32.store8
              local.get 0
              i32.const -1
              i32.add
              local.set 3
              local.get 4
              i64.const 16
              i64.lt_u
              local.set 6
              local.get 4
              i64.const 4
              i64.shr_u
              local.set 4
              local.get 6
              i32.eqz
              br_if 0 (;@5;)
            end
            local.get 0
            i32.const 128
            i32.gt_u
            br_if 2 (;@2;)
            local.get 1
            i32.const 1050084
            i32.const 2
            local.get 5
            i32.const 129
            local.get 0
            i32.const 1
            i32.add
            i32.sub
            call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
            local.set 0
            br 1 (;@3;)
          end
          local.get 0
          i64.load
          local.set 4
          i32.const 127
          local.set 3
          loop  ;; label = @4
            local.get 2
            local.get 3
            local.tee 0
            i32.add
            local.tee 5
            i32.const 48
            i32.const 55
            local.get 4
            i32.wrap_i64
            i32.const 15
            i32.and
            local.tee 3
            i32.const 10
            i32.lt_u
            select
            local.get 3
            i32.add
            i32.store8
            local.get 0
            i32.const -1
            i32.add
            local.set 3
            local.get 4
            i64.const 16
            i64.lt_u
            local.set 6
            local.get 4
            i64.const 4
            i64.shr_u
            local.set 4
            local.get 6
            i32.eqz
            br_if 0 (;@4;)
          end
          local.get 0
          i32.const 128
          i32.gt_u
          br_if 2 (;@1;)
          local.get 1
          i32.const 1050084
          i32.const 2
          local.get 5
          i32.const 129
          local.get 0
          i32.const 1
          i32.add
          i32.sub
          call $_ZN4core3fmt9Formatter12pad_integral17h7a951cd58b7dcc45E
          local.set 0
        end
        local.get 2
        i32.const 128
        i32.add
        global.set $__stack_pointer
        local.get 0
        return
      end
      local.get 0
      i32.const 128
      i32.const 1050068
      call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
      unreachable
    end
    local.get 0
    i32.const 128
    i32.const 1050068
    call $_ZN4core5slice5index26slice_start_index_len_fail17hab50c0479c16b22eE
    unreachable)
  (func $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h926ca1870139b5b7E (type 2) (param i32 i32) (result i32)
    local.get 0
    i32.load
    local.get 1
    call $_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u64$GT$3fmt17h50bbec3a2579ee20E.475)
  (func $_ZN62_$LT$ruint..string..ParseError$u20$as$u20$core..fmt..Debug$GT$3fmt17h4a7a3ca15a8223f4E (type 2) (param i32 i32) (result i32)
    (local i32 i64)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 2
    global.set $__stack_pointer
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            local.get 0
            i64.load
            i64.const -3
            i64.add
            local.tee 3
            i64.const 2
            local.get 3
            i64.const 2
            i64.lt_u
            select
            i32.wrap_i64
            br_table 0 (;@4;) 1 (;@3;) 2 (;@2;) 0 (;@4;)
          end
          local.get 2
          local.get 0
          i32.const 8
          i32.add
          i32.store offset=4
          local.get 1
          i32.const 1053336
          i32.const 12
          local.get 2
          i32.const 4
          i32.add
          i32.const 1053348
          call $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE
          local.set 0
          br 2 (;@1;)
        end
        local.get 2
        local.get 0
        i32.const 8
        i32.add
        i32.store offset=8
        local.get 1
        i32.const 1053364
        i32.const 12
        local.get 2
        i32.const 8
        i32.add
        i32.const 1053320
        call $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE
        local.set 0
        br 1 (;@1;)
      end
      local.get 2
      local.get 0
      i32.store offset=12
      local.get 1
      i32.const 1053376
      i32.const 16
      local.get 2
      i32.const 12
      i32.add
      i32.const 1053392
      call $_ZN4core3fmt9Formatter25debug_tuple_field1_finish17h9427edf46c10eb0cE
      local.set 0
    end
    local.get 2
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 0)
  (func $_ZN88_$LT$wee_alloc..size_classes..SizeClassAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$22new_cell_for_free_list17h2dbdd2f6b9eb0b2fE (type 3) (param i32 i32 i32 i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 4
    global.set $__stack_pointer
    local.get 4
    local.get 1
    i32.load
    local.tee 5
    i32.load
    i32.store offset=12
    local.get 2
    i32.const 2
    i32.add
    local.tee 1
    local.get 1
    i32.mul
    local.tee 1
    i32.const 2048
    local.get 1
    i32.const 2048
    i32.gt_u
    select
    local.tee 2
    i32.const 4
    local.get 4
    i32.const 12
    i32.add
    i32.const 1049804
    i32.const 1053432
    call $_ZN9wee_alloc17alloc_with_refill17h36ec3c7d762c0443E
    local.set 1
    local.get 5
    local.get 4
    i32.load offset=12
    i32.store
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        br_if 0 (;@2;)
        i32.const 1
        local.set 2
        br 1 (;@1;)
      end
      local.get 1
      i64.const 0
      i64.store offset=4 align=4
      local.get 1
      local.get 1
      local.get 2
      i32.const 2
      i32.shl
      i32.add
      i32.const 2
      i32.or
      i32.store
      i32.const 0
      local.set 2
    end
    local.get 0
    local.get 1
    i32.store offset=4
    local.get 0
    local.get 2
    i32.store
    local.get 4
    i32.const 16
    i32.add
    global.set $__stack_pointer)
  (func $_ZN9wee_alloc17alloc_with_refill17h36ec3c7d762c0443E (type 11) (param i32 i32 i32 i32 i32) (result i32)
    (local i32 i32)
    global.get $__stack_pointer
    i32.const 16
    i32.sub
    local.tee 5
    global.set $__stack_pointer
    block  ;; label = @1
      local.get 0
      local.get 1
      local.get 2
      local.get 3
      local.get 4
      call $_ZN9wee_alloc15alloc_first_fit17h22da331dc42fcdb7E
      local.tee 6
      br_if 0 (;@1;)
      local.get 5
      i32.const 8
      i32.add
      local.get 3
      local.get 0
      local.get 1
      local.get 4
      i32.load offset=12
      call_indirect (type 3)
      i32.const 0
      local.set 6
      local.get 5
      i32.load offset=8
      br_if 0 (;@1;)
      local.get 5
      i32.load offset=12
      local.tee 6
      local.get 2
      i32.load
      i32.store offset=8
      local.get 2
      local.get 6
      i32.store
      local.get 0
      local.get 1
      local.get 2
      local.get 3
      local.get 4
      call $_ZN9wee_alloc15alloc_first_fit17h22da331dc42fcdb7E
      local.set 6
    end
    local.get 5
    i32.const 16
    i32.add
    global.set $__stack_pointer
    local.get 6)
  (func $_ZN9wee_alloc15alloc_first_fit17h22da331dc42fcdb7E (type 11) (param i32 i32 i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32)
    local.get 1
    i32.const -1
    i32.add
    local.set 5
    i32.const 0
    local.set 6
    i32.const 0
    local.get 1
    i32.sub
    local.set 7
    local.get 0
    i32.const 2
    i32.shl
    local.set 8
    local.get 2
    i32.load
    local.set 9
    loop (result i32)  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 9
          i32.eqz
          br_if 0 (;@3;)
          local.get 9
          local.set 1
          block  ;; label = @4
            block  ;; label = @5
              loop  ;; label = @6
                block  ;; label = @7
                  local.get 1
                  i32.load offset=8
                  local.tee 9
                  i32.const 1
                  i32.and
                  br_if 0 (;@7;)
                  local.get 1
                  i32.load
                  i32.const -4
                  i32.and
                  local.tee 10
                  local.get 1
                  i32.const 8
                  i32.add
                  local.tee 11
                  i32.sub
                  local.get 8
                  i32.lt_u
                  br_if 5 (;@2;)
                  block  ;; label = @8
                    local.get 11
                    local.get 3
                    local.get 0
                    local.get 4
                    i32.load offset=16
                    call_indirect (type 2)
                    i32.const 2
                    i32.shl
                    i32.add
                    i32.const 8
                    i32.add
                    local.get 10
                    local.get 8
                    i32.sub
                    local.get 7
                    i32.and
                    local.tee 9
                    i32.le_u
                    br_if 0 (;@8;)
                    local.get 11
                    i32.load
                    local.set 9
                    local.get 5
                    local.get 11
                    i32.and
                    br_if 6 (;@2;)
                    local.get 2
                    local.get 9
                    i32.const -4
                    i32.and
                    i32.store
                    local.get 1
                    i32.load
                    local.set 2
                    local.get 1
                    local.set 9
                    br 4 (;@4;)
                  end
                  i32.const 0
                  local.set 2
                  local.get 9
                  i32.const 0
                  i32.store
                  local.get 9
                  i32.const -8
                  i32.add
                  local.tee 9
                  i64.const 0
                  i64.store align=4
                  local.get 9
                  local.get 1
                  i32.load
                  i32.const -4
                  i32.and
                  i32.store
                  block  ;; label = @8
                    local.get 1
                    i32.load
                    local.tee 11
                    i32.const -4
                    i32.and
                    local.tee 8
                    i32.eqz
                    br_if 0 (;@8;)
                    i32.const 0
                    local.get 8
                    local.get 11
                    i32.const 2
                    i32.and
                    select
                    local.tee 11
                    i32.eqz
                    br_if 0 (;@8;)
                    local.get 11
                    local.get 11
                    i32.load offset=4
                    i32.const 3
                    i32.and
                    local.get 9
                    i32.or
                    i32.store offset=4
                    local.get 9
                    i32.load offset=4
                    i32.const 3
                    i32.and
                    local.set 2
                  end
                  local.get 9
                  local.get 2
                  local.get 1
                  i32.or
                  i32.store offset=4
                  local.get 1
                  local.get 1
                  i32.load offset=8
                  i32.const -2
                  i32.and
                  i32.store offset=8
                  local.get 1
                  local.get 1
                  i32.load
                  local.tee 2
                  i32.const 3
                  i32.and
                  local.get 9
                  i32.or
                  local.tee 11
                  i32.store
                  local.get 2
                  i32.const 2
                  i32.and
                  br_if 2 (;@5;)
                  local.get 9
                  i32.load
                  local.set 2
                  br 3 (;@4;)
                end
                local.get 1
                local.get 9
                i32.const -2
                i32.and
                i32.store offset=8
                block  ;; label = @7
                  block  ;; label = @8
                    local.get 1
                    i32.load offset=4
                    i32.const -4
                    i32.and
                    local.tee 9
                    br_if 0 (;@8;)
                    i32.const 0
                    local.set 9
                    br 1 (;@7;)
                  end
                  i32.const 0
                  local.get 9
                  local.get 9
                  i32.load8_u
                  i32.const 1
                  i32.and
                  select
                  local.set 9
                end
                local.get 1
                call $_ZN9wee_alloc9neighbors18Neighbors$LT$T$GT$6remove17hf16437817cec7526E
                block  ;; label = @7
                  local.get 1
                  i32.load8_u
                  i32.const 2
                  i32.and
                  i32.eqz
                  br_if 0 (;@7;)
                  local.get 9
                  local.get 9
                  i32.load
                  i32.const 2
                  i32.or
                  i32.store
                end
                local.get 2
                local.get 9
                i32.store
                local.get 9
                local.set 1
                br 0 (;@6;)
              end
            end
            local.get 1
            local.get 11
            i32.const -3
            i32.and
            i32.store
            local.get 9
            i32.load
            i32.const 2
            i32.or
            local.set 2
          end
          local.get 9
          local.get 2
          i32.const 1
          i32.or
          i32.store
          local.get 9
          i32.const 8
          i32.add
          local.set 6
        end
        local.get 6
        return
      end
      local.get 2
      local.get 9
      i32.store
      br 0 (;@1;)
    end)
  (func $_ZN9wee_alloc9neighbors18Neighbors$LT$T$GT$6remove17hf16437817cec7526E (type 6) (param i32)
    (local i32 i32 i32)
    block  ;; label = @1
      local.get 0
      i32.load
      local.tee 1
      i32.const -4
      i32.and
      local.tee 2
      i32.eqz
      br_if 0 (;@1;)
      i32.const 0
      local.get 2
      local.get 1
      i32.const 2
      i32.and
      select
      local.tee 2
      i32.eqz
      br_if 0 (;@1;)
      local.get 2
      local.get 2
      i32.load offset=4
      i32.const 3
      i32.and
      local.get 0
      i32.load offset=4
      i32.const -4
      i32.and
      i32.or
      i32.store offset=4
      local.get 0
      i32.load
      local.set 1
    end
    block  ;; label = @1
      local.get 0
      i32.load offset=4
      local.tee 2
      i32.const -4
      i32.and
      local.tee 3
      i32.eqz
      br_if 0 (;@1;)
      local.get 3
      local.get 3
      i32.load
      i32.const 3
      i32.and
      local.get 1
      i32.const -4
      i32.and
      i32.or
      i32.store
      local.get 0
      i32.load offset=4
      local.set 2
      local.get 0
      i32.load
      local.set 1
    end
    local.get 0
    local.get 2
    i32.const 3
    i32.and
    i32.store offset=4
    local.get 0
    local.get 1
    i32.const 3
    i32.and
    i32.store)
  (func $_ZN4core3ptr48drop_in_place$LT$wee_alloc..LargeAllocPolicy$GT$17h313790cd3427debeE (type 6) (param i32))
  (func $_ZN70_$LT$wee_alloc..LargeAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$22new_cell_for_free_list17hd26626a9a5c15f27E (type 3) (param i32 i32 i32 i32)
    block  ;; label = @1
      block  ;; label = @2
        local.get 2
        i32.const 2
        i32.shl
        local.tee 2
        local.get 3
        i32.const 3
        i32.shl
        i32.const 16384
        i32.add
        local.tee 3
        local.get 2
        local.get 3
        i32.gt_u
        select
        i32.const 65543
        i32.add
        local.tee 3
        i32.const 16
        i32.shr_u
        memory.grow
        local.tee 2
        i32.const -1
        i32.ne
        br_if 0 (;@2;)
        i32.const 1
        local.set 3
        i32.const 0
        local.set 2
        br 1 (;@1;)
      end
      local.get 2
      i32.const 16
      i32.shl
      local.tee 2
      i64.const 0
      i64.store offset=4 align=4
      local.get 2
      local.get 2
      local.get 3
      i32.const -65536
      i32.and
      i32.add
      i32.const 2
      i32.or
      i32.store
      i32.const 0
      local.set 3
    end
    local.get 0
    local.get 2
    i32.store offset=4
    local.get 0
    local.get 3
    i32.store)
  (func $_ZN70_$LT$wee_alloc..LargeAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$13min_cell_size17h9770fca7a4c7f4dfE (type 2) (param i32 i32) (result i32)
    i32.const 512)
  (func $_ZN70_$LT$wee_alloc..LargeAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$32should_merge_adjacent_free_cells17h0139b5342d8d3e07E (type 4) (param i32) (result i32)
    i32.const 1)
  (func $_ZN88_$LT$wee_alloc..size_classes..SizeClassAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$13min_cell_size17h8dd662e4ee4e1c91E (type 2) (param i32 i32) (result i32)
    local.get 1)
  (func $_ZN88_$LT$wee_alloc..size_classes..SizeClassAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$32should_merge_adjacent_free_cells17h58bded8ac15b670aE (type 4) (param i32) (result i32)
    i32.const 0)
  (func $_ZN9wee_alloc8WeeAlloc12dealloc_impl28_$u7b$$u7b$closure$u7d$$u7d$17h9341e162cd3d43dcE (type 3) (param i32 i32 i32 i32)
    (local i32)
    local.get 0
    i32.const 0
    i32.store
    local.get 0
    i32.const -8
    i32.add
    local.tee 4
    local.get 4
    i32.load
    i32.const -2
    i32.and
    i32.store
    block  ;; label = @1
      local.get 2
      local.get 3
      call_indirect (type 4)
      i32.eqz
      br_if 0 (;@1;)
      block  ;; label = @2
        block  ;; label = @3
          local.get 0
          i32.const -4
          i32.add
          i32.load
          i32.const -4
          i32.and
          local.tee 2
          i32.eqz
          br_if 0 (;@3;)
          local.get 2
          i32.load8_u
          i32.const 1
          i32.and
          br_if 0 (;@3;)
          local.get 4
          call $_ZN9wee_alloc9neighbors18Neighbors$LT$T$GT$6remove17hf16437817cec7526E
          local.get 4
          i32.load8_u
          i32.const 2
          i32.and
          i32.eqz
          br_if 1 (;@2;)
          local.get 2
          local.get 2
          i32.load
          i32.const 2
          i32.or
          i32.store
          return
        end
        local.get 4
        i32.load
        local.tee 2
        i32.const -4
        i32.and
        local.tee 3
        i32.eqz
        br_if 1 (;@1;)
        i32.const 0
        local.get 3
        local.get 2
        i32.const 2
        i32.and
        select
        local.tee 2
        i32.eqz
        br_if 1 (;@1;)
        local.get 2
        i32.load8_u
        i32.const 1
        i32.and
        br_if 1 (;@1;)
        local.get 0
        local.get 2
        i32.load offset=8
        i32.const -4
        i32.and
        i32.store
        local.get 2
        local.get 4
        i32.const 1
        i32.or
        i32.store offset=8
      end
      return
    end
    local.get 0
    local.get 1
    i32.load
    i32.store
    local.get 1
    local.get 4
    i32.store)
  (func $_ZN17compiler_builtins3mem6memcpy17h4ab6845275ba77f4E (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32 i32 i32 i32 i32 i32 i32)
    block  ;; label = @1
      block  ;; label = @2
        local.get 2
        i32.const 16
        i32.ge_u
        br_if 0 (;@2;)
        local.get 0
        local.set 3
        br 1 (;@1;)
      end
      local.get 0
      i32.const 0
      local.get 0
      i32.sub
      i32.const 3
      i32.and
      local.tee 4
      i32.add
      local.set 5
      block  ;; label = @2
        local.get 4
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        local.set 3
        local.get 1
        local.set 6
        loop  ;; label = @3
          local.get 3
          local.get 6
          i32.load8_u
          i32.store8
          local.get 6
          i32.const 1
          i32.add
          local.set 6
          local.get 3
          i32.const 1
          i32.add
          local.tee 3
          local.get 5
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 5
      local.get 2
      local.get 4
      i32.sub
      local.tee 7
      i32.const -4
      i32.and
      local.tee 8
      i32.add
      local.set 3
      block  ;; label = @2
        block  ;; label = @3
          local.get 1
          local.get 4
          i32.add
          local.tee 9
          i32.const 3
          i32.and
          i32.eqz
          br_if 0 (;@3;)
          local.get 8
          i32.const 1
          i32.lt_s
          br_if 1 (;@2;)
          local.get 9
          i32.const 3
          i32.shl
          local.tee 6
          i32.const 24
          i32.and
          local.set 2
          local.get 9
          i32.const -4
          i32.and
          local.tee 10
          i32.const 4
          i32.add
          local.set 1
          i32.const 0
          local.get 6
          i32.sub
          i32.const 24
          i32.and
          local.set 4
          local.get 10
          i32.load
          local.set 6
          loop  ;; label = @4
            local.get 5
            local.get 6
            local.get 2
            i32.shr_u
            local.get 1
            i32.load
            local.tee 6
            local.get 4
            i32.shl
            i32.or
            i32.store
            local.get 1
            i32.const 4
            i32.add
            local.set 1
            local.get 5
            i32.const 4
            i32.add
            local.tee 5
            local.get 3
            i32.lt_u
            br_if 0 (;@4;)
            br 2 (;@2;)
          end
        end
        local.get 8
        i32.const 1
        i32.lt_s
        br_if 0 (;@2;)
        local.get 9
        local.set 1
        loop  ;; label = @3
          local.get 5
          local.get 1
          i32.load
          i32.store
          local.get 1
          i32.const 4
          i32.add
          local.set 1
          local.get 5
          i32.const 4
          i32.add
          local.tee 5
          local.get 3
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 7
      i32.const 3
      i32.and
      local.set 2
      local.get 9
      local.get 8
      i32.add
      local.set 1
    end
    block  ;; label = @1
      local.get 2
      i32.eqz
      br_if 0 (;@1;)
      local.get 3
      local.get 2
      i32.add
      local.set 5
      loop  ;; label = @2
        local.get 3
        local.get 1
        i32.load8_u
        i32.store8
        local.get 1
        i32.const 1
        i32.add
        local.set 1
        local.get 3
        i32.const 1
        i32.add
        local.tee 3
        local.get 5
        i32.lt_u
        br_if 0 (;@2;)
      end
    end
    local.get 0)
  (func $_ZN17compiler_builtins3mem6memset17h7e84e2271aaccac9E (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32 i32)
    block  ;; label = @1
      block  ;; label = @2
        local.get 2
        i32.const 16
        i32.ge_u
        br_if 0 (;@2;)
        local.get 0
        local.set 3
        br 1 (;@1;)
      end
      local.get 0
      i32.const 0
      local.get 0
      i32.sub
      i32.const 3
      i32.and
      local.tee 4
      i32.add
      local.set 5
      block  ;; label = @2
        local.get 4
        i32.eqz
        br_if 0 (;@2;)
        local.get 0
        local.set 3
        loop  ;; label = @3
          local.get 3
          local.get 1
          i32.store8
          local.get 3
          i32.const 1
          i32.add
          local.tee 3
          local.get 5
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 5
      local.get 2
      local.get 4
      i32.sub
      local.tee 4
      i32.const -4
      i32.and
      local.tee 2
      i32.add
      local.set 3
      block  ;; label = @2
        local.get 2
        i32.const 1
        i32.lt_s
        br_if 0 (;@2;)
        local.get 1
        i32.const 255
        i32.and
        i32.const 16843009
        i32.mul
        local.set 2
        loop  ;; label = @3
          local.get 5
          local.get 2
          i32.store
          local.get 5
          i32.const 4
          i32.add
          local.tee 5
          local.get 3
          i32.lt_u
          br_if 0 (;@3;)
        end
      end
      local.get 4
      i32.const 3
      i32.and
      local.set 2
    end
    block  ;; label = @1
      local.get 2
      i32.eqz
      br_if 0 (;@1;)
      local.get 3
      local.get 2
      i32.add
      local.set 5
      loop  ;; label = @2
        local.get 3
        local.get 1
        i32.store8
        local.get 3
        i32.const 1
        i32.add
        local.tee 3
        local.get 5
        i32.lt_u
        br_if 0 (;@2;)
      end
    end
    local.get 0)
  (func $_ZN17compiler_builtins3mem6memcmp17h934ee432a6c6c000E (type 1) (param i32 i32 i32) (result i32)
    (local i32 i32 i32)
    i32.const 0
    local.set 3
    block  ;; label = @1
      local.get 2
      i32.eqz
      br_if 0 (;@1;)
      block  ;; label = @2
        loop  ;; label = @3
          local.get 0
          i32.load8_u
          local.tee 4
          local.get 1
          i32.load8_u
          local.tee 5
          i32.ne
          br_if 1 (;@2;)
          local.get 0
          i32.const 1
          i32.add
          local.set 0
          local.get 1
          i32.const 1
          i32.add
          local.set 1
          local.get 2
          i32.const -1
          i32.add
          local.tee 2
          i32.eqz
          br_if 2 (;@1;)
          br 0 (;@3;)
        end
      end
      local.get 4
      local.get 5
      i32.sub
      local.set 3
    end
    local.get 3)
  (func $memcmp (type 1) (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $_ZN17compiler_builtins3mem6memcmp17h934ee432a6c6c000E)
  (func $memcpy (type 1) (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $_ZN17compiler_builtins3mem6memcpy17h4ab6845275ba77f4E)
  (func $memset (type 1) (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $_ZN17compiler_builtins3mem6memset17h7e84e2271aaccac9E)
  (func $__multi3 (type 14) (param i32 i64 i64 i64 i64)
    (local i64 i64 i64 i64 i64 i64)
    local.get 0
    local.get 3
    i64.const 4294967295
    i64.and
    local.tee 5
    local.get 1
    i64.const 4294967295
    i64.and
    local.tee 6
    i64.mul
    local.tee 7
    local.get 3
    i64.const 32
    i64.shr_u
    local.tee 8
    local.get 6
    i64.mul
    local.tee 6
    local.get 5
    local.get 1
    i64.const 32
    i64.shr_u
    local.tee 9
    i64.mul
    i64.add
    local.tee 5
    i64.const 32
    i64.shl
    i64.add
    local.tee 10
    i64.store
    local.get 0
    local.get 8
    local.get 9
    i64.mul
    local.get 5
    local.get 6
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    local.get 5
    i64.const 32
    i64.shr_u
    i64.or
    i64.add
    local.get 10
    local.get 7
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 4
    local.get 1
    i64.mul
    local.get 3
    local.get 2
    i64.mul
    i64.add
    i64.add
    i64.store offset=8)
  (table (;0;) 45 45 funcref)
  (memory (;0;) 17)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1054488))
  (global (;2;) i32 (i32.const 1054496))
  (export "memory" (memory 0))
  (export "deploy" (func $deploy))
  (export "main" (func $main))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (elem (;0;) (i32.const 1) func $_ZN70_$LT$wee_alloc..LargeAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$32should_merge_adjacent_free_cells17h0139b5342d8d3e07E $_ZN88_$LT$wee_alloc..size_classes..SizeClassAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$32should_merge_adjacent_free_cells17h58bded8ac15b670aE $_ZN4core3fmt3num3imp52_$LT$impl$u20$core..fmt..Display$u20$for$u20$u32$GT$3fmt17he696c0e431156bceE $_ZN4core3ops8function6FnOnce9call_once17h3b98e4a5ab67f033E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h02a298d5b218d667E $_ZN44_$LT$$RF$T$u20$as$u20$core..fmt..Display$GT$3fmt17h743d7417ec5e8687E $_ZN71_$LT$core..ops..range..Range$LT$Idx$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17h938a7622bad5fb44E $_ZN41_$LT$char$u20$as$u20$core..fmt..Debug$GT$3fmt17hd37a4f87b17c40b9E $_ZN4core3ptr51drop_in_place$LT$alloy_sol_types..errors..Error$GT$17h005c935592b50f1bE $_ZN67_$LT$alloy_sol_types..errors..Error$u20$as$u20$core..fmt..Debug$GT$3fmt17hae0106c69bd4ae0eE $_ZN4core3ptr24drop_in_place$LT$u64$GT$17h08a8e06d40a6cf17E $_ZN62_$LT$ruint..string..ParseError$u20$as$u20$core..fmt..Debug$GT$3fmt17h4a7a3ca15a8223f4E $_ZN5bytes5bytes12static_clone17h796320c79f4bc4b4E $_ZN5bytes5bytes13static_to_vec17h3ffba84ffc73a880E $_ZN5bytes5bytes11static_drop17h78a6f2685218f0d1E $_ZN4core3ptr101drop_in_place$LT$alloc..vec..Vec$LT$alloy_primitives..bits..fixed..FixedBytes$LT$32_usize$GT$$GT$$GT$17hb9871b3d6a9bf65fE $_ZN65_$LT$alloc..vec..Vec$LT$T$C$A$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17he4e6f5a62f3f6b24E $_ZN4core3ptr25drop_in_place$LT$char$GT$17hc0b8a240319b7b4fE $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17hecf5f15880712f7eE $_ZN4core3ptr50drop_in_place$LT$alloc..borrow..Cow$LT$str$GT$$GT$17h8819b2f7b427b7ceE.195 $_ZN64_$LT$alloc..borrow..Cow$LT$B$GT$$u20$as$u20$core..fmt..Debug$GT$3fmt17he20c672607de4bccE $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h994d0780e9319ec1E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h331c597912658dd7E $_ZN4core3ptr23drop_in_place$LT$u8$GT$17ha25c34eed54542e7E $_ZN4core3fmt3num49_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u8$GT$3fmt17h61affea28b29a946E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h54ff38d57ce564c9E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h2406429923ed46efE $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h451fa72cb80b8158E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h0006516aa0b41931E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h3b800cbb107d7d7bE $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h3be23310966946e7E $_ZN4core3ptr102drop_in_place$LT$$RF$core..iter..adapters..copied..Copied$LT$core..slice..iter..Iter$LT$u8$GT$$GT$$GT$17h8916d5767b34df94E $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$9write_str17h85101c618d2f4861E $_ZN68_$LT$core..fmt..builders..PadAdapter$u20$as$u20$core..fmt..Write$GT$10write_char17h2f9f00c342b8af4cE $_ZN4core3fmt5Write9write_fmt17he38a55ddba5b1872E $_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u64$GT$3fmt17h50bbec3a2579ee20E.475 $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h926ca1870139b5b7E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h289f8e0c551f4d07E $_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h6951c5766b812742E $_ZN88_$LT$wee_alloc..size_classes..SizeClassAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$22new_cell_for_free_list17h2dbdd2f6b9eb0b2fE $_ZN88_$LT$wee_alloc..size_classes..SizeClassAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$13min_cell_size17h8dd662e4ee4e1c91E $_ZN4core3ptr48drop_in_place$LT$wee_alloc..LargeAllocPolicy$GT$17h313790cd3427debeE $_ZN70_$LT$wee_alloc..LargeAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$22new_cell_for_free_list17hd26626a9a5c15f27E $_ZN70_$LT$wee_alloc..LargeAllocPolicy$u20$as$u20$wee_alloc..AllocPolicy$GT$13min_cell_size17h9770fca7a4c7f4dfE)
  (data $.rodata (i32.const 1048576) "\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00called `Result::unwrap()` on an `Err` value\00\09\00\00\00\1c\00\00\00\04\00\00\00\0a\00\00\00\0b\00\00\00\18\00\00\00\08\00\00\00\0c\00\00\00\0d\00\00\00\0e\00\00\00\0f\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00 \00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00examples/src/erc20.rs\00\00\00\cc\00\10\00\15\00\00\00\22\00\00\00\1c\00\00\00\cc\00\10\00\15\00\00\00#\00\00\00\1d\00\00\001000000000000000000000000\00\00\00\cc\00\10\00\15\00\00\00+\00\00\00U\00\00\00TokenTOKinsufficient balance8\01\10\00\14\00\00\00\cc\00\10\00\15\00\00\00_\00\00\00\11\00\00\00invalid receiverd\01\10\00\10\00\00\00\cc\00\10\00\15\00\00\00T\00\00\00\0d\00\00\00invalid sender\00\00\8c\01\10\00\0e\00\00\00\cc\00\10\00\15\00\00\00R\00\00\00\0d\00\00\00\cc\00\10\00\15\00\00\00\99\00\00\00$\00\00\00\cc\00\10\00\15\00\00\00\9a\00\00\00#\00\00\00unknown method\00\00\d4\01\10\00\0e\00\00\00\cc\00\10\00\15\00\00\00\9b\00\00\00\0e\00\00\00library/alloc/src/raw_vec.rscapacity overflow\00\00\00\18\02\10\00\11\00\00\00\fc\01\10\00\1c\00\00\00\17\02\00\00\05\00\00\00memory allocation of  bytes failed\00\00D\02\10\00\15\00\00\00Y\02\10\00\0d\00\00\00/Users/dmitry/.cargo/registry/src/index.crates.io-6f17d22bba15001f/const-hex-1.10.0/src/lib.rs\00\00x\02\10\00^\00\00\00C\02\00\00\0c\00\00\00x\02\10\00^\00\00\00D\02\00\00\0c\00\00\00x\02\10\00^\00\00\00E\02\00\00\11\00\00\00Bytes(Logtopics\00\10\00\00\00\0c\00\00\00\04\00\00\00\11\00\00\00\12\00\00\00\04\00\00\00\04\00\00\00\13\00\00\00TypeCheckFailexpected_type\00\00\14\00\00\00\0c\00\00\00\04\00\00\00\15\00\00\00data\12\00\00\00\04\00\00\00\04\00\00\00\16\00\00\00OverrunBufferNotEmptyReserMismatchInvalidEnumValuename\00\00\12\00\00\00\08\00\00\00\04\00\00\00\17\00\00\00value\00\00\00\18\00\00\00\01\00\00\00\01\00\00\00\19\00\00\00max\00\12\00\00\00\04\00\00\00\04\00\00\00\1a\00\00\00InvalidLoglog\00\00\00\12\00\00\00\04\00\00\00\04\00\00\00\1b\00\00\00UnknownSelectorselector\00\12\00\00\00\04\00\00\00\04\00\00\00\1c\00\00\00FromHexError\12\00\00\00\04\00\00\00\04\00\00\00\1d\00\00\00Other\00\00\00\12\00\00\00\04\00\00\00\04\00\00\00\1e\00\00\00\0d\00\00\00\0e\00\00\00\0f\00\00\00InvalidHexCharacterc\12\00\00\00\04\00\00\00\04\00\00\00\08\00\00\00index\00\00\00\12\00\00\00\04\00\00\00\04\00\00\00\1f\00\00\00OddLengthInvalidStringLengthcalled `Option::unwrap()` on a `None` value)library/core/src/fmt/mod.rs..\00\00\00\13\05\10\00\02\00\00\000123456789abcdef[index out of bounds: the len is  but the index is \001\05\10\00 \00\00\00Q\05\10\00\12\00\00\00: \00\00\cc\04\10\00\00\00\00\00t\05\10\00\02\00\00\00 \00\00\00\0c\00\00\00\04\00\00\00!\00\00\00\22\00\00\00#\00\00\00     { ,  {\0a,\0a} }((\0a,\0a]library/core/src/fmt/num.rs\00\00\b7\05\10\00\1b\00\00\00i\00\00\00\17\00\00\000x00010203040506070809101112131415161718192021222324252627282930313233343536373839404142434445464748495051525354555657585960616263646566676869707172737475767778798081828384858687888990919293949596979899\00\00\f8\04\10\00\1b\00\00\002\09\00\00\1a\00\00\00\f8\04\10\00\1b\00\00\00+\09\00\00\22\00\00\00range start index  out of range for slice of length \d0\06\10\00\12\00\00\00\e2\06\10\00\22\00\00\00range end index \14\07\10\00\10\00\00\00\e2\06\10\00\22\00\00\00slice index starts at  but ends at \004\07\10\00\16\00\00\00J\07\10\00\0d\00\00\00source slice length () does not match destination slice length (h\07\10\00\15\00\00\00}\07\10\00+\00\00\00\f7\04\10\00\01\00\00\00[...]begin <= end ( <= ) when slicing ``\c5\07\10\00\0e\00\00\00\d3\07\10\00\04\00\00\00\d7\07\10\00\10\00\00\00\e7\07\10\00\01\00\00\00byte index  is not a char boundary; it is inside  (bytes ) of `\00\08\08\10\00\0b\00\00\00\13\08\10\00&\00\00\009\08\10\00\08\00\00\00A\08\10\00\06\00\00\00\e7\07\10\00\01\00\00\00 is out of bounds of `\00\00\08\08\10\00\0b\00\00\00p\08\10\00\16\00\00\00\e7\07\10\00\01\00\00\00library/core/src/str/mod.rs\00\a0\08\10\00\1b\00\00\00\09\01\00\00,\00\00\00library/core/src/unicode/printable.rs\00\00\00\cc\08\10\00%\00\00\00\1a\00\00\006\00\00\00\cc\08\10\00%\00\00\00\0a\00\00\00+\00\00\00\00\06\01\01\03\01\04\02\05\07\07\02\08\08\09\02\0a\05\0b\02\0e\04\10\01\11\02\12\05\13\11\14\01\15\02\17\02\19\0d\1c\05\1d\08\1f\01$\01j\04k\02\af\03\b1\02\bc\02\cf\02\d1\02\d4\0c\d5\09\d6\02\d7\02\da\01\e0\05\e1\02\e7\04\e8\02\ee \f0\04\f8\02\fa\03\fb\01\0c';>NO\8f\9e\9e\9f{\8b\93\96\a2\b2\ba\86\b1\06\07\096=>V\f3\d0\d1\04\14\1867VW\7f\aa\ae\af\bd5\e0\12\87\89\8e\9e\04\0d\0e\11\12)14:EFIJNOde\5c\b6\b7\1b\1c\07\08\0a\0b\14\1769:\a8\a9\d8\d9\097\90\91\a8\07\0a;>fi\8f\92\11o_\bf\ee\efZb\f4\fc\ffST\9a\9b./'(U\9d\a0\a1\a3\a4\a7\a8\ad\ba\bc\c4\06\0b\0c\15\1d:?EQ\a6\a7\cc\cd\a0\07\19\1a\22%>?\e7\ec\ef\ff\c5\c6\04 #%&(38:HJLPSUVXZ\5c^`cefksx}\7f\8a\a4\aa\af\b0\c0\d0\ae\afno\be\93^\22{\05\03\04-\03f\03\01/.\80\82\1d\031\0f\1c\04$\09\1e\05+\05D\04\0e*\80\aa\06$\04$\04(\084\0bNC\817\09\16\0a\08\18;E9\03c\08\090\16\05!\03\1b\05\01@8\04K\05/\04\0a\07\09\07@ '\04\0c\096\03:\05\1a\07\04\0c\07PI73\0d3\07.\08\0a\81&RK+\08*\16\1a&\1c\14\17\09N\04$\09D\0d\19\07\0a\06H\08'\09u\0bB>*\06;\05\0a\06Q\06\01\05\10\03\05\80\8bb\1eH\08\0a\80\a6^\22E\0b\0a\06\0d\13:\06\0a6,\04\17\80\b9<dS\0cH\09\0aFE\1bH\08S\0dI\07\0a\80\f6F\0a\1d\03GI7\03\0e\08\0a\069\07\0a\816\19\07;\03\1cV\01\0f2\0d\83\9bfu\0b\80\c4\8aLc\0d\840\10\16\8f\aa\82G\a1\b9\829\07*\04\5c\06&\0aF\0a(\05\13\82\b0[eK\049\07\11@\05\0b\02\0e\97\f8\08\84\d6*\09\a2\e7\813\0f\01\1d\06\0e\04\08\81\8c\89\04k\05\0d\03\09\07\10\92`G\09t<\80\f6\0as\08p\15Fz\14\0c\14\0cW\09\19\80\87\81G\03\85B\0f\15\84P\1f\06\06\80\d5+\05>!\01p-\03\1a\04\02\81@\1f\11:\05\01\81\d0*\82\e6\80\f7)L\04\0a\04\02\83\11DL=\80\c2<\06\01\04U\05\1b4\02\81\0e,\04d\0cV\0a\80\ae8\1d\0d,\04\09\07\02\0e\06\80\9a\83\d8\04\11\03\0d\03w\04_\06\0c\04\01\0f\0c\048\08\0a\06(\08\22N\81T\0c\1d\03\09\076\08\0e\04\09\07\09\07\80\cb%\0a\84\06\00\01\03\05\05\06\06\02\07\06\08\07\09\11\0a\1c\0b\19\0c\1a\0d\10\0e\0c\0f\04\10\03\12\12\13\09\16\01\17\04\18\01\19\03\1a\07\1b\01\1c\02\1f\16 \03+\03-\0b.\010\031\022\01\a7\02\a9\02\aa\04\ab\08\fa\02\fb\05\fd\02\fe\03\ff\09\adxy\8b\8d\a20WX\8b\8c\90\1c\dd\0e\0fKL\fb\fc./?\5c]_\e2\84\8d\8e\91\92\a9\b1\ba\bb\c5\c6\c9\ca\de\e4\e5\ff\00\04\11\12)147:;=IJ]\84\8e\92\a9\b1\b4\ba\bb\c6\ca\ce\cf\e4\e5\00\04\0d\0e\11\12)14:;EFIJ^de\84\91\9b\9d\c9\ce\cf\0d\11):;EIW[\5c^_de\8d\91\a9\b4\ba\bb\c5\c9\df\e4\e5\f0\0d\11EIde\80\84\b2\bc\be\bf\d5\d7\f0\f1\83\85\8b\a4\a6\be\bf\c5\c7\cf\da\dbH\98\bd\cd\c6\ce\cfINOWY^_\89\8e\8f\b1\b6\b7\bf\c1\c6\c7\d7\11\16\17[\5c\f6\f7\fe\ff\80mq\de\df\0e\1fno\1c\1d_}~\ae\af\7f\bb\bc\16\17\1e\1fFGNOXZ\5c^~\7f\b5\c5\d4\d5\dc\f0\f1\f5rs\8ftu\96&./\a7\af\b7\bf\c7\cf\d7\df\9a@\97\980\8f\1f\d2\d4\ce\ffNOZ[\07\08\0f\10'/\ee\efno7=?BE\90\91Sgu\c8\c9\d0\d1\d8\d9\e7\fe\ff\00 _\22\82\df\04\82D\08\1b\04\06\11\81\ac\0e\80\ab\05\1f\09\81\1b\03\19\08\01\04/\044\04\07\03\01\07\06\07\11\0aP\0f\12\07U\07\03\04\1c\0a\09\03\08\03\07\03\02\03\03\03\0c\04\05\03\0b\06\01\0e\15\05N\07\1b\07W\07\02\06\17\0cP\04C\03-\03\01\04\11\06\0f\0c:\04\1d%_ m\04j%\80\c8\05\82\b0\03\1a\06\82\fd\03Y\07\16\09\18\09\14\0c\14\0cj\06\0a\06\1a\06Y\07+\05F\0a,\04\0c\04\01\031\0b,\04\1a\06\0b\03\80\ac\06\0a\06/1M\03\80\a4\08<\03\0f\03<\078\08+\05\82\ff\11\18\08/\11-\03!\0f!\0f\80\8c\04\82\97\19\0b\15\88\94\05/\05;\07\02\0e\18\09\80\be\22t\0c\80\d6\1a\0c\05\80\ff\05\80\df\0c\f2\9d\037\09\81\5c\14\80\b8\08\80\cb\05\0a\18;\03\0a\068\08F\08\0c\06t\0b\1e\03Z\04Y\09\80\83\18\1c\0a\16\09L\04\80\8a\06\ab\a4\0c\17\041\a1\04\81\da&\07\0c\05\05\80\a6\10\81\f5\07\01 *\06L\04\80\8d\04\80\be\03\1b\03\0f\0dlibrary/core/src/unicode/unicode_data.rs\90\0e\10\00(\00\00\00P\00\00\00(\00\00\00\90\0e\10\00(\00\00\00\5c\00\00\00\16\00\00\00library/core/src/escape.rs\5cu{\00\00\00\d8\0e\10\00\1a\00\00\00b\00\00\00#\00\00\00\00\03\00\00\83\04 \00\91\05`\00]\13\a0\00\12\17 \1f\0c `\1f\ef,\a0+*0 ,o\a6\e0,\02\a8`-\1e\fb`.\00\fe 6\9e\ff`6\fd\01\e16\01\0a!7$\0d\e17\ab\0ea9/\18\a190\1caH\f3\1e\a1L@4aP\f0j\a1QOo!R\9d\bc\a1R\00\cfaSe\d1\a1S\00\da!T\00\e0\e1U\ae\e2aW\ec\e4!Y\d0\e8\a1Y \00\eeY\f0\01\7fZ\00p\00\07\00-\01\01\01\02\01\02\01\01H\0b0\15\10\01e\07\02\06\02\02\01\04#\01\1e\1b[\0b:\09\09\01\18\04\01\09\01\03\01\05+\03<\08*\18\01 7\01\01\01\04\08\04\01\03\07\0a\02\1d\01:\01\01\01\02\04\08\01\09\01\0a\02\1a\01\02\029\01\04\02\04\02\02\03\03\01\1e\02\03\01\0b\029\01\04\05\01\02\04\01\14\02\16\06\01\01:\01\01\02\01\04\08\01\07\03\0a\02\1e\01;\01\01\01\0c\01\09\01(\01\03\017\01\01\03\05\03\01\04\07\02\0b\02\1d\01:\01\02\01\02\01\03\01\05\02\07\02\0b\02\1c\029\02\01\01\02\04\08\01\09\01\0a\02\1d\01H\01\04\01\02\03\01\01\08\01Q\01\02\07\0c\08b\01\02\09\0b\07I\02\1b\01\01\01\01\017\0e\01\05\01\02\05\0b\01$\09\01f\04\01\06\01\02\02\02\19\02\04\03\10\04\0d\01\02\02\06\01\0f\01\00\03\00\03\1d\02\1e\02\1e\02@\02\01\07\08\01\02\0b\09\01-\03\01\01u\02\22\01v\03\04\02\09\01\06\03\db\02\02\01:\01\01\07\01\01\01\01\02\08\06\0a\02\010\1f1\040\07\01\01\05\01(\09\0c\02 \04\02\02\01\038\01\01\02\03\01\01\03:\08\02\02\98\03\01\0d\01\07\04\01\06\01\03\02\c6@\00\01\c3!\00\03\8d\01` \00\06i\02\00\04\01\0a \02P\02\00\01\03\01\04\01\19\02\05\01\97\02\1a\12\0d\01&\08\19\0b.\030\01\02\04\02\02'\01C\06\02\02\02\02\0c\01\08\01/\013\01\01\03\02\02\05\02\01\01*\02\08\01\ee\01\02\01\04\01\00\01\00\10\10\10\00\02\00\01\e2\01\95\05\00\03\01\02\05\04(\03\04\01\a5\02\00\04\00\02P\03F\0b1\04{\016\0f)\01\02\02\0a\031\04\02\02\07\01=\03$\05\01\08>\01\0c\024\09\0a\04\02\01_\03\02\01\01\02\06\01\02\01\9d\01\03\08\15\029\02\01\01\01\01\16\01\0e\07\03\05\c3\08\02\03\01\01\17\01Q\01\02\06\01\01\02\01\01\02\01\02\eb\01\02\04\06\02\01\02\1b\02U\08\02\01\01\02j\01\01\01\02\06\01\01e\03\02\04\01\05\00\09\01\02\f5\01\0a\02\01\01\04\01\90\04\02\02\04\01 \0a(\06\02\04\08\01\09\06\02\03.\0d\01\02\00\07\01\06\01\01R\16\02\07\01\02\01\02z\06\03\01\01\02\01\07\01\01H\02\03\01\01\01\00\02\0b\024\05\05\01\01\01\00\01\06\0f\00\05;\07\00\01?\04Q\01\00\02\00.\02\17\00\01\01\03\04\05\08\08\02\07\1e\04\94\03\007\042\08\01\0e\01\16\05\01\0f\00\07\01\11\02\07\01\02\01\05d\01\a0\07\00\01=\04\00\04\00\07m\07\00`\80\f0\00\00\0b\00\00\00\08\00\00\00\08\00\00\00$\00\00\00OverflowInvalidBase\00\12\00\00\00\04\00\00\00\04\00\00\00%\00\00\00InvalidDigit\12\00\00\00\04\00\00\00\04\00\00\00&\00\00\00InvalidRadixBaseConvertError\12\00\00\00\04\00\00\00\04\00\00\00'\00\00\00\12\00\00\00\04\00\00\00\04\00\00\00(\00\00\00)\00\00\00\02\00\00\00*\00\00\00\00\00\00\00\01\00\00\00+\00\00\00,\00\00\00\01\00\00\00"))
