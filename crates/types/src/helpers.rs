use crate::{
    create_sovereign_import_linker,
    ExitCode,
    SysFuncIdx::SYS_STATE,
    STATE_DEPLOY,
    STATE_MAIN,
};
use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
    Error,
};
