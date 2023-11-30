use crate::{
    translator::{host::Host, translator::Translator},
    utilities::{WASM_I64_BYTES, WASM_I64_IN_EVM_WORD_COUNT},
};
use log::debug;

pub fn mload<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MLOAD";
    panic!("op:{} not implemented", OP);
    let instruction_set = host.instruction_set();

    // offset manipulation
    instruction_set.op_local_set(4);
    // drop values left
    (0..2).for_each(|_| instruction_set.op_drop());
    instruction_set.op_local_get(1);

    // drop values left
    // (0..WASM_I64_IN_EVM_WORD_COUNT).for_each(|_|instruction_set.op_drop())
}

pub fn mstore<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSTORE";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();
    for i in 0..WASM_I64_IN_EVM_WORD_COUNT as u32 {
        // for offset
        instruction_set.op_local_get(5);
        // for value
        instruction_set.op_local_get(5 - i);
        instruction_set.op_i64_store(i * WASM_I64_BYTES as u32);
    }
    // drop values left
    (0..WASM_I64_IN_EVM_WORD_COUNT * 2).for_each(|_| instruction_set.op_drop());
}

pub fn mstore8<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSORE8";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();
    // for offset
    instruction_set.op_local_get(5);
    // for value
    instruction_set.op_local_get(2);
    instruction_set.op_i64_store8(0);
    // drop values left
    (0..WASM_I64_IN_EVM_WORD_COUNT * 2).for_each(|_| instruction_set.op_drop());
}

pub fn msize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MSIZE";
    panic!("op:{} not implemented", OP);
}

pub fn mcopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MCOPY";
    panic!("op:{} not implemented", OP);
}
