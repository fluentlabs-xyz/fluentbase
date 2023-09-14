pub(crate) mod sys_halt;
pub(crate) mod sys_read;
pub(crate) mod sys_write;

pub use crate::exec_step::{ExecStep, GadgetError};
use crate::util::Field;

pub trait PlatformGadget<F: Field> {}
