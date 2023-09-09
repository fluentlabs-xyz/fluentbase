use fluentbase_rwasm::common::Trap;

#[derive(Debug, Copy, Clone)]
pub enum ExitCode {
    Stop = -1001,
    MemoryOutOfBounds = -1002,
}

impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        Trap::i32_exit(self as i32)
    }
}
