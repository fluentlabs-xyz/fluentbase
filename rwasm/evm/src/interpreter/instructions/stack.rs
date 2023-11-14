use crate::interpreter::host::Host;
use crate::interpreter::instruction_result::InstructionResult;
use crate::interpreter::interpreter::Translator;
use fluentbase_rwasm::rwasm::_CRYPTO_KECCAK256_FUEL_AMOUNT;

pub fn pop<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    if let Err(result) = interpreter.stack.pop() {
        interpreter.instruction_result = result;
    }
}

/// EIP-3855: PUSH0 instruction
///
/// Introduce a new instruction which pushes the constant value 0 onto the stack.
pub fn push0<H: Host /* , SPEC: Spec */>(interpreter: &mut Translator<'_>, host: &mut H) {
    println!("op: PUSH0");
    let instruction_set = host.instruction_set();
    (0..4).for_each(|_| instruction_set.op_i64_const(0));

    // check!(interpreter, SHANGHAI);
    // gas!(interpreter, gas::BASE);
    // if let Err(result) = interpreter.stack.push(U256::ZERO) {
    //     interpreter.instruction_result = result;
    // }
}

pub fn push<const N: usize, H: Host>(interpreter: &mut Translator<'_>, host: &mut H) {
    let ip = interpreter.instruction_pointer;
    let data = unsafe { core::slice::from_raw_parts(ip, N) };
    println!("op: PUSH{} {:x?}", N, data);

    let instruction_set = host.instruction_set();
    (0..N).for_each(|i| {
        instruction_set.op_i32_const(data[i]);
    });

    interpreter.instruction_result = InstructionResult::Continue;
    interpreter.instruction_pointer = unsafe { ip.add(N) }

    // gas!(interpreter, gas::VERYLOW);
    // SAFETY: In analysis we append trailing bytes to the bytecode so that this is safe to do
    // without bounds checking.
    // let ip = interpreter.instruction_pointer;
    // if let Err(result) = interpreter
    //     .stack
    //     .push_slice(unsafe { core::slice::from_raw_parts(ip, N) })
    // {
    //     interpreter.instruction_result = result;
    //     return;
    // }
    // interpreter.instruction_pointer = unsafe { ip.add(N) };
}

pub fn dup<const N: usize, H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::VERYLOW);
    if let Err(result) = interpreter.stack.dup::<N>() {
        interpreter.instruction_result = result;
    }
}

pub fn swap<const N: usize, H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::VERYLOW);
    if let Err(result) = interpreter.stack.swap::<N>() {
        interpreter.instruction_result = result;
    }
}
