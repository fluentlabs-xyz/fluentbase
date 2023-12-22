(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    (local i64 i32 i64 i64 i64 i64 i32 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64)
    i32.const 32768
    i32.const 0
    i64.load offset=32768
    local.tee 0
    i32.wrap_i64
    local.tee 1
    i32.sub
    i64.load align=1
    local.set 2
    i32.const 32776
    local.get 1
    i32.sub
    i64.load align=1
    local.set 3
    i32.const 32784
    local.get 1
    i32.sub
    i64.load align=1
    local.set 4
    i32.const 32792
    local.get 1
    i32.sub
    i64.load align=1
    local.set 5
    i32.const 0
    local.get 0
    i64.const 32
    i64.shl
    i64.const -137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 0
    i64.store offset=32768
    i32.const 32792
    local.get 0
    i32.wrap_i64
    local.tee 1
    i32.sub
    local.tee 6
    local.get 6
    i64.load align=1
    local.tee 0
    i64.const 56
    i64.shl
    local.get 0
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 0
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 0
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.tee 7
    local.get 0
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 0
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 0
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 0
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 8
    i64.const 4294967295
    i64.and
    local.tee 9
    local.get 5
    i64.const 56
    i64.shl
    local.get 5
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 5
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 5
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.tee 0
    local.get 5
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 5
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 5
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 5
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 10
    i64.const 4294967295
    i64.and
    local.tee 11
    i64.mul
    local.tee 12
    local.get 7
    i64.const 32
    i64.shr_u
    local.tee 7
    local.get 11
    i64.mul
    local.tee 13
    local.get 9
    local.get 0
    i64.const 32
    i64.shr_u
    local.tee 14
    i64.mul
    i64.add
    local.tee 15
    i64.const 32
    i64.shl
    i64.add
    local.tee 5
    i64.const 56
    i64.shl
    local.get 5
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 5
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 5
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 5
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 5
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 5
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 5
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 32784
    local.get 1
    i32.sub
    local.tee 6
    local.get 6
    i64.load align=1
    local.tee 0
    i64.const 56
    i64.shl
    local.get 0
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 0
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 0
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.tee 16
    local.get 0
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 0
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 0
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 0
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 17
    i64.const 4294967295
    i64.and
    local.tee 0
    local.get 11
    i64.mul
    local.tee 18
    local.get 16
    i64.const 32
    i64.shr_u
    local.tee 16
    local.get 11
    i64.mul
    local.tee 19
    local.get 0
    local.get 14
    i64.mul
    i64.add
    local.tee 20
    i64.const 32
    i64.shl
    i64.add
    local.tee 21
    local.get 9
    local.get 4
    i64.const 56
    i64.shl
    local.get 4
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 4
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
    local.tee 22
    local.get 4
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 4
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 4
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 4
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 23
    i64.const 4294967295
    i64.and
    local.tee 24
    i64.mul
    local.tee 25
    local.get 7
    local.get 24
    i64.mul
    local.tee 26
    local.get 9
    local.get 22
    i64.const 32
    i64.shr_u
    local.tee 22
    i64.mul
    i64.add
    local.tee 27
    i64.const 32
    i64.shl
    i64.add
    local.tee 28
    local.get 15
    i64.const 32
    i64.shr_u
    local.get 7
    local.get 14
    i64.mul
    i64.add
    local.get 15
    local.get 13
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    i64.add
    local.get 5
    local.get 12
    i64.lt_u
    i64.extend_i32_u
    i64.add
    i64.add
    local.tee 12
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
    local.get 4
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
    local.get 4
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 4
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 4
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 4
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 32776
    local.get 1
    i32.sub
    local.tee 6
    local.get 6
    i64.load align=1
    local.tee 5
    i64.const 56
    i64.shl
    local.get 5
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 5
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 5
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.tee 15
    local.get 5
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 5
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 5
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 5
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 13
    i64.const 4294967295
    i64.and
    local.tee 5
    local.get 11
    i64.mul
    local.tee 29
    local.get 15
    i64.const 32
    i64.shr_u
    local.tee 30
    local.get 11
    i64.mul
    local.tee 31
    local.get 5
    local.get 14
    i64.mul
    i64.add
    local.tee 5
    i64.const 32
    i64.shl
    i64.add
    local.tee 11
    local.get 0
    local.get 24
    i64.mul
    local.tee 32
    local.get 16
    local.get 24
    i64.mul
    local.tee 33
    local.get 0
    local.get 22
    i64.mul
    i64.add
    local.tee 0
    i64.const 32
    i64.shl
    i64.add
    local.tee 24
    local.get 20
    i64.const 32
    i64.shr_u
    local.get 16
    local.get 14
    i64.mul
    i64.add
    local.get 20
    local.get 19
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    i64.add
    local.get 21
    local.get 18
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.tee 18
    local.get 4
    local.get 21
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.tee 15
    local.get 9
    local.get 3
    i64.const 56
    i64.shl
    local.get 3
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 3
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 3
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.tee 4
    local.get 3
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 3
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 3
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 3
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.tee 21
    i64.const 4294967295
    i64.and
    local.tee 3
    i64.mul
    local.tee 19
    local.get 7
    local.get 3
    i64.mul
    local.tee 34
    local.get 9
    local.get 4
    i64.const 32
    i64.shr_u
    local.tee 35
    i64.mul
    i64.add
    local.tee 9
    i64.const 32
    i64.shl
    i64.add
    local.tee 20
    local.get 27
    i64.const 32
    i64.shr_u
    local.get 7
    local.get 22
    i64.mul
    i64.add
    local.get 27
    local.get 26
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    i64.add
    local.get 28
    local.get 25
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.tee 27
    local.get 12
    local.get 28
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.tee 28
    i64.add
    local.tee 12
    i64.add
    local.tee 25
    i64.add
    local.tee 26
    i64.add
    local.tee 3
    i64.const 56
    i64.shl
    local.get 3
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 3
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 3
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 3
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 3
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 3
    i64.const 40
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
    i32.const 32768
    local.get 1
    i32.sub
    local.tee 1
    local.get 13
    local.get 23
    i64.mul
    local.get 1
    i64.load align=1
    local.tee 4
    i64.const 56
    i64.shl
    local.get 4
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 4
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
    local.get 4
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 4
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 4
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 4
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    local.get 10
    i64.mul
    i64.add
    local.get 30
    local.get 14
    i64.mul
    i64.add
    local.get 17
    local.get 21
    i64.mul
    i64.add
    local.get 8
    local.get 2
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
    i64.mul
    i64.add
    local.get 16
    local.get 22
    i64.mul
    i64.add
    local.get 7
    local.get 35
    i64.mul
    i64.add
    local.get 5
    i64.const 32
    i64.shr_u
    i64.add
    local.get 5
    local.get 31
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    i64.add
    local.get 0
    i64.const 32
    i64.shr_u
    i64.add
    local.get 0
    local.get 33
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    i64.add
    local.get 9
    i64.const 32
    i64.shr_u
    i64.add
    local.get 9
    local.get 34
    i64.lt_u
    i64.extend_i32_u
    i64.const 32
    i64.shl
    i64.add
    local.get 11
    local.get 29
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 24
    local.get 32
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 20
    local.get 19
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 28
    local.get 27
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 12
    local.get 20
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 15
    local.get 18
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 25
    local.get 15
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 26
    local.get 24
    i64.lt_u
    i64.extend_i32_u
    i64.add
    local.get 3
    local.get 11
    i64.lt_u
    i64.extend_i32_u
    i64.add
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
    i64.store align=1)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_mul" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
