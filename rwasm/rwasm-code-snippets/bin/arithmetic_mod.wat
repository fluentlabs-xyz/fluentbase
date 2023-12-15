(module
  (type (;0;) (func (param i32 i64 i64 i64 i64 i64 i64 i64 i64)))
  (func (;0;) (type 0) (param i32 i64 i64 i64 i64 i64 i64 i64 i64)
    (local i64 i32 i64 i64 i64 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
    i64.const 0
    local.set 9
    global.get 0
    i32.const 112
    i32.sub
    local.tee 10
    i32.const 24
    i32.add
    i64.const 0
    i64.store
    local.get 10
    i32.const 16
    i32.add
    i64.const 0
    i64.store
    local.get 10
    i32.const 8
    i32.add
    i64.const 0
    i64.store
    local.get 10
    i64.const 0
    i64.store
    block  ;; label = @1
      block  ;; label = @2
        local.get 5
        local.get 1
        i64.ne
        br_if 0 (;@2;)
        local.get 6
        local.get 2
        i64.ne
        br_if 0 (;@2;)
        local.get 7
        local.get 3
        i64.ne
        br_if 0 (;@2;)
        local.get 8
        local.get 4
        i64.ne
        br_if 0 (;@2;)
        i64.const 0
        local.set 11
        i64.const 0
        local.set 12
        i64.const 0
        local.set 13
        br 1 (;@1;)
      end
      block  ;; label = @2
        local.get 1
        i64.const 1
        i64.ne
        br_if 0 (;@2;)
        i64.const 0
        local.set 11
        i64.const 0
        local.set 12
        i64.const 0
        local.set 13
        local.get 3
        local.get 2
        i64.or
        local.get 4
        i64.or
        i64.eqz
        br_if 1 (;@1;)
      end
      local.get 10
      i32.const 32
      i32.add
      i32.const 24
      i32.add
      i64.const 0
      i64.store
      local.get 10
      i32.const 32
      i32.add
      i32.const 16
      i32.add
      i64.const 0
      i64.store
      local.get 10
      i32.const 32
      i32.add
      i32.const 8
      i32.add
      i64.const 0
      i64.store
      local.get 10
      i64.const 0
      i64.store offset=32
      local.get 10
      i32.const 64
      i32.add
      i32.const 24
      i32.add
      i64.const 0
      i64.store
      local.get 10
      i32.const 64
      i32.add
      i32.const 16
      i32.add
      i64.const 0
      i64.store
      local.get 10
      i32.const 64
      i32.add
      i32.const 8
      i32.add
      i64.const 0
      i64.store
      local.get 10
      i64.const 0
      i64.store offset=64
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
      local.set 1
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
      local.set 5
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
      local.set 6
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
      local.set 7
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
      local.set 4
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
      local.set 8
      i32.const -8
      local.set 14
      loop  ;; label = @2
        block  ;; label = @3
          local.get 14
          br_if 0 (;@3;)
          i32.const 0
          local.set 15
          i32.const 0
          local.set 16
          block  ;; label = @4
            loop  ;; label = @5
              local.get 16
              i32.const 32
              i32.eq
              br_if 1 (;@4;)
              local.get 10
              i32.const 32
              i32.add
              local.get 16
              i32.add
              local.set 14
              local.get 16
              i32.const 1
              i32.add
              local.tee 17
              local.set 16
              local.get 14
              i32.load8_u
              i32.eqz
              br_if 0 (;@5;)
            end
            local.get 17
            i32.const -1
            i32.add
            local.set 15
          end
          i32.const 0
          local.set 18
          i32.const 0
          local.set 16
          block  ;; label = @4
            loop  ;; label = @5
              local.get 16
              i32.const 32
              i32.eq
              br_if 1 (;@4;)
              local.get 10
              i32.const 64
              i32.add
              local.get 16
              i32.add
              local.set 14
              local.get 16
              i32.const 1
              i32.add
              local.tee 17
              local.set 16
              local.get 14
              i32.load8_u
              i32.eqz
              br_if 0 (;@5;)
            end
            local.get 17
            i32.const -1
            i32.add
            local.set 18
          end
          i32.const 0
          local.get 15
          i32.sub
          local.set 19
          i32.const 31
          local.get 18
          i32.sub
          local.set 20
          i32.const 32
          local.get 18
          i32.sub
          local.set 21
          local.get 15
          local.get 18
          i32.sub
          local.tee 14
          i32.const 32
          i32.add
          local.set 16
          local.get 18
          local.get 15
          i32.sub
          local.tee 17
          local.get 10
          i32.const 96
          i32.add
          i32.add
          i32.const -24
          i32.add
          local.set 22
          local.get 10
          i32.const 64
          i32.add
          local.get 17
          i32.add
          local.set 23
          local.get 10
          i32.const 32
          i32.add
          local.get 14
          i32.const 31
          i32.add
          local.tee 24
          i32.add
          local.set 25
          local.get 10
          i32.const 64
          i32.add
          local.get 18
          i32.add
          local.set 26
          loop  ;; label = @4
            block  ;; label = @5
              block  ;; label = @6
                block  ;; label = @7
                  local.get 16
                  local.tee 27
                  local.get 15
                  i32.sub
                  local.tee 16
                  local.get 21
                  i32.or
                  i32.const 8
                  i32.lt_u
                  br_if 0 (;@7;)
                  local.get 21
                  local.get 16
                  i32.gt_u
                  br_if 2 (;@5;)
                  local.get 24
                  local.get 15
                  i32.sub
                  local.set 28
                  local.get 15
                  local.get 19
                  i32.add
                  local.set 29
                  local.get 23
                  local.get 15
                  i32.add
                  local.set 30
                  i32.const 0
                  local.set 31
                  loop  ;; label = @8
                    local.get 15
                    local.set 16
                    local.get 29
                    local.set 14
                    local.get 30
                    local.set 17
                    block  ;; label = @9
                      loop  ;; label = @10
                        local.get 27
                        local.get 16
                        i32.eq
                        br_if 1 (;@9;)
                        local.get 10
                        i32.const 32
                        i32.add
                        local.get 16
                        i32.add
                        i32.load8_u
                        local.set 32
                        block  ;; label = @11
                          block  ;; label = @12
                            local.get 14
                            i32.const 0
                            i32.lt_s
                            br_if 0 (;@12;)
                            local.get 32
                            i32.const 255
                            i32.and
                            local.tee 32
                            local.get 17
                            i32.load8_u
                            local.tee 33
                            i32.gt_u
                            br_if 3 (;@9;)
                            local.get 32
                            local.get 33
                            i32.ge_u
                            br_if 1 (;@11;)
                            br 6 (;@6;)
                          end
                          local.get 32
                          i32.const 255
                          i32.and
                          br_if 2 (;@9;)
                        end
                        local.get 16
                        i32.const 1
                        i32.add
                        local.set 16
                        local.get 14
                        i32.const 1
                        i32.add
                        local.set 14
                        local.get 17
                        i32.const 1
                        i32.add
                        local.set 17
                        br 0 (;@10;)
                      end
                    end
                    i32.const 0
                    local.set 32
                    local.get 28
                    local.set 17
                    local.get 25
                    local.set 16
                    local.get 20
                    local.set 14
                    loop  ;; label = @9
                      block  ;; label = @10
                        block  ;; label = @11
                          block  ;; label = @12
                            local.get 17
                            i32.const 0
                            i32.lt_s
                            br_if 0 (;@12;)
                            block  ;; label = @13
                              local.get 14
                              i32.const 0
                              i32.lt_s
                              br_if 0 (;@13;)
                              block  ;; label = @14
                                block  ;; label = @15
                                  local.get 26
                                  local.get 14
                                  i32.add
                                  i32.load8_u
                                  local.tee 33
                                  local.get 32
                                  i32.const 255
                                  i32.and
                                  i32.add
                                  local.get 16
                                  i32.load8_u
                                  local.tee 34
                                  i32.le_u
                                  br_if 0 (;@15;)
                                  local.get 33
                                  i32.const -1
                                  i32.xor
                                  local.get 32
                                  i32.sub
                                  local.set 33
                                  i32.const 1
                                  local.set 32
                                  local.get 33
                                  local.get 34
                                  i32.add
                                  i32.const 1
                                  i32.add
                                  local.set 33
                                  br 1 (;@14;)
                                end
                                local.get 34
                                local.get 33
                                local.get 32
                                i32.add
                                i32.sub
                                local.set 33
                                i32.const 0
                                local.set 32
                              end
                              local.get 16
                              local.get 33
                              i32.store8
                              local.get 14
                              i32.const -1
                              i32.add
                              local.set 14
                              br 3 (;@10;)
                            end
                            local.get 32
                            i32.const 255
                            i32.and
                            br_if 1 (;@11;)
                          end
                          local.get 31
                          i32.const 1
                          i32.add
                          local.set 31
                          br 3 (;@8;)
                        end
                        i32.const -1
                        local.set 14
                        local.get 16
                        local.get 16
                        i32.load8_u
                        local.tee 32
                        i32.const -1
                        i32.add
                        i32.store8
                        local.get 32
                        i32.eqz
                        local.set 32
                      end
                      local.get 17
                      i32.const -1
                      i32.add
                      local.set 17
                      local.get 16
                      i32.const -1
                      i32.add
                      local.set 16
                      br 0 (;@9;)
                    end
                  end
                end
                local.get 10
                i64.const 0
                i64.store offset=96
                local.get 15
                local.set 16
                block  ;; label = @7
                  loop  ;; label = @8
                    block  ;; label = @9
                      local.get 27
                      local.get 16
                      i32.ne
                      br_if 0 (;@9;)
                      local.get 10
                      i64.const 0
                      i64.store offset=104
                      local.get 18
                      local.set 16
                      loop  ;; label = @10
                        block  ;; label = @11
                          local.get 16
                          i32.const 32
                          i32.ne
                          br_if 0 (;@11;)
                          i64.const 0
                          local.set 1
                          i32.const 0
                          local.set 16
                          i64.const 0
                          local.set 5
                          block  ;; label = @12
                            loop  ;; label = @13
                              block  ;; label = @14
                                local.get 16
                                i32.const 8
                                i32.ne
                                br_if 0 (;@14;)
                                local.get 5
                                i64.eqz
                                i32.eqz
                                br_if 2 (;@12;)
                                i32.const 0
                                local.set 31
                                br 7 (;@7;)
                              end
                              local.get 5
                              i64.const 8
                              i64.shl
                              local.get 10
                              i32.const 104
                              i32.add
                              local.get 16
                              i32.add
                              i64.load8_u
                              i64.or
                              local.set 5
                              local.get 1
                              i64.const 8
                              i64.shl
                              local.get 10
                              i32.const 96
                              i32.add
                              local.get 16
                              i32.add
                              i64.load8_u
                              i64.or
                              local.set 1
                              local.get 16
                              i32.const 1
                              i32.add
                              local.set 16
                              br 0 (;@13;)
                            end
                          end
                          local.get 1
                          local.get 1
                          local.get 5
                          i64.div_u
                          local.tee 2
                          i64.const 255
                          i64.and
                          local.get 5
                          i64.mul
                          i64.sub
                          local.set 1
                          local.get 2
                          i32.wrap_i64
                          local.set 31
                          br 4 (;@7;)
                        end
                        local.get 10
                        i32.const 104
                        i32.add
                        local.get 16
                        i32.add
                        i32.const -24
                        i32.add
                        local.get 10
                        i32.const 64
                        i32.add
                        local.get 16
                        i32.add
                        i32.load8_u
                        i32.store8
                        local.get 16
                        i32.const 1
                        i32.add
                        local.set 16
                        br 0 (;@10;)
                      end
                    end
                    local.get 22
                    local.get 16
                    i32.add
                    local.get 10
                    i32.const 32
                    i32.add
                    local.get 16
                    i32.add
                    i32.load8_u
                    i32.store8
                    local.get 16
                    i32.const 1
                    i32.add
                    local.set 16
                    br 0 (;@8;)
                  end
                end
                local.get 10
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
                i64.store offset=96
                local.get 15
                local.set 16
                loop  ;; label = @7
                  local.get 27
                  local.get 16
                  i32.eq
                  br_if 1 (;@6;)
                  local.get 10
                  i32.const 32
                  i32.add
                  local.get 16
                  i32.add
                  local.get 22
                  local.get 16
                  i32.add
                  i32.load8_u
                  i32.store8
                  local.get 16
                  i32.const 1
                  i32.add
                  local.set 16
                  br 0 (;@7;)
                end
              end
              local.get 31
              i32.const 255
              i32.and
              i32.eqz
              br_if 0 (;@5;)
              local.get 15
              i32.const 32
              local.get 15
              i32.const 32
              i32.gt_u
              select
              local.set 16
              loop  ;; label = @6
                block  ;; label = @7
                  local.get 16
                  local.get 15
                  i32.ne
                  br_if 0 (;@7;)
                  local.get 16
                  local.set 15
                  br 2 (;@5;)
                end
                local.get 10
                i32.const 32
                i32.add
                local.get 15
                i32.add
                i32.load8_u
                br_if 1 (;@5;)
                local.get 15
                i32.const 1
                i32.add
                local.set 15
                br 0 (;@6;)
              end
            end
            local.get 22
            i32.const -1
            i32.add
            local.set 22
            local.get 24
            i32.const 1
            i32.add
            local.set 24
            local.get 25
            i32.const 1
            i32.add
            local.set 25
            local.get 19
            i32.const -1
            i32.add
            local.set 19
            local.get 23
            i32.const -1
            i32.add
            local.set 23
            local.get 27
            i32.const 1
            i32.add
            local.set 16
            local.get 27
            i32.const 32
            i32.lt_u
            br_if 0 (;@4;)
          end
          local.get 10
          i32.const 32
          i32.add
          local.set 14
          i32.const 24
          local.set 16
          block  ;; label = @4
            loop  ;; label = @5
              local.get 16
              i32.const -8
              i32.eq
              br_if 1 (;@4;)
              local.get 10
              local.get 16
              i32.add
              local.get 14
              i64.load align=1
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
              i64.store
              local.get 16
              i32.const -8
              i32.add
              local.set 16
              local.get 14
              i32.const 8
              i32.add
              local.set 14
              br 0 (;@5;)
            end
          end
          local.get 10
          i64.load offset=24
          local.set 9
          local.get 10
          i64.load offset=16
          local.set 11
          local.get 10
          i64.load offset=8
          local.set 12
          local.get 10
          i64.load
          local.set 13
          br 2 (;@1;)
        end
        local.get 10
        local.get 8
        i64.store offset=104
        local.get 10
        i32.const 32
        i32.add
        local.get 14
        i32.add
        local.tee 17
        i32.const 8
        i32.add
        local.get 10
        i32.const 104
        i32.add
        local.get 14
        i32.add
        i32.const 8
        i32.add
        local.tee 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 4
        i64.store offset=104
        local.get 10
        i32.const 64
        i32.add
        local.get 14
        i32.add
        local.tee 32
        i32.const 8
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 7
        i64.store offset=104
        local.get 17
        i32.const 16
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 3
        i64.store offset=104
        local.get 32
        i32.const 16
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 6
        i64.store offset=104
        local.get 17
        i32.const 24
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 2
        i64.store offset=104
        local.get 32
        i32.const 24
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 5
        i64.store offset=104
        local.get 17
        i32.const 32
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 10
        local.get 1
        i64.store offset=104
        local.get 32
        i32.const 32
        i32.add
        local.get 16
        i32.load8_u
        i32.store8
        local.get 14
        i32.const 1
        i32.add
        local.set 14
        br 0 (;@2;)
      end
    end
    local.get 0
    local.get 9
    i64.store offset=24
    local.get 0
    local.get 11
    i64.store offset=16
    local.get 0
    local.get 12
    i64.store offset=8
    local.get 0
    local.get 13
    i64.store)
  (memory (;0;) 16)
  (global (;0;) (mut i32) (i32.const 1048576))
  (global (;1;) i32 (i32.const 1048576))
  (global (;2;) i32 (i32.const 1048576))
  (export "memory" (memory 0))
  (export "arithmetic_mod" (func 0))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2)))
