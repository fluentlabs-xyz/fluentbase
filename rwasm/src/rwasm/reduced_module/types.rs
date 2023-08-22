use crate::rwasm::BinaryFormatError;

#[derive(Debug)]
pub enum ReducedModuleError {
    MissingEntrypoint,
    NotSupportedOpcode,
    NotSupportedImport,
    NotSupportedMemory(&'static str),
    ParseError(&'static str),
    OutOfBuffer,
    ReachedUnreachable,
    IllegalOpcode(u8),
    ImpossibleJump,
    InternalError(&'static str),
    MemoryOverflow,
    EmptyBytecode,
    BinaryFormat(BinaryFormatError),
}

pub const MAX_MEMORY_PAGES: u32 = 512;
