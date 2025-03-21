use rwasm::core::TrapCode;
use rwasm::engine::bytecode::Instruction;
use crate::RwasmExecutor;

macro_rules! next_instr {
    ($exec:expr) => {{
        let instr_idx = $exec.dtc_code[$exec.dtc_ip];
        let handler = DISPATCH_TABLE[instr_idx];
        handler($exec);
    }};
}
