use fluentbase_rwasm::common::Trap;

#[derive(Debug, Copy, Clone)]
pub enum ExitCode {
    EvmStop = -1001,
    MemoryOutOfBounds = -1002,
    NotSupportedCall = -1003,
}

impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        Trap::i32_exit(self as i32)
    }
}
