.globl memcpy
.hidden memcpy

memcpy:
  .functype memcpy(i32, i32, i32) -> (i32)
  local.get 0
  local.get 1
  local.get 2
  memory.copy 0, 1
  i32.const 0
  end_function