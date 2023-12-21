(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (type (;2;) (func (param i32 i32)))
  (func $arithmetic_sdiv (type 0)
    (local i32 i64 i64 i64 i64 i64 i64 i64 i64 i64 i32 i64 i64 i64 i64 i64 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    global.get $__stack_pointer
    i32.const 272
    i32.sub
    local.tee 0
    global.set $__stack_pointer
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h1d18dbc2fbdc0d73E
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h1d18dbc2fbdc0d73E
    local.get 0
    i32.const 64
    i32.add
    local.get 0
    i32.const 32
    i32.add
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h7a03d9a932c8cd0eE
    local.get 0
    i32.const 96
    i32.add
    local.get 0
    call $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h7a03d9a932c8cd0eE
    local.get 0
    i64.load offset=120
    local.set 1
    local.get 0
    i64.load offset=104
    local.set 2
    local.get 0
    i64.load offset=112
    local.set 3
    local.get 0
    i64.load offset=80
    local.set 4
    local.get 0
    i64.load offset=72
    local.set 5
    local.get 0
    i64.load offset=64
    local.set 6
    local.get 0
    i64.load offset=88
    local.set 7
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                local.get 0
                i64.load offset=96
                local.tee 8
                i64.const 1
                i64.gt_u
                br_if 0 (;@6;)
                local.get 2
                local.get 1
                i64.or
                local.get 3
                i64.or
                i64.const 0
                i64.eq
                br_if 1 (;@5;)
              end
              local.get 8
              local.get 1
              i64.and
              local.get 2
              i64.and
              local.get 3
              i64.and
              i64.const -1
              i64.eq
              br_if 0 (;@5;)
              block  ;; label = @6
                block  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      local.get 6
                      local.get 8
                      i64.ne
                      br_if 0 (;@9;)
                      local.get 5
                      local.get 2
                      i64.ne
                      br_if 0 (;@9;)
                      local.get 7
                      local.get 1
                      i64.ne
                      br_if 0 (;@9;)
                      local.get 4
                      local.get 3
                      i64.eq
                      br_if 1 (;@8;)
                    end
                    local.get 7
                    i64.const 0
                    i64.lt_s
                    br_if 1 (;@7;)
                    local.get 7
                    local.set 9
                    br 2 (;@6;)
                  end
                  local.get 6
                  i64.eqz
                  br_if 3 (;@4;)
                  i64.const -1
                  i64.const 1
                  local.get 7
                  i64.const 0
                  i64.lt_s
                  local.get 1
                  i64.const 0
                  i64.lt_s
                  i32.xor
                  local.tee 10
                  select
                  local.set 11
                  i64.const 0
                  local.get 10
                  i64.extend_i32_u
                  i64.sub
                  local.tee 12
                  local.set 13
                  local.get 12
                  local.set 14
                  br 6 (;@1;)
                end
                local.get 0
                local.get 7
                i64.store offset=248
                local.get 0
                local.get 4
                i64.store offset=240
                local.get 0
                local.get 5
                i64.store offset=232
                local.get 0
                local.get 6
                i64.store offset=224
                local.get 0
                i32.const 192
                i32.add
                local.get 0
                i32.const 224
                i32.add
                call $_ZN30fluentbase_rwasm_code_snippets6common15convert_sign_le17hd7eaa31eb5c86d58E
                local.get 0
                i64.load offset=192
                local.set 6
                local.get 0
                i64.load offset=200
                local.set 5
                local.get 0
                i64.load offset=208
                local.set 4
                local.get 0
                i64.load offset=216
                local.set 9
              end
              block  ;; label = @6
                block  ;; label = @7
                  local.get 1
                  i64.const 0
                  i64.lt_s
                  br_if 0 (;@7;)
                  local.get 1
                  local.set 15
                  br 1 (;@6;)
                end
                local.get 0
                local.get 1
                i64.store offset=248
                local.get 0
                local.get 3
                i64.store offset=240
                local.get 0
                local.get 2
                i64.store offset=232
                local.get 0
                local.get 8
                i64.store offset=224
                local.get 0
                i32.const 192
                i32.add
                local.get 0
                i32.const 224
                i32.add
                call $_ZN30fluentbase_rwasm_code_snippets6common15convert_sign_le17hd7eaa31eb5c86d58E
                local.get 0
                i64.load offset=192
                local.set 8
                local.get 0
                i64.load offset=200
                local.set 2
                local.get 0
                i64.load offset=208
                local.set 3
                local.get 0
                i64.load offset=216
                local.set 15
              end
              block  ;; label = @6
                local.get 9
                local.get 15
                i64.gt_u
                br_if 0 (;@6;)
                block  ;; label = @7
                  local.get 9
                  local.get 15
                  i64.ne
                  br_if 0 (;@7;)
                  local.get 4
                  local.get 3
                  i64.gt_u
                  br_if 1 (;@6;)
                end
                block  ;; label = @7
                  local.get 9
                  local.get 15
                  i64.eq
                  local.get 4
                  local.get 3
                  i64.eq
                  i32.and
                  local.tee 10
                  i32.const 1
                  i32.ne
                  br_if 0 (;@7;)
                  local.get 5
                  local.get 2
                  i64.gt_u
                  br_if 1 (;@6;)
                end
                i64.const 0
                local.set 12
                local.get 10
                local.get 5
                local.get 2
                i64.eq
                i32.and
                i32.const 1
                i32.ne
                br_if 3 (;@3;)
                i64.const 0
                local.set 13
                i64.const 0
                local.set 14
                i64.const 0
                local.set 11
                local.get 6
                local.get 8
                i64.le_u
                br_if 5 (;@1;)
              end
              local.get 0
              i32.const 128
              i32.add
              i32.const 24
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 128
              i32.add
              i32.const 16
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 128
              i32.add
              i32.const 8
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i64.const 0
              i64.store offset=128
              local.get 0
              i32.const 160
              i32.add
              i32.const 24
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 160
              i32.add
              i32.const 16
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 160
              i32.add
              i32.const 8
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i64.const 0
              i64.store offset=160
              local.get 0
              i32.const 192
              i32.add
              i32.const 24
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 192
              i32.add
              i32.const 16
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 192
              i32.add
              i32.const 8
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i64.const 0
              i64.store offset=192
              local.get 0
              i32.const 224
              i32.add
              i32.const 24
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 224
              i32.add
              i32.const 16
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i32.const 224
              i32.add
              i32.const 8
              i32.add
              i64.const 0
              i64.store
              local.get 0
              i64.const 0
              i64.store offset=224
              local.get 8
              i64.const 56
              i64.shl
              local.get 8
              i64.const 65280
              i64.and
              i64.const 40
              i64.shl
              i64.or
              local.get 8
              i64.const 16711680
              i64.and
              i64.const 24
              i64.shl
              local.get 8
              i64.const 4278190080
              i64.and
              i64.const 8
              i64.shl
              i64.or
              i64.or
              local.get 8
              i64.const 8
              i64.shr_u
              i64.const 4278190080
              i64.and
              local.get 8
              i64.const 24
              i64.shr_u
              i64.const 16711680
              i64.and
              i64.or
              local.get 8
              i64.const 40
              i64.shr_u
              i64.const 65280
              i64.and
              local.get 8
              i64.const 56
              i64.shr_u
              i64.or
              i64.or
              i64.or
              local.set 12
              local.get 6
              i64.const 56
              i64.shl
              local.get 6
              i64.const 65280
              i64.and
              i64.const 40
              i64.shl
              i64.or
              local.get 6
              i64.const 16711680
              i64.and
              i64.const 24
              i64.shl
              local.get 6
              i64.const 4278190080
              i64.and
              i64.const 8
              i64.shl
              i64.or
              i64.or
              local.get 6
              i64.const 8
              i64.shr_u
              i64.const 4278190080
              i64.and
              local.get 6
              i64.const 24
              i64.shr_u
              i64.const 16711680
              i64.and
              i64.or
              local.get 6
              i64.const 40
              i64.shr_u
              i64.const 65280
              i64.and
              local.get 6
              i64.const 56
              i64.shr_u
              i64.or
              i64.or
              i64.or
              local.set 11
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
              local.set 13
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
              local.set 14
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
              local.set 8
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
              local.set 2
              local.get 15
              i64.const 56
              i64.shl
              local.get 15
              i64.const 65280
              i64.and
              i64.const 40
              i64.shl
              i64.or
              local.get 15
              i64.const 16711680
              i64.and
              i64.const 24
              i64.shl
              local.get 15
              i64.const 4278190080
              i64.and
              i64.const 8
              i64.shl
              i64.or
              i64.or
              local.get 15
              i64.const 8
              i64.shr_u
              i64.const 4278190080
              i64.and
              local.get 15
              i64.const 24
              i64.shr_u
              i64.const 16711680
              i64.and
              i64.or
              local.get 15
              i64.const 40
              i64.shr_u
              i64.const 65280
              i64.and
              local.get 15
              i64.const 56
              i64.shr_u
              i64.or
              i64.or
              i64.or
              local.set 3
              local.get 9
              i64.const 56
              i64.shl
              local.get 9
              i64.const 65280
              i64.and
              i64.const 40
              i64.shl
              i64.or
              local.get 9
              i64.const 16711680
              i64.and
              i64.const 24
              i64.shl
              local.get 9
              i64.const 4278190080
              i64.and
              i64.const 8
              i64.shl
              i64.or
              i64.or
              local.get 9
              i64.const 8
              i64.shr_u
              i64.const 4278190080
              i64.and
              local.get 9
              i64.const 24
              i64.shr_u
              i64.const 16711680
              i64.and
              i64.or
              local.get 9
              i64.const 40
              i64.shr_u
              i64.const 65280
              i64.and
              local.get 9
              i64.const 56
              i64.shr_u
              i64.or
              i64.or
              i64.or
              local.set 6
              i32.const -8
              local.set 16
              loop  ;; label = @6
                block  ;; label = @7
                  local.get 16
                  br_if 0 (;@7;)
                  i32.const 0
                  local.set 17
                  i32.const 0
                  local.set 10
                  block  ;; label = @8
                    loop  ;; label = @9
                      local.get 10
                      i32.const 32
                      i32.eq
                      br_if 1 (;@8;)
                      local.get 0
                      i32.const 192
                      i32.add
                      local.get 10
                      i32.add
                      local.set 16
                      local.get 10
                      i32.const 1
                      i32.add
                      local.tee 18
                      local.set 10
                      local.get 16
                      i32.load8_u
                      i32.eqz
                      br_if 0 (;@9;)
                    end
                    local.get 18
                    i32.const -1
                    i32.add
                    local.set 17
                  end
                  i32.const 0
                  local.set 19
                  i32.const 0
                  local.set 10
                  block  ;; label = @8
                    loop  ;; label = @9
                      local.get 10
                      i32.const 32
                      i32.eq
                      br_if 1 (;@8;)
                      local.get 0
                      i32.const 224
                      i32.add
                      local.get 10
                      i32.add
                      local.set 16
                      local.get 10
                      i32.const 1
                      i32.add
                      local.tee 18
                      local.set 10
                      local.get 16
                      i32.load8_u
                      i32.eqz
                      br_if 0 (;@9;)
                    end
                    local.get 18
                    i32.const -1
                    i32.add
                    local.set 19
                  end
                  i32.const 0
                  local.get 17
                  i32.sub
                  local.set 20
                  i32.const 31
                  local.get 19
                  i32.sub
                  local.set 21
                  i32.const 32
                  local.get 19
                  i32.sub
                  local.set 22
                  local.get 17
                  local.get 19
                  i32.sub
                  local.tee 16
                  i32.const 32
                  i32.add
                  local.set 10
                  local.get 19
                  local.get 17
                  i32.sub
                  local.tee 18
                  local.get 0
                  i32.const 256
                  i32.add
                  i32.add
                  i32.const -24
                  i32.add
                  local.set 23
                  local.get 0
                  i32.const 224
                  i32.add
                  local.get 18
                  i32.add
                  local.set 24
                  local.get 0
                  i32.const 192
                  i32.add
                  local.get 16
                  i32.const 31
                  i32.add
                  local.tee 25
                  i32.add
                  local.set 26
                  local.get 0
                  i32.const 224
                  i32.add
                  local.get 19
                  i32.add
                  local.set 27
                  i32.const 0
                  local.set 16
                  loop  ;; label = @8
                    local.get 16
                    local.set 28
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 10
                        local.tee 29
                        local.get 17
                        i32.sub
                        local.tee 10
                        local.get 22
                        i32.or
                        i32.const 8
                        i32.lt_u
                        br_if 0 (;@10;)
                        i32.const 0
                        local.set 30
                        local.get 22
                        local.get 10
                        i32.gt_u
                        br_if 1 (;@9;)
                        local.get 25
                        local.get 17
                        i32.sub
                        local.set 31
                        local.get 17
                        local.get 20
                        i32.add
                        local.set 32
                        local.get 24
                        local.get 17
                        i32.add
                        local.set 33
                        i32.const 0
                        local.set 30
                        loop  ;; label = @11
                          local.get 17
                          local.set 10
                          local.get 32
                          local.set 16
                          local.get 33
                          local.set 18
                          block  ;; label = @12
                            loop  ;; label = @13
                              local.get 29
                              local.get 10
                              i32.eq
                              br_if 1 (;@12;)
                              local.get 0
                              i32.const 192
                              i32.add
                              local.get 10
                              i32.add
                              i32.load8_u
                              local.set 34
                              block  ;; label = @14
                                block  ;; label = @15
                                  local.get 16
                                  i32.const 0
                                  i32.lt_s
                                  br_if 0 (;@15;)
                                  local.get 34
                                  i32.const 255
                                  i32.and
                                  local.tee 34
                                  local.get 18
                                  i32.load8_u
                                  local.tee 35
                                  i32.gt_u
                                  br_if 3 (;@12;)
                                  local.get 34
                                  local.get 35
                                  i32.ge_u
                                  br_if 1 (;@14;)
                                  br 6 (;@9;)
                                end
                                local.get 34
                                i32.const 255
                                i32.and
                                br_if 2 (;@12;)
                              end
                              local.get 10
                              i32.const 1
                              i32.add
                              local.set 10
                              local.get 16
                              i32.const 1
                              i32.add
                              local.set 16
                              local.get 18
                              i32.const 1
                              i32.add
                              local.set 18
                              br 0 (;@13;)
                            end
                          end
                          i32.const 0
                          local.set 34
                          local.get 31
                          local.set 18
                          local.get 26
                          local.set 10
                          local.get 21
                          local.set 16
                          loop  ;; label = @12
                            block  ;; label = @13
                              block  ;; label = @14
                                block  ;; label = @15
                                  local.get 18
                                  i32.const 0
                                  i32.lt_s
                                  br_if 0 (;@15;)
                                  block  ;; label = @16
                                    local.get 16
                                    i32.const 0
                                    i32.lt_s
                                    br_if 0 (;@16;)
                                    block  ;; label = @17
                                      block  ;; label = @18
                                        local.get 27
                                        local.get 16
                                        i32.add
                                        i32.load8_u
                                        local.tee 35
                                        local.get 34
                                        i32.const 255
                                        i32.and
                                        i32.add
                                        local.get 10
                                        i32.load8_u
                                        local.tee 36
                                        i32.le_u
                                        br_if 0 (;@18;)
                                        local.get 35
                                        i32.const -1
                                        i32.xor
                                        local.get 34
                                        i32.sub
                                        local.set 35
                                        i32.const 1
                                        local.set 34
                                        local.get 35
                                        local.get 36
                                        i32.add
                                        i32.const 1
                                        i32.add
                                        local.set 35
                                        br 1 (;@17;)
                                      end
                                      local.get 36
                                      local.get 35
                                      local.get 34
                                      i32.add
                                      i32.sub
                                      local.set 35
                                      i32.const 0
                                      local.set 34
                                    end
                                    local.get 10
                                    local.get 35
                                    i32.store8
                                    local.get 16
                                    i32.const -1
                                    i32.add
                                    local.set 16
                                    br 3 (;@13;)
                                  end
                                  local.get 34
                                  i32.const 255
                                  i32.and
                                  br_if 1 (;@14;)
                                end
                                local.get 30
                                i32.const 1
                                i32.add
                                local.set 30
                                br 3 (;@11;)
                              end
                              i32.const -1
                              local.set 16
                              local.get 10
                              local.get 10
                              i32.load8_u
                              local.tee 34
                              i32.const -1
                              i32.add
                              i32.store8
                              local.get 34
                              i32.eqz
                              local.set 34
                            end
                            local.get 18
                            i32.const -1
                            i32.add
                            local.set 18
                            local.get 10
                            i32.const -1
                            i32.add
                            local.set 10
                            br 0 (;@12;)
                          end
                        end
                      end
                      local.get 0
                      i64.const 0
                      i64.store offset=256
                      local.get 17
                      local.set 10
                      block  ;; label = @10
                        loop  ;; label = @11
                          block  ;; label = @12
                            local.get 29
                            local.get 10
                            i32.ne
                            br_if 0 (;@12;)
                            local.get 0
                            i64.const 0
                            i64.store offset=264
                            local.get 19
                            local.set 10
                            loop  ;; label = @13
                              block  ;; label = @14
                                local.get 10
                                i32.const 32
                                i32.ne
                                br_if 0 (;@14;)
                                i64.const 0
                                local.set 12
                                i32.const 0
                                local.set 10
                                i64.const 0
                                local.set 11
                                block  ;; label = @15
                                  loop  ;; label = @16
                                    block  ;; label = @17
                                      local.get 10
                                      i32.const 8
                                      i32.ne
                                      br_if 0 (;@17;)
                                      local.get 11
                                      i64.eqz
                                      i32.eqz
                                      br_if 2 (;@15;)
                                      i32.const 0
                                      local.set 30
                                      br 7 (;@10;)
                                    end
                                    local.get 11
                                    i64.const 8
                                    i64.shl
                                    local.get 0
                                    i32.const 264
                                    i32.add
                                    local.get 10
                                    i32.add
                                    i64.load8_u
                                    i64.or
                                    local.set 11
                                    local.get 12
                                    i64.const 8
                                    i64.shl
                                    local.get 0
                                    i32.const 256
                                    i32.add
                                    local.get 10
                                    i32.add
                                    i64.load8_u
                                    i64.or
                                    local.set 12
                                    local.get 10
                                    i32.const 1
                                    i32.add
                                    local.set 10
                                    br 0 (;@16;)
                                  end
                                end
                                local.get 12
                                local.get 12
                                local.get 11
                                i64.div_u
                                local.tee 13
                                i64.const 255
                                i64.and
                                local.get 11
                                i64.mul
                                i64.sub
                                local.set 12
                                local.get 13
                                i32.wrap_i64
                                local.set 30
                                br 4 (;@10;)
                              end
                              local.get 0
                              i32.const 264
                              i32.add
                              local.get 10
                              i32.add
                              i32.const -24
                              i32.add
                              local.get 0
                              i32.const 224
                              i32.add
                              local.get 10
                              i32.add
                              i32.load8_u
                              i32.store8
                              local.get 10
                              i32.const 1
                              i32.add
                              local.set 10
                              br 0 (;@13;)
                            end
                          end
                          local.get 23
                          local.get 10
                          i32.add
                          local.get 0
                          i32.const 192
                          i32.add
                          local.get 10
                          i32.add
                          i32.load8_u
                          i32.store8
                          local.get 10
                          i32.const 1
                          i32.add
                          local.set 10
                          br 0 (;@11;)
                        end
                      end
                      local.get 0
                      local.get 12
                      i64.const 56
                      i64.shl
                      local.get 12
                      i64.const 65280
                      i64.and
                      i64.const 40
                      i64.shl
                      i64.or
                      local.get 12
                      i64.const 16711680
                      i64.and
                      i64.const 24
                      i64.shl
                      local.get 12
                      i64.const 4278190080
                      i64.and
                      i64.const 8
                      i64.shl
                      i64.or
                      i64.or
                      local.get 12
                      i64.const 8
                      i64.shr_u
                      i64.const 4278190080
                      i64.and
                      local.get 12
                      i64.const 24
                      i64.shr_u
                      i64.const 16711680
                      i64.and
                      i64.or
                      local.get 12
                      i64.const 40
                      i64.shr_u
                      i64.const 65280
                      i64.and
                      local.get 12
                      i64.const 56
                      i64.shr_u
                      i64.or
                      i64.or
                      i64.or
                      i64.store offset=256
                      local.get 17
                      local.set 10
                      loop  ;; label = @10
                        local.get 29
                        local.get 10
                        i32.eq
                        br_if 1 (;@9;)
                        local.get 0
                        i32.const 192
                        i32.add
                        local.get 10
                        i32.add
                        local.get 23
                        local.get 10
                        i32.add
                        i32.load8_u
                        i32.store8
                        local.get 10
                        i32.const 1
                        i32.add
                        local.set 10
                        br 0 (;@10;)
                      end
                    end
                    local.get 0
                    i32.const 160
                    i32.add
                    local.get 28
                    i32.add
                    local.get 30
                    i32.store8
                    block  ;; label = @9
                      local.get 30
                      i32.const 255
                      i32.and
                      i32.eqz
                      br_if 0 (;@9;)
                      local.get 17
                      i32.const 32
                      local.get 17
                      i32.const 32
                      i32.gt_u
                      select
                      local.set 10
                      loop  ;; label = @10
                        block  ;; label = @11
                          local.get 10
                          local.get 17
                          i32.ne
                          br_if 0 (;@11;)
                          local.get 10
                          local.set 17
                          br 2 (;@9;)
                        end
                        local.get 0
                        i32.const 192
                        i32.add
                        local.get 17
                        i32.add
                        i32.load8_u
                        br_if 1 (;@9;)
                        local.get 17
                        i32.const 1
                        i32.add
                        local.set 17
                        br 0 (;@10;)
                      end
                    end
                    local.get 29
                    i32.const 1
                    i32.add
                    local.set 10
                    local.get 28
                    i32.const 1
                    i32.add
                    local.set 16
                    local.get 23
                    i32.const -1
                    i32.add
                    local.set 23
                    local.get 25
                    i32.const 1
                    i32.add
                    local.set 25
                    local.get 26
                    i32.const 1
                    i32.add
                    local.set 26
                    local.get 20
                    i32.const -1
                    i32.add
                    local.set 20
                    local.get 24
                    i32.const -1
                    i32.add
                    local.set 24
                    local.get 29
                    i32.const 32
                    i32.lt_u
                    br_if 0 (;@8;)
                  end
                  local.get 0
                  i32.const 128
                  i32.add
                  local.get 28
                  i32.sub
                  i32.const 31
                  i32.add
                  local.set 16
                  i32.const 0
                  local.set 10
                  block  ;; label = @8
                    loop  ;; label = @9
                      local.get 10
                      local.get 28
                      i32.gt_u
                      br_if 1 (;@8;)
                      local.get 16
                      local.get 0
                      i32.const 160
                      i32.add
                      local.get 10
                      i32.add
                      i32.load8_u
                      i32.store8
                      local.get 16
                      i32.const 1
                      i32.add
                      local.set 16
                      local.get 10
                      i32.const 1
                      i32.add
                      local.set 10
                      br 0 (;@9;)
                    end
                  end
                  block  ;; label = @8
                    local.get 7
                    i64.const 0
                    i64.lt_s
                    local.get 1
                    i64.const 0
                    i64.lt_s
                    i32.eq
                    br_if 0 (;@8;)
                    i32.const 31
                    local.set 10
                    i32.const 1
                    local.set 16
                    loop  ;; label = @9
                      local.get 10
                      i32.const -1
                      i32.eq
                      br_if 1 (;@8;)
                      local.get 0
                      i32.const 128
                      i32.add
                      local.get 10
                      i32.add
                      local.tee 18
                      i32.const 0
                      local.get 18
                      i32.load8_u
                      local.tee 18
                      i32.sub
                      local.get 18
                      i32.const -1
                      i32.xor
                      local.get 16
                      i32.const 1
                      i32.and
                      select
                      i32.store8
                      local.get 10
                      i32.const -1
                      i32.add
                      local.set 10
                      local.get 16
                      local.get 18
                      i32.eqz
                      i32.and
                      local.set 16
                      br 0 (;@9;)
                    end
                  end
                  local.get 0
                  i64.load offset=128
                  local.tee 12
                  i64.const 56
                  i64.shl
                  local.get 12
                  i64.const 65280
                  i64.and
                  i64.const 40
                  i64.shl
                  i64.or
                  local.get 12
                  i64.const 16711680
                  i64.and
                  i64.const 24
                  i64.shl
                  local.get 12
                  i64.const 4278190080
                  i64.and
                  i64.const 8
                  i64.shl
                  i64.or
                  i64.or
                  local.get 12
                  i64.const 8
                  i64.shr_u
                  i64.const 4278190080
                  i64.and
                  local.get 12
                  i64.const 24
                  i64.shr_u
                  i64.const 16711680
                  i64.and
                  i64.or
                  local.get 12
                  i64.const 40
                  i64.shr_u
                  i64.const 65280
                  i64.and
                  local.get 12
                  i64.const 56
                  i64.shr_u
                  i64.or
                  i64.or
                  i64.or
                  local.set 12
                  local.get 0
                  i64.load offset=136
                  local.tee 11
                  i64.const 56
                  i64.shl
                  local.get 11
                  i64.const 65280
                  i64.and
                  i64.const 40
                  i64.shl
                  i64.or
                  local.get 11
                  i64.const 16711680
                  i64.and
                  i64.const 24
                  i64.shl
                  local.get 11
                  i64.const 4278190080
                  i64.and
                  i64.const 8
                  i64.shl
                  i64.or
                  i64.or
                  local.get 11
                  i64.const 8
                  i64.shr_u
                  i64.const 4278190080
                  i64.and
                  local.get 11
                  i64.const 24
                  i64.shr_u
                  i64.const 16711680
                  i64.and
                  i64.or
                  local.get 11
                  i64.const 40
                  i64.shr_u
                  i64.const 65280
                  i64.and
                  local.get 11
                  i64.const 56
                  i64.shr_u
                  i64.or
                  i64.or
                  i64.or
                  local.set 13
                  local.get 0
                  i64.load offset=144
                  local.tee 11
                  i64.const 56
                  i64.shl
                  local.get 11
                  i64.const 65280
                  i64.and
                  i64.const 40
                  i64.shl
                  i64.or
                  local.get 11
                  i64.const 16711680
                  i64.and
                  i64.const 24
                  i64.shl
                  local.get 11
                  i64.const 4278190080
                  i64.and
                  i64.const 8
                  i64.shl
                  i64.or
                  i64.or
                  local.get 11
                  i64.const 8
                  i64.shr_u
                  i64.const 4278190080
                  i64.and
                  local.get 11
                  i64.const 24
                  i64.shr_u
                  i64.const 16711680
                  i64.and
                  i64.or
                  local.get 11
                  i64.const 40
                  i64.shr_u
                  i64.const 65280
                  i64.and
                  local.get 11
                  i64.const 56
                  i64.shr_u
                  i64.or
                  i64.or
                  i64.or
                  local.set 14
                  local.get 0
                  i64.load offset=152
                  local.tee 11
                  i64.const 56
                  i64.shl
                  local.get 11
                  i64.const 65280
                  i64.and
                  i64.const 40
                  i64.shl
                  i64.or
                  local.get 11
                  i64.const 16711680
                  i64.and
                  i64.const 24
                  i64.shl
                  local.get 11
                  i64.const 4278190080
                  i64.and
                  i64.const 8
                  i64.shl
                  i64.or
                  i64.or
                  local.get 11
                  i64.const 8
                  i64.shr_u
                  i64.const 4278190080
                  i64.and
                  local.get 11
                  i64.const 24
                  i64.shr_u
                  i64.const 16711680
                  i64.and
                  i64.or
                  local.get 11
                  i64.const 40
                  i64.shr_u
                  i64.const 65280
                  i64.and
                  local.get 11
                  i64.const 56
                  i64.shr_u
                  i64.or
                  i64.or
                  i64.or
                  local.set 11
                  br 6 (;@1;)
                end
                local.get 0
                local.get 6
                i64.store offset=264
                local.get 0
                i32.const 192
                i32.add
                local.get 16
                i32.add
                local.tee 18
                i32.const 8
                i32.add
                local.get 0
                i32.const 264
                i32.add
                local.get 16
                i32.add
                i32.const 8
                i32.add
                local.tee 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 3
                i64.store offset=264
                local.get 0
                i32.const 224
                i32.add
                local.get 16
                i32.add
                local.tee 34
                i32.const 8
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 2
                i64.store offset=264
                local.get 18
                i32.const 16
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 8
                i64.store offset=264
                local.get 34
                i32.const 16
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 14
                i64.store offset=264
                local.get 18
                i32.const 24
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 13
                i64.store offset=264
                local.get 34
                i32.const 24
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 11
                i64.store offset=264
                local.get 18
                i32.const 32
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 0
                local.get 12
                i64.store offset=264
                local.get 34
                i32.const 32
                i32.add
                local.get 10
                i32.load8_u
                i32.store8
                local.get 16
                i32.const 1
                i32.add
                local.set 16
                br 0 (;@6;)
              end
            end
            local.get 8
            i64.eqz
            i32.eqz
            br_if 2 (;@2;)
          end
          i64.const 0
          local.set 12
        end
        i64.const 0
        local.set 13
        i64.const 0
        local.set 14
        i64.const 0
        local.set 11
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 1
        local.get 7
        i64.and
        i64.const -1
        i64.gt_s
        br_if 0 (;@2;)
        local.get 8
        i64.const -1
        i64.ne
        br_if 0 (;@2;)
        local.get 0
        local.get 7
        i64.store offset=248
        local.get 0
        local.get 4
        i64.store offset=240
        local.get 0
        local.get 5
        i64.store offset=232
        local.get 0
        local.get 6
        i64.store offset=224
        local.get 0
        i32.const 192
        i32.add
        local.get 0
        i32.const 224
        i32.add
        call $_ZN30fluentbase_rwasm_code_snippets6common15convert_sign_le17hd7eaa31eb5c86d58E
        local.get 0
        i64.load offset=192
        local.set 11
        local.get 0
        i64.load offset=200
        local.set 14
        local.get 0
        i64.load offset=208
        local.set 13
        local.get 0
        i64.load offset=216
        local.set 12
        br 1 (;@1;)
      end
      local.get 7
      local.set 12
      local.get 4
      local.set 13
      local.get 5
      local.set 14
      local.get 6
      local.set 11
    end
    i32.const 0
    i32.const 0
    i64.load offset=500
    i64.const 32
    i64.shl
    i64.const 137438953472
    i64.add
    i64.const 32
    i64.shr_s
    local.tee 8
    i64.store offset=500
    i32.const 524
    local.get 8
    i32.wrap_i64
    local.tee 10
    i32.sub
    local.get 11
    i64.const 56
    i64.shl
    local.get 11
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 11
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 11
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 11
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 11
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 11
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 11
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 516
    local.get 10
    i32.sub
    local.get 14
    i64.const 56
    i64.shl
    local.get 14
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 14
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 14
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 14
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 14
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 14
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 14
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 508
    local.get 10
    i32.sub
    local.get 13
    i64.const 56
    i64.shl
    local.get 13
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 13
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 13
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 13
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 13
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 13
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 13
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 500
    local.get 10
    i32.sub
    local.get 12
    i64.const 56
    i64.shl
    local.get 12
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 12
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 12
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 12
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 12
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 12
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 12
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    local.get 0
    i32.const 272
    i32.add
    global.set $__stack_pointer)
  (func $_ZN30fluentbase_rwasm_code_snippets9common_sp8u256_pop17h1d18dbc2fbdc0d73E (type 1) (param i32)
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
  (func $_ZN30fluentbase_rwasm_code_snippets6common19u256_be_to_tuple_le17h7a03d9a932c8cd0eE (type 2) (param i32 i32)
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
  (func $_ZN30fluentbase_rwasm_code_snippets6common15convert_sign_le17hd7eaa31eb5c86d58E (type 2) (param i32 i32)
    (local i64 i64 i64 i64 i64 i64 i64)
    local.get 1
    i64.load offset=16
    local.set 2
    local.get 1
    i64.load offset=8
    local.set 3
    local.get 1
    i64.load
    local.set 4
    block  ;; label = @1
      block  ;; label = @2
        local.get 1
        i64.load offset=24
        local.tee 5
        i64.const 0
        i64.lt_s
        br_if 0 (;@2;)
        local.get 2
        i64.const -1
        i64.xor
        local.set 6
        local.get 5
        i64.const -1
        i64.xor
        local.set 7
        block  ;; label = @3
          local.get 4
          i64.eqz
          br_if 0 (;@3;)
          i64.const 0
          local.get 4
          i64.sub
          local.set 8
          local.get 3
          i64.const -1
          i64.xor
          local.set 4
          br 2 (;@1;)
        end
        block  ;; label = @3
          local.get 3
          i64.eqz
          br_if 0 (;@3;)
          i64.const 0
          local.set 8
          i64.const 0
          local.get 3
          i64.sub
          local.set 4
          br 2 (;@1;)
        end
        block  ;; label = @3
          block  ;; label = @4
            local.get 2
            i64.eqz
            br_if 0 (;@4;)
            i64.const 0
            local.set 4
            i64.const 0
            local.get 2
            i64.sub
            local.set 6
            br 1 (;@3;)
          end
          i64.const 0
          local.set 6
          i64.const 0
          local.get 5
          i64.sub
          local.set 7
          i64.const 0
          local.set 4
        end
        i64.const 0
        local.set 8
        br 1 (;@1;)
      end
      block  ;; label = @2
        block  ;; label = @3
          local.get 4
          i64.eqz
          br_if 0 (;@3;)
          i64.const 0
          local.get 4
          i64.sub
          local.set 8
          br 1 (;@2;)
        end
        block  ;; label = @3
          local.get 3
          i64.eqz
          br_if 0 (;@3;)
          local.get 3
          i64.const -1
          i64.add
          local.set 3
          i64.const 0
          local.set 8
          br 1 (;@2;)
        end
        block  ;; label = @3
          local.get 2
          i64.eqz
          br_if 0 (;@3;)
          i64.const -1
          local.set 3
          local.get 2
          i64.const -1
          i64.add
          local.set 2
          i64.const 0
          local.set 8
          br 1 (;@2;)
        end
        i64.const -1
        local.set 2
        local.get 5
        i64.const -1
        i64.add
        local.set 5
        i64.const 0
        local.set 8
        i64.const -1
        local.set 3
      end
      local.get 3
      i64.const -1
      i64.xor
      local.set 4
      local.get 2
      i64.const -1
      i64.xor
      local.set 6
      local.get 5
      i64.const -1
      i64.xor
      local.set 7
    end
    local.get 0
    local.get 7
    i64.store offset=24
    local.get 0
    local.get 6
    i64.store offset=16
    local.get 0
    local.get 4
    i64.store offset=8
    local.get 0
    local.get 8
    i64.store)
  (memory (;0;) 16)
  (global $__stack_pointer (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_sdiv" (func $arithmetic_sdiv))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
