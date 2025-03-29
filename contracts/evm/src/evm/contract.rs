use crate::utils::{get_memory_input_and_out_ranges, insert_call_outcome, insert_create_outcome};
use alloc::boxed::Box;
use fluentbase_sdk::{SharedAPI, FUEL_DENOM_RATE};
use revm_interpreter::{
    as_usize_or_fail,
    gas,
    gas::{cost_per_word, EOF_CREATE_GAS, KECCAK256WORD},
    gas_or_fail,
    instructions::contract::resize_memory,
    interpreter::Interpreter,
    pop,
    pop_address,
    pop_ret,
    primitives::{
        eof::EofHeader,
        keccak256,
        wasm::{WASM_MAGIC_BYTES, WASM_MAX_CODE_SIZE},
        Address,
        Bytes,
        Eof,
        B256,
        U256,
    },
    require_eof,
    require_init_eof,
    require_non_staticcall,
    resize_memory,
    CallInputs,
    CallScheme,
    CallValue,
    EOFCreateInputs,
    InstructionResult,
    InterpreterAction,
    InterpreterResult,
    MAX_INITCODE_SIZE,
};

/// EOF Create instruction
pub fn eofcreate<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    require_eof!(interpreter);
    require_non_staticcall!(interpreter);
    gas!(interpreter, EOF_CREATE_GAS);
    let initcontainer_index = unsafe { *interpreter.instruction_pointer };
    pop!(interpreter, value, salt, data_offset, data_size);

    let sub_container = interpreter
        .eof()
        .expect("EOF is set")
        .body
        .container_section
        .get(initcontainer_index as usize)
        .cloned()
        .expect("EOF is checked");

    // resize memory and get return range.
    let Some(input_range) = resize_memory(interpreter, data_offset, data_size) else {
        return;
    };

    let input = if !input_range.is_empty() {
        interpreter
            .shared_memory
            .slice_range(input_range)
            .to_vec()
            .into()
    } else {
        Bytes::new()
    };

    let eof = Eof::decode(sub_container.clone()).expect("Subcontainer is verified");

    if !eof.body.is_data_filled {
        // should be always false as it is verified by eof verification.
        panic!("Panic if data section is not full");
    }

    // deduct gas for hash that is needed to calculate address.
    gas_or_fail!(
        interpreter,
        cost_per_word(sub_container.len() as u64, KECCAK256WORD)
    );

    let created_address = interpreter
        .contract
        .target_address
        .create2(salt.to_be_bytes(), keccak256(sub_container));

    let gas_limit = interpreter.gas().remaining_63_of_64_parts();
    gas!(interpreter, gas_limit);
    // Send container for execution container is preverified.
    interpreter.instruction_result = InstructionResult::CallOrCreate;
    interpreter.next_action = InterpreterAction::EOFCreate {
        inputs: Box::new(EOFCreateInputs::new_opcode(
            interpreter.contract.target_address,
            created_address,
            value,
            eof,
            gas_limit,
            input,
        )),
    };

    interpreter.instruction_pointer = unsafe { interpreter.instruction_pointer.offset(1) };
}

pub fn return_contract<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    require_init_eof!(interpreter);
    let deploy_container_index = unsafe { *interpreter.instruction_pointer };
    pop!(interpreter, aux_data_offset, aux_data_size);
    let aux_data_size = as_usize_or_fail!(interpreter, aux_data_size);
    // important: offset must be ignored if len is zeros
    let container = interpreter
        .eof()
        .expect("EOF is set")
        .body
        .container_section
        .get(deploy_container_index as usize)
        .expect("EOF is checked")
        .clone();

    // convert to EOF so we can check data section size.
    let (eof_header, _) = EofHeader::decode(&container).expect("valid EOF header");

    let aux_slice = if aux_data_size != 0 {
        let aux_data_offset = as_usize_or_fail!(interpreter, aux_data_offset);
        resize_memory!(interpreter, aux_data_offset, aux_data_size);

        interpreter
            .shared_memory
            .slice(aux_data_offset, aux_data_size)
    } else {
        &[]
    };

    let static_aux_size = eof_header.eof_size() - container.len();

    // data_size - static_aux_size give us current data `container` size.
    // and with aux_slice len we can calculate new data size.
    let new_data_size = eof_header.data_size as usize - static_aux_size + aux_slice.len();
    if new_data_size > 0xFFFF {
        // aux data is too big
        interpreter.instruction_result = InstructionResult::EofAuxDataOverflow;
        return;
    }
    if new_data_size < eof_header.data_size as usize {
        // aux data is too small
        interpreter.instruction_result = InstructionResult::EofAuxDataTooSmall;
        return;
    }
    let new_data_size = (new_data_size as u16).to_be_bytes();

    let mut output = [&container, aux_slice].concat();
    // set new data size in eof bytes as we know exact index.
    output[eof_header.data_size_raw_i()..][..2].clone_from_slice(&new_data_size);
    let output: Bytes = output.into();

    let result = InstructionResult::ReturnContract;
    interpreter.instruction_result = result;
    interpreter.next_action = InterpreterAction::Return {
        result: InterpreterResult {
            output,
            gas: interpreter.gas,
            result,
        },
    };
}

pub fn extcall_input(interpreter: &mut Interpreter) -> Option<Bytes> {
    pop_ret!(interpreter, input_offset, input_size, None);

    let return_memory_offset = resize_memory(interpreter, input_offset, input_size)?;

    if return_memory_offset.is_empty() {
        return Some(Bytes::new());
    }

    Some(Bytes::copy_from_slice(
        interpreter
            .shared_memory
            .slice_range(return_memory_offset.clone()),
    ))
}

pub fn extcall_gas_calc<SDK: SharedAPI>(
    _interpreter: &mut Interpreter,
    _sdk: &mut SDK,
    _target: Address,
    _transfers_value: bool,
) -> Option<u64> {
    todo!("not implemented")
}

/// Pop target address from stack and check if it is valid.
///
/// Valid address has first 12 bytes as zeroes.
#[inline]
pub fn pop_extcall_target_address(interpreter: &mut Interpreter) -> Option<Address> {
    pop_ret!(interpreter, target_address, None);
    let target_address = B256::from(target_address);
    // Check if target is left padded with zeroes.
    if target_address[..12].iter().any(|i| *i != 0) {
        interpreter.instruction_result = InstructionResult::InvalidEXTCALLTarget;
        return None;
    }
    // discard first 12 bytes.
    Some(Address::from_word(target_address))
}

pub fn extcall<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_eof!(interpreter);

    // pop target address
    let Some(target_address) = pop_extcall_target_address(interpreter) else {
        return;
    };

    // input call
    let Some(input) = extcall_input(interpreter) else {
        return;
    };

    pop!(interpreter, value);
    let has_transfer = !value.is_zero();
    if interpreter.is_static && has_transfer {
        interpreter.instruction_result = InstructionResult::CallNotAllowedInsideStatic;
        return;
    }

    let Some(gas_limit) = extcall_gas_calc(interpreter, sdk, target_address, has_transfer) else {
        return;
    };

    // Call host to interact with target contract
    interpreter.next_action = InterpreterAction::Call {
        inputs: Box::new(CallInputs {
            input,
            gas_limit,
            target_address,
            caller: interpreter.contract.target_address,
            bytecode_address: target_address,
            value: CallValue::Transfer(value),
            scheme: CallScheme::ExtCall,
            is_static: interpreter.is_static,
            is_eof: true,
            return_memory_offset: 0..0,
        }),
    };
    interpreter.instruction_result = InstructionResult::CallOrCreate;
}

pub fn extdelegatecall<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_eof!(interpreter);

    // pop target address
    let Some(target_address) = pop_extcall_target_address(interpreter) else {
        return;
    };

    // input call
    let Some(input) = extcall_input(interpreter) else {
        return;
    };

    let Some(gas_limit) = extcall_gas_calc(interpreter, sdk, target_address, false) else {
        return;
    };

    // Call host to interact with target contract
    interpreter.next_action = InterpreterAction::Call {
        inputs: Box::new(CallInputs {
            input,
            gas_limit,
            target_address: interpreter.contract.target_address,
            caller: interpreter.contract.caller,
            bytecode_address: target_address,
            value: CallValue::Apparent(interpreter.contract.call_value),
            scheme: CallScheme::ExtDelegateCall,
            is_static: interpreter.is_static,
            is_eof: true,
            return_memory_offset: 0..0,
        }),
    };
    interpreter.instruction_result = InstructionResult::CallOrCreate;
}

pub fn extstaticcall<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_eof!(interpreter);

    // pop target address
    let Some(target_address) = pop_extcall_target_address(interpreter) else {
        return;
    };

    // input call
    let Some(input) = extcall_input(interpreter) else {
        return;
    };

    let Some(gas_limit) = extcall_gas_calc(interpreter, sdk, target_address, false) else {
        return;
    };

    // Call host to interact with target contract
    interpreter.next_action = InterpreterAction::Call {
        inputs: Box::new(CallInputs {
            input,
            gas_limit,
            target_address,
            caller: interpreter.contract.target_address,
            bytecode_address: target_address,
            value: CallValue::Transfer(U256::ZERO),
            scheme: CallScheme::ExtStaticCall,
            is_static: true,
            is_eof: true,
            return_memory_offset: 0..0,
        }),
    };
    interpreter.instruction_result = InstructionResult::CallOrCreate;
}

pub fn create<const IS_CREATE2: bool, SDK: SharedAPI>(
    interpreter: &mut Interpreter,
    sdk: &mut SDK,
) {
    require_non_staticcall!(interpreter);
    pop!(interpreter, value, code_offset, len);
    let code_len = as_usize_or_fail!(interpreter, len);
    let mut init_code = Bytes::new();
    let init_gas_cost = if code_len != 0 {
        let code_offset = as_usize_or_fail!(interpreter, code_offset);
        let max_initcode_size = if code_len >= 4
            && interpreter.shared_memory.try_slice(code_offset, 4) == Some(&WASM_MAGIC_BYTES)
        {
            WASM_MAX_CODE_SIZE
        } else {
            MAX_INITCODE_SIZE
        };
        if code_len > max_initcode_size {
            interpreter.instruction_result = InstructionResult::CreateInitCodeSizeLimit;
            return;
        }
        let init_gas_cost = gas::initcode_cost(code_len as u64);
        if init_gas_cost > interpreter.gas.remaining() {
            interpreter.instruction_result = InstructionResult::OutOfGas;
            return;
        }
        resize_memory!(interpreter, code_offset, code_len);
        init_code = Bytes::copy_from_slice(interpreter.shared_memory.slice(code_offset, code_len));
        init_gas_cost
    } else {
        0
    };
    let gas_cost = if IS_CREATE2 {
        let Some(gas) = gas::create2_cost(init_code.len().try_into().unwrap()) else {
            interpreter.instruction_result = InstructionResult::OutOfGas;
            return;
        };
        gas
    } else {
        gas::CREATE
    };
    if init_gas_cost + gas_cost > interpreter.gas.remaining() {
        interpreter.instruction_result = InstructionResult::OutOfGas;
        return;
    }
    let salt: Option<U256> = if IS_CREATE2 {
        pop!(interpreter, salt);
        Some(salt)
    } else {
        None
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    sdk.sync_evm_gas(interpreter.gas.remaining(), interpreter.gas.refunded());
    let result = sdk.create(salt, &value, init_code.as_ref());
    insert_create_outcome(interpreter, result)
}

pub fn call<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop!(interpreter, local_gas_limit);
    pop_address!(interpreter, to);
    // max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    pop!(interpreter, value);
    let has_transfer = !value.is_zero();
    if interpreter.is_static && has_transfer {
        interpreter.instruction_result = InstructionResult::CallNotAllowedInsideStatic;
        return;
    }
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(interpreter) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    sdk.sync_evm_gas(interpreter.gas.remaining(), interpreter.gas.refunded());
    let result = sdk.call(
        to,
        value,
        input.as_ref(),
        Some(local_gas_limit * FUEL_DENOM_RATE),
    );
    insert_call_outcome(interpreter, result, return_memory_offset);
}

pub fn call_code<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop!(interpreter, local_gas_limit);
    pop_address!(interpreter, to);
    // max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    pop!(interpreter, value);
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(interpreter) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    sdk.sync_evm_gas(interpreter.gas.remaining(), interpreter.gas.refunded());
    let result = sdk.call_code(
        to,
        value,
        input.as_ref(),
        Some(local_gas_limit * FUEL_DENOM_RATE),
    );
    insert_call_outcome(interpreter, result, return_memory_offset);
}

pub fn delegate_call<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop!(interpreter, local_gas_limit);
    pop_address!(interpreter, to);
    // max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(interpreter) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    sdk.sync_evm_gas(interpreter.gas.remaining(), interpreter.gas.refunded());
    let result = sdk.delegate_call(to, input.as_ref(), Some(local_gas_limit * FUEL_DENOM_RATE));
    insert_call_outcome(interpreter, result, return_memory_offset);
}

pub fn static_call<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop!(interpreter, local_gas_limit);
    pop_address!(interpreter, to);
    // max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(interpreter) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    sdk.sync_evm_gas(interpreter.gas.remaining(), interpreter.gas.refunded());
    let result = sdk.static_call(to, input.as_ref(), Some(local_gas_limit * FUEL_DENOM_RATE));
    insert_call_outcome(interpreter, result, return_memory_offset);
}
