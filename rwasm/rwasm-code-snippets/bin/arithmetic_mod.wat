(module
  (type (;0;) (func))
  (type (;1;) (func (param i32)))
  (type (;2;) (func (param i32 i32)))
  (func (;0;) (type 0)
    (local i32 i64 i64 i64 i64 i64 i64 i64 i64 i64 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    global.get 0
    i32.const 240
    i32.sub
    local.tee 0
    global.set 0
    local.get 0
    call 1
    local.get 0
    i32.const 32
    i32.add
    call 1
    local.get 0
    i32.const 64
    i32.add
    local.get 0
    call 2
    local.get 0
    i32.const 96
    i32.add
    local.get 0
    i32.const 32
    i32.add
    call 2
    local.get 0
    i32.const 152
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 144
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i32.const 136
    i32.add
    i64.const 0
    i64.store
    local.get 0
    i64.const 0
    i64.store offset=128
    block  ;; label = @1
      block  ;; label = @2
        block  ;; label = @3
          local.get 0
          i64.load offset=120
          local.tee 1
          local.get 0
          i64.load offset=88
          local.tee 2
          i64.eq
          br_if 0 (;@3;)
          local.get 0
          i64.load offset=80
          local.set 3
          br 1 (;@2;)
        end
        local.get 0
        i64.load offset=112
        local.tee 4
        local.get 0
        i64.load offset=80
        local.tee 3
        i64.ne
        br_if 0 (;@2;)
        block  ;; label = @3
          local.get 0
          i64.load offset=104
          local.get 0
          i64.load offset=72
          i64.eq
          br_if 0 (;@3;)
          local.get 4
          local.set 3
          br 1 (;@2;)
        end
        i64.const 0
        local.set 5
        local.get 4
        local.set 3
        i64.const 0
        local.set 4
        i64.const 0
        local.set 6
        i64.const 0
        local.set 7
        local.get 0
        i64.load offset=96
        local.get 0
        i64.load offset=64
        i64.eq
        br_if 1 (;@1;)
      end
      i64.const 0
      local.set 5
      local.get 0
      i64.load offset=64
      local.set 8
      local.get 0
      i64.load offset=72
      local.set 9
      block  ;; label = @2
        local.get 2
        i64.const 0
        i64.ne
        br_if 0 (;@2;)
        local.get 3
        i64.const 0
        i64.ne
        br_if 0 (;@2;)
        local.get 9
        i64.const 0
        i64.ne
        br_if 0 (;@2;)
        i64.const 0
        local.set 4
        i64.const 0
        local.set 6
        i64.const 0
        local.set 7
        local.get 8
        i64.const 1
        i64.eq
        br_if 1 (;@1;)
      end
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
      local.set 5
      local.get 0
      i64.load offset=96
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
      local.set 4
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
      local.get 0
      i64.load offset=104
      local.tee 7
      i64.const 56
      i64.shl
      local.get 7
      i64.const 65280
      i64.and
      i64.const 40
      i64.shl
      i64.or
      local.get 7
      i64.const 16711680
      i64.and
      i64.const 24
      i64.shl
      local.get 7
      i64.const 4278190080
      i64.and
      i64.const 8
      i64.shl
      i64.or
      i64.or
      local.get 7
      i64.const 8
      i64.shr_u
      i64.const 4278190080
      i64.and
      local.get 7
      i64.const 24
      i64.shr_u
      i64.const 16711680
      i64.and
      i64.or
      local.get 7
      i64.const 40
      i64.shr_u
      i64.const 65280
      i64.and
      local.get 7
      i64.const 56
      i64.shr_u
      i64.or
      i64.or
      i64.or
      local.set 7
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
      local.set 3
      local.get 0
      i64.load offset=112
      local.tee 9
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
      local.set 9
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
      local.set 2
      local.get 1
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
      local.set 8
      i32.const -8
      local.set 10
      block  ;; label = @2
        loop  ;; label = @3
          local.get 10
          i32.eqz
          br_if 1 (;@2;)
          local.get 0
          local.get 8
          i64.store offset=232
          local.get 0
          i32.const 160
          i32.add
          local.get 10
          i32.add
          local.tee 11
          i32.const 8
          i32.add
          local.get 0
          i32.const 232
          i32.add
          local.get 10
          i32.add
          i32.const 8
          i32.add
          local.tee 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 2
          i64.store offset=232
          local.get 0
          i32.const 192
          i32.add
          local.get 10
          i32.add
          local.tee 13
          i32.const 8
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 9
          i64.store offset=232
          local.get 11
          i32.const 16
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 3
          i64.store offset=232
          local.get 13
          i32.const 16
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 7
          i64.store offset=232
          local.get 11
          i32.const 24
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 6
          i64.store offset=232
          local.get 13
          i32.const 24
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 4
          i64.store offset=232
          local.get 11
          i32.const 32
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 0
          local.get 5
          i64.store offset=232
          local.get 13
          i32.const 32
          i32.add
          local.get 12
          i32.load8_u
          i32.store8
          local.get 10
          i32.const 1
          i32.add
          local.set 10
          br 0 (;@3;)
        end
      end
      i32.const 0
      local.set 14
      i32.const 0
      local.set 12
      block  ;; label = @2
        loop  ;; label = @3
          local.get 12
          i32.const 32
          i32.eq
          br_if 1 (;@2;)
          local.get 0
          i32.const 160
          i32.add
          local.get 12
          i32.add
          local.set 10
          local.get 12
          i32.const 1
          i32.add
          local.tee 11
          local.set 12
          local.get 10
          i32.load8_u
          i32.eqz
          br_if 0 (;@3;)
        end
        local.get 11
        i32.const -1
        i32.add
        local.set 14
      end
      i32.const 0
      local.set 15
      i32.const 0
      local.set 12
      block  ;; label = @2
        loop  ;; label = @3
          local.get 12
          i32.const 32
          i32.eq
          br_if 1 (;@2;)
          local.get 0
          i32.const 192
          i32.add
          local.get 12
          i32.add
          local.set 10
          local.get 12
          i32.const 1
          i32.add
          local.tee 11
          local.set 12
          local.get 10
          i32.load8_u
          i32.eqz
          br_if 0 (;@3;)
        end
        local.get 11
        i32.const -1
        i32.add
        local.set 15
      end
      i32.const 0
      local.get 14
      i32.sub
      local.set 16
      i32.const 31
      local.get 15
      i32.sub
      local.set 17
      i32.const 32
      local.get 15
      i32.sub
      local.set 18
      local.get 14
      local.get 15
      i32.sub
      local.tee 10
      i32.const 32
      i32.add
      local.set 12
      local.get 15
      local.get 14
      i32.sub
      local.tee 11
      local.get 0
      i32.const 224
      i32.add
      i32.add
      i32.const -24
      i32.add
      local.set 19
      local.get 0
      i32.const 192
      i32.add
      local.get 11
      i32.add
      local.set 20
      local.get 0
      i32.const 160
      i32.add
      local.get 10
      i32.const 31
      i32.add
      local.tee 21
      i32.add
      local.set 22
      local.get 0
      i32.const 192
      i32.add
      local.get 15
      i32.add
      local.set 23
      loop  ;; label = @2
        block  ;; label = @3
          block  ;; label = @4
            block  ;; label = @5
              local.get 12
              local.tee 24
              local.get 14
              i32.sub
              local.tee 12
              local.get 18
              i32.or
              i32.const 8
              i32.lt_u
              br_if 0 (;@5;)
              local.get 18
              local.get 12
              i32.gt_u
              br_if 2 (;@3;)
              local.get 21
              local.get 14
              i32.sub
              local.set 25
              local.get 14
              local.get 16
              i32.add
              local.set 26
              local.get 20
              local.get 14
              i32.add
              local.set 27
              i32.const 0
              local.set 28
              loop  ;; label = @6
                local.get 14
                local.set 12
                local.get 26
                local.set 10
                local.get 27
                local.set 11
                block  ;; label = @7
                  loop  ;; label = @8
                    local.get 24
                    local.get 12
                    i32.eq
                    br_if 1 (;@7;)
                    local.get 0
                    i32.const 160
                    i32.add
                    local.get 12
                    i32.add
                    i32.load8_u
                    local.set 13
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 10
                        i32.const 0
                        i32.lt_s
                        br_if 0 (;@10;)
                        local.get 13
                        i32.const 255
                        i32.and
                        local.tee 13
                        local.get 11
                        i32.load8_u
                        local.tee 29
                        i32.gt_u
                        br_if 3 (;@7;)
                        local.get 13
                        local.get 29
                        i32.ge_u
                        br_if 1 (;@9;)
                        br 6 (;@4;)
                      end
                      local.get 13
                      i32.const 255
                      i32.and
                      br_if 2 (;@7;)
                    end
                    local.get 12
                    i32.const 1
                    i32.add
                    local.set 12
                    local.get 10
                    i32.const 1
                    i32.add
                    local.set 10
                    local.get 11
                    i32.const 1
                    i32.add
                    local.set 11
                    br 0 (;@8;)
                  end
                end
                i32.const 0
                local.set 13
                local.get 25
                local.set 11
                local.get 22
                local.set 12
                local.get 17
                local.set 10
                loop  ;; label = @7
                  block  ;; label = @8
                    block  ;; label = @9
                      block  ;; label = @10
                        local.get 11
                        i32.const 0
                        i32.lt_s
                        br_if 0 (;@10;)
                        block  ;; label = @11
                          local.get 10
                          i32.const 0
                          i32.lt_s
                          br_if 0 (;@11;)
                          block  ;; label = @12
                            block  ;; label = @13
                              local.get 23
                              local.get 10
                              i32.add
                              i32.load8_u
                              local.tee 29
                              local.get 13
                              i32.const 255
                              i32.and
                              i32.add
                              local.get 12
                              i32.load8_u
                              local.tee 30
                              i32.le_u
                              br_if 0 (;@13;)
                              local.get 29
                              i32.const -1
                              i32.xor
                              local.get 13
                              i32.sub
                              local.set 29
                              i32.const 1
                              local.set 13
                              local.get 29
                              local.get 30
                              i32.add
                              i32.const 1
                              i32.add
                              local.set 29
                              br 1 (;@12;)
                            end
                            local.get 30
                            local.get 29
                            local.get 13
                            i32.add
                            i32.sub
                            local.set 29
                            i32.const 0
                            local.set 13
                          end
                          local.get 12
                          local.get 29
                          i32.store8
                          local.get 10
                          i32.const -1
                          i32.add
                          local.set 10
                          br 3 (;@8;)
                        end
                        local.get 13
                        i32.const 255
                        i32.and
                        br_if 1 (;@9;)
                      end
                      local.get 28
                      i32.const 1
                      i32.add
                      local.set 28
                      br 3 (;@6;)
                    end
                    i32.const -1
                    local.set 10
                    local.get 12
                    local.get 12
                    i32.load8_u
                    local.tee 13
                    i32.const -1
                    i32.add
                    i32.store8
                    local.get 13
                    i32.eqz
                    local.set 13
                  end
                  local.get 11
                  i32.const -1
                  i32.add
                  local.set 11
                  local.get 12
                  i32.const -1
                  i32.add
                  local.set 12
                  br 0 (;@7;)
                end
              end
            end
            local.get 0
            i64.const 0
            i64.store offset=224
            local.get 14
            local.set 12
            block  ;; label = @5
              loop  ;; label = @6
                block  ;; label = @7
                  local.get 24
                  local.get 12
                  i32.ne
                  br_if 0 (;@7;)
                  local.get 0
                  i64.const 0
                  i64.store offset=232
                  local.get 15
                  local.set 12
                  loop  ;; label = @8
                    block  ;; label = @9
                      local.get 12
                      i32.const 32
                      i32.ne
                      br_if 0 (;@9;)
                      i64.const 0
                      local.set 5
                      i32.const 0
                      local.set 12
                      i64.const 0
                      local.set 4
                      block  ;; label = @10
                        loop  ;; label = @11
                          block  ;; label = @12
                            local.get 12
                            i32.const 8
                            i32.ne
                            br_if 0 (;@12;)
                            local.get 4
                            i64.eqz
                            i32.eqz
                            br_if 2 (;@10;)
                            i32.const 0
                            local.set 28
                            br 7 (;@5;)
                          end
                          local.get 4
                          i64.const 8
                          i64.shl
                          local.get 0
                          i32.const 232
                          i32.add
                          local.get 12
                          i32.add
                          i64.load8_u
                          i64.or
                          local.set 4
                          local.get 5
                          i64.const 8
                          i64.shl
                          local.get 0
                          i32.const 224
                          i32.add
                          local.get 12
                          i32.add
                          i64.load8_u
                          i64.or
                          local.set 5
                          local.get 12
                          i32.const 1
                          i32.add
                          local.set 12
                          br 0 (;@11;)
                        end
                      end
                      local.get 5
                      local.get 5
                      local.get 4
                      i64.div_u
                      local.tee 6
                      i64.const 255
                      i64.and
                      local.get 4
                      i64.mul
                      i64.sub
                      local.set 5
                      local.get 6
                      i32.wrap_i64
                      local.set 28
                      br 4 (;@5;)
                    end
                    local.get 0
                    i32.const 232
                    i32.add
                    local.get 12
                    i32.add
                    i32.const -24
                    i32.add
                    local.get 0
                    i32.const 192
                    i32.add
                    local.get 12
                    i32.add
                    i32.load8_u
                    i32.store8
                    local.get 12
                    i32.const 1
                    i32.add
                    local.set 12
                    br 0 (;@8;)
                  end
                end
                local.get 19
                local.get 12
                i32.add
                local.get 0
                i32.const 160
                i32.add
                local.get 12
                i32.add
                i32.load8_u
                i32.store8
                local.get 12
                i32.const 1
                i32.add
                local.set 12
                br 0 (;@6;)
              end
            end
            local.get 0
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
            i64.store offset=224
            local.get 14
            local.set 12
            loop  ;; label = @5
              local.get 24
              local.get 12
              i32.eq
              br_if 1 (;@4;)
              local.get 0
              i32.const 160
              i32.add
              local.get 12
              i32.add
              local.get 19
              local.get 12
              i32.add
              i32.load8_u
              i32.store8
              local.get 12
              i32.const 1
              i32.add
              local.set 12
              br 0 (;@5;)
            end
          end
          local.get 28
          i32.const 255
          i32.and
          i32.eqz
          br_if 0 (;@3;)
          local.get 14
          i32.const 32
          local.get 14
          i32.const 32
          i32.gt_u
          select
          local.set 12
          loop  ;; label = @4
            block  ;; label = @5
              local.get 12
              local.get 14
              i32.ne
              br_if 0 (;@5;)
              local.get 12
              local.set 14
              br 2 (;@3;)
            end
            local.get 0
            i32.const 160
            i32.add
            local.get 14
            i32.add
            i32.load8_u
            br_if 1 (;@3;)
            local.get 14
            i32.const 1
            i32.add
            local.set 14
            br 0 (;@4;)
          end
        end
        local.get 19
        i32.const -1
        i32.add
        local.set 19
        local.get 21
        i32.const 1
        i32.add
        local.set 21
        local.get 22
        i32.const 1
        i32.add
        local.set 22
        local.get 16
        i32.const -1
        i32.add
        local.set 16
        local.get 20
        i32.const -1
        i32.add
        local.set 20
        local.get 24
        i32.const 1
        i32.add
        local.set 12
        local.get 24
        i32.const 32
        i32.lt_u
        br_if 0 (;@2;)
      end
      local.get 0
      i32.const 160
      i32.add
      local.set 10
      i32.const 24
      local.set 12
      block  ;; label = @2
        loop  ;; label = @3
          local.get 12
          i32.const -8
          i32.eq
          br_if 1 (;@2;)
          local.get 0
          i32.const 128
          i32.add
          local.get 12
          i32.add
          local.get 10
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
          i64.store
          local.get 12
          i32.const -8
          i32.add
          local.set 12
          local.get 10
          i32.const 8
          i32.add
          local.set 10
          br 0 (;@3;)
        end
      end
      local.get 0
      i64.load offset=152
      local.set 5
      local.get 0
      i64.load offset=144
      local.set 4
      local.get 0
      i64.load offset=136
      local.set 6
      local.get 0
      i64.load offset=128
      local.set 7
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
    local.tee 3
    i64.store offset=500
    i32.const 524
    local.get 3
    i32.wrap_i64
    local.tee 12
    i32.sub
    local.get 7
    i64.const 56
    i64.shl
    local.get 7
    i64.const 65280
    i64.and
    i64.const 40
    i64.shl
    i64.or
    local.get 7
    i64.const 16711680
    i64.and
    i64.const 24
    i64.shl
    local.get 7
    i64.const 4278190080
    i64.and
    i64.const 8
    i64.shl
    i64.or
    i64.or
    local.get 7
    i64.const 8
    i64.shr_u
    i64.const 4278190080
    i64.and
    local.get 7
    i64.const 24
    i64.shr_u
    i64.const 16711680
    i64.and
    i64.or
    local.get 7
    i64.const 40
    i64.shr_u
    i64.const 65280
    i64.and
    local.get 7
    i64.const 56
    i64.shr_u
    i64.or
    i64.or
    i64.or
    i64.store align=1
    i32.const 516
    local.get 12
    i32.sub
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
    i64.store align=1
    i32.const 508
    local.get 12
    i32.sub
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
    i64.store align=1
    i32.const 500
    local.get 12
    i32.sub
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
    i64.store align=1
    local.get 0
    i32.const 240
    i32.add
    global.set 0)
  (func (;1;) (type 1) (param i32)
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
  (func (;2;) (type 2) (param i32 i32)
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
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_mod" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))