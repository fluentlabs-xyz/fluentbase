use fluentbase_rwasm::common::Trap;

pub const EXIT_CODE_EVM_STOP: i32 = -1001;

#[derive(Debug, Copy, Clone)]
pub enum ExitCode {
    Stop,
}

impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        Trap::i32_exit(self.exit_code() as i32)
    }
}

impl ExitCode {
    pub fn exit_code(&self) -> u32 {
        match self {
            Self::Stop => EXIT_CODE_EVM_STOP as u32,
            _ => unreachable!("unknown exit code: {:?}", self),
        }
    }
}
