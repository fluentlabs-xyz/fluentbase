.globl __get_stack_pointer
.hidden __get_stack_pointer
.globl __set_stack_pointer
.hidden __set_stack_pointer

.globaltype __stack_pointer, i32

__get_stack_pointer:
  .functype __get_stack_pointer() -> (i32)
  global.get __stack_pointer
  end_function

__set_stack_pointer:
  .functype __set_stack_pointer(i32) -> ()
  local.get 0
  global.set __stack_pointer
  end_function
