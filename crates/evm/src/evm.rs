use crate::{
    bytecode::AnalyzedBytecode,
    gas::Gas,
    memory::SharedMemory,
    result::{InstructionResult, InterpreterResult},
    stack::Stack,
};
use fluentbase_sdk::{debug_log, Bytes, ContextReader, SharedAPI, FUEL_DENOM_RATE};

mod arithmetic;
mod bitwise;
mod context;
mod contract;
mod control;
mod host;
mod i256;
mod memory;
mod stack;
mod system;

/// EVM opcode function signature.
pub type Instruction<SDK> = fn(&mut EVM<SDK>);

/// Instruction table is a list of instruction function pointers mapped to 256 EVM opcodes.
pub type InstructionTable<SDK> = [Instruction<SDK>; 0x100];

pub struct EVM<'a, SDK: SharedAPI> {
    pub sdk: &'a mut SDK,
    pub analyzed_bytecode: AnalyzedBytecode,
    pub input: &'a [u8],
    pub gas: Gas,
    pub committed_gas: Gas,
    pub ip: *const u8,
    pub state: InstructionResult,
    pub return_data_buffer: Bytes,
    pub is_static: bool,
    pub output: Option<InterpreterResult>,
    pub memory: SharedMemory,
    pub stack: Stack,
}

impl<'a, SDK: SharedAPI> EVM<'a, SDK> {
    pub fn new(
        sdk: &'a mut SDK,
        analyzed_bytecode: AnalyzedBytecode,
        input: &'a [u8],
        gas_limit: u64,
    ) -> Self {
        let is_static = sdk.context().contract_is_static();
        let ip = analyzed_bytecode.bytecode.as_ptr();
        let gas = Gas::new(gas_limit);
        Self {
            sdk,
            analyzed_bytecode,
            input,
            gas,
            committed_gas: gas,
            ip,
            state: InstructionResult::Continue,
            return_data_buffer: Default::default(),
            is_static,
            output: None,
            memory: Default::default(),
            stack: Default::default(),
        }
    }

    pub fn sync_evm_gas(&mut self) -> bool {
        let remaining_diff = self.committed_gas.remaining() - self.gas.remaining();
        let refunded_diff = self.gas.refunded() - self.committed_gas.refunded();
        if remaining_diff == 0 && refunded_diff == 0 {
            return false;
        }
        self.sdk.charge_fuel_manually(
            remaining_diff * FUEL_DENOM_RATE,
            refunded_diff * FUEL_DENOM_RATE as i64,
        );
        self.committed_gas = self.gas;
        true
    }

    pub fn exec(&mut self) -> InterpreterResult {
        let instruction_table = make_instruction_table::<SDK>();
        while self.state == InstructionResult::Continue {
            let opcode = unsafe { *self.ip };
            debug_log!("opcode: {}", opcode);
            self.ip = unsafe { self.ip.offset(1) };
            instruction_table[opcode as usize](self);
        }
        if let Some(output) = self.output.take() {
            return output;
        }
        InterpreterResult {
            result: self.state,
            output: Bytes::new(),
            gas: self.gas,
            committed_gas: self.committed_gas,
        }
    }

    pub fn program_counter(&self) -> usize {
        unsafe {
            self.ip
                .offset_from(self.analyzed_bytecode.bytecode.as_ptr()) as usize
        }
    }
}

#[inline]
pub const fn make_instruction_table<SDK: SharedAPI>() -> InstructionTable<SDK> {
    const {
        let mut tables: InstructionTable<SDK> = [control::unknown; 0x100];
        tables[0x00] = control::stop;
        tables[0x01] = arithmetic::add;
        tables[0x02] = arithmetic::mul;
        tables[0x03] = arithmetic::sub;
        tables[0x04] = arithmetic::div;
        tables[0x05] = arithmetic::sdiv;
        tables[0x06] = arithmetic::rem;
        tables[0x07] = arithmetic::smod;
        tables[0x08] = arithmetic::addmod;
        tables[0x09] = arithmetic::mulmod;
        tables[0x0A] = arithmetic::exp;
        tables[0x0B] = arithmetic::signextend;
        // 0x0C
        // 0x0D
        // 0x0E
        // 0x0F
        tables[0x10] = bitwise::lt;
        tables[0x11] = bitwise::gt;
        tables[0x12] = bitwise::slt;
        tables[0x13] = bitwise::sgt;
        tables[0x14] = bitwise::eq;
        tables[0x15] = bitwise::iszero;
        tables[0x16] = bitwise::bitand;
        tables[0x17] = bitwise::bitor;
        tables[0x18] = bitwise::bitxor;
        tables[0x19] = bitwise::not;
        tables[0x1A] = bitwise::byte;
        tables[0x1B] = bitwise::shl;
        tables[0x1C] = bitwise::shr;
        tables[0x1D] = bitwise::sar;
        // 0x1E
        // 0x1F
        tables[0x20] = system::keccak256;
        // 0x21
        // 0x22
        // 0x23
        // 0x24
        // 0x25
        // 0x26
        // 0x27
        // 0x28
        // 0x29
        // 0x2A
        // 0x2B
        // 0x2C
        // 0x2D
        // 0x2E
        // 0x2F
        tables[0x30] = system::address;
        tables[0x31] = host::balance;
        tables[0x32] = context::origin;
        tables[0x33] = system::caller;
        tables[0x34] = system::callvalue;
        tables[0x35] = system::calldataload;
        tables[0x36] = system::calldatasize;
        tables[0x37] = system::calldatacopy;
        tables[0x38] = system::codesize;
        tables[0x39] = system::codecopy;
        tables[0x3A] = context::gasprice;
        tables[0x3B] = host::extcodesize;
        tables[0x3C] = host::extcodecopy;
        tables[0x3D] = system::returndatasize;
        tables[0x3E] = system::returndatacopy;
        tables[0x3F] = host::extcodehash;
        tables[0x40] = host::blockhash;
        tables[0x41] = context::coinbase;
        tables[0x42] = context::timestamp;
        tables[0x43] = context::block_number;
        tables[0x44] = context::difficulty;
        tables[0x45] = context::gaslimit;
        tables[0x46] = context::chainid;
        tables[0x47] = host::selfbalance;
        tables[0x48] = context::basefee;
        tables[0x49] = context::blob_hash;
        tables[0x4A] = context::blob_basefee;
        // 0x4B
        // 0x4C
        // 0x4D
        // 0x4E
        // 0x4F
        tables[0x50] = stack::pop;
        tables[0x51] = memory::mload;
        tables[0x52] = memory::mstore;
        tables[0x53] = memory::mstore8;
        tables[0x54] = host::sload;
        tables[0x55] = host::sstore;
        tables[0x56] = control::jump;
        tables[0x57] = control::jumpi;
        tables[0x58] = control::pc;
        tables[0x59] = memory::msize;
        tables[0x5A] = system::gas;
        tables[0x5B] = control::jumpdest_or_nop;
        tables[0x5C] = host::tload;
        tables[0x5D] = host::tstore;
        tables[0x5E] = memory::mcopy;
        tables[0x5F] = stack::push0;
        tables[0x60] = stack::push::<1, SDK>;
        tables[0x61] = stack::push::<2, SDK>;
        tables[0x62] = stack::push::<3, SDK>;
        tables[0x63] = stack::push::<4, SDK>;
        tables[0x64] = stack::push::<5, SDK>;
        tables[0x65] = stack::push::<6, SDK>;
        tables[0x66] = stack::push::<7, SDK>;
        tables[0x67] = stack::push::<8, SDK>;
        tables[0x68] = stack::push::<9, SDK>;
        tables[0x69] = stack::push::<10, SDK>;
        tables[0x6A] = stack::push::<11, SDK>;
        tables[0x6B] = stack::push::<12, SDK>;
        tables[0x6C] = stack::push::<13, SDK>;
        tables[0x6D] = stack::push::<14, SDK>;
        tables[0x6E] = stack::push::<15, SDK>;
        tables[0x6F] = stack::push::<16, SDK>;
        tables[0x70] = stack::push::<17, SDK>;
        tables[0x71] = stack::push::<18, SDK>;
        tables[0x72] = stack::push::<19, SDK>;
        tables[0x73] = stack::push::<20, SDK>;
        tables[0x74] = stack::push::<21, SDK>;
        tables[0x75] = stack::push::<22, SDK>;
        tables[0x76] = stack::push::<23, SDK>;
        tables[0x77] = stack::push::<24, SDK>;
        tables[0x78] = stack::push::<25, SDK>;
        tables[0x79] = stack::push::<26, SDK>;
        tables[0x7A] = stack::push::<27, SDK>;
        tables[0x7B] = stack::push::<28, SDK>;
        tables[0x7C] = stack::push::<29, SDK>;
        tables[0x7D] = stack::push::<30, SDK>;
        tables[0x7E] = stack::push::<31, SDK>;
        tables[0x7F] = stack::push::<32, SDK>;
        tables[0x80] = stack::dup::<1, SDK>;
        tables[0x81] = stack::dup::<2, SDK>;
        tables[0x82] = stack::dup::<3, SDK>;
        tables[0x83] = stack::dup::<4, SDK>;
        tables[0x84] = stack::dup::<5, SDK>;
        tables[0x85] = stack::dup::<6, SDK>;
        tables[0x86] = stack::dup::<7, SDK>;
        tables[0x87] = stack::dup::<8, SDK>;
        tables[0x88] = stack::dup::<9, SDK>;
        tables[0x89] = stack::dup::<10, SDK>;
        tables[0x8A] = stack::dup::<11, SDK>;
        tables[0x8B] = stack::dup::<12, SDK>;
        tables[0x8C] = stack::dup::<13, SDK>;
        tables[0x8D] = stack::dup::<14, SDK>;
        tables[0x8E] = stack::dup::<15, SDK>;
        tables[0x8F] = stack::dup::<16, SDK>;
        tables[0x90] = stack::swap::<1, SDK>;
        tables[0x91] = stack::swap::<2, SDK>;
        tables[0x92] = stack::swap::<3, SDK>;
        tables[0x93] = stack::swap::<4, SDK>;
        tables[0x94] = stack::swap::<5, SDK>;
        tables[0x95] = stack::swap::<6, SDK>;
        tables[0x96] = stack::swap::<7, SDK>;
        tables[0x97] = stack::swap::<8, SDK>;
        tables[0x98] = stack::swap::<9, SDK>;
        tables[0x99] = stack::swap::<10, SDK>;
        tables[0x9A] = stack::swap::<11, SDK>;
        tables[0x9B] = stack::swap::<12, SDK>;
        tables[0x9C] = stack::swap::<13, SDK>;
        tables[0x9D] = stack::swap::<14, SDK>;
        tables[0x9E] = stack::swap::<15, SDK>;
        tables[0x9F] = stack::swap::<16, SDK>;
        tables[0xA0] = host::log::<0, SDK>;
        tables[0xA1] = host::log::<1, SDK>;
        tables[0xA2] = host::log::<2, SDK>;
        tables[0xA3] = host::log::<3, SDK>;
        tables[0xA4] = host::log::<4, SDK>;
        // 0xA5
        // 0xA6
        // 0xA7
        // 0xA8
        // 0xA9
        // 0xAA
        // 0xAB
        // 0xAC
        // 0xAD
        // 0xAE
        // 0xAF
        // 0xB0
        // 0xB1
        // 0xB2
        // 0xB3
        // 0xB4
        // 0xB5
        // 0xB6
        // 0xB7
        // 0xB8
        // 0xB9
        // 0xBA
        // 0xBB
        // 0xBC
        // 0xBD
        // 0xBE
        // 0xBF
        // 0xC0
        // 0xC1
        // 0xC2
        // 0xC3
        // 0xC4
        // 0xC5
        // 0xC6
        // 0xC7
        // 0xC8
        // 0xC9
        // 0xCA
        // 0xCB
        // 0xCC
        // 0xCD
        // 0xCE
        // 0xCF
        // tables[0xD0] = data::data_load;
        // tables[0xD1] = data::data_loadn;
        // tables[0xD2] = data::data_size;
        // tables[0xD3] = data::data_copy;
        // 0xD4
        // 0xD5
        // 0xD6
        // 0xD7
        // 0xD8
        // 0xD9
        // 0xDA
        // 0xDB
        // 0xDC
        // 0xDD
        // 0xDE
        // 0xDF
        // tables[0xE0] = control::rjump;
        // tables[0xE1] = control::rjumpi;
        // tables[0xE2] = control::rjumpv;
        // tables[0xE3] = control::callf;
        // tables[0xE4] = control::retf;
        // tables[0xE5] = control::jumpf;
        // tables[0xE6] = stack::dupn;
        // tables[0xE7] = stack::swapn;
        // tables[0xE8] = stack::exchange;
        // 0xE9
        // 0xEA
        // 0xEB
        // tables[0xEC] = contract::eofcreate;
        // 0xED
        // tables[0xEE] = contract::return_contract;
        // 0xEF
        tables[0xF0] = contract::create::<false, SDK>;
        tables[0xF1] = contract::call;
        tables[0xF2] = contract::call_code;
        tables[0xF3] = control::ret;
        tables[0xF4] = contract::delegate_call;
        tables[0xF5] = contract::create::<true, SDK>;
        // 0xF6
        // tables[0xF7] = system::returndataload;
        // tables[0xF8] = contract::extcall;
        // tables[0xF9] = contract::extdelegatecall;
        tables[0xFA] = contract::static_call;
        // tables[0xFB] = contract::extstaticcall;
        // 0xFC
        tables[0xFD] = control::revert;
        tables[0xFE] = control::invalid;
        tables[0xFF] = host::selfdestruct;
        tables
    }
}
