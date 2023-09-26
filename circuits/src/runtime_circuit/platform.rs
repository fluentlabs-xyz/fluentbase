pub(crate) mod rwasm_transact;
pub(crate) mod sys_halt;
pub(crate) mod sys_read;
pub(crate) mod sys_write;
pub(crate) mod wasi_args_get;
pub(crate) mod wasi_args_sizes_get;
pub(crate) mod wasi_environ_get;
pub(crate) mod wasi_environ_sizes_get;
pub(crate) mod wasi_fd_write;
pub(crate) mod wasi_proc_exit;

pub use crate::exec_step::{ExecStep, GadgetError};
use crate::util::Field;

pub trait PlatformGadget<F: Field> {}
