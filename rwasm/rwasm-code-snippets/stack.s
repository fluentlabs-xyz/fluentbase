.globl __get_stack_pointer
.hidden __get_stack_pointer

.globaltype __stack_pointer, i32

__get_stack_pointer:
  .functype __get_stack_pointer() -> (i32)
  global.get __stack_pointer
  end_function
