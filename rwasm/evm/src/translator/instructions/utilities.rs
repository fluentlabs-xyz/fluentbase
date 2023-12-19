use crate::translator::{host::Host, instructions::opcode, translator::Translator};
use fluentbase_rwasm::{engine::bytecode::Instruction, module::ImportName, rwasm::InstructionSet};

pub(super) enum SystemFuncs {
    CryptoKeccak256,
    EvmSstore,
    EvmSload,
}

pub(super) fn wasm_call(
    instruction_set: &mut InstructionSet,
    fn_name: SystemFuncs,
    translator: &mut Translator,
) {
    let fn_name = match fn_name {
        SystemFuncs::CryptoKeccak256 => "_crypto_keccak256",
        SystemFuncs::EvmSstore => "_evm_sstore",
        SystemFuncs::EvmSload => "_evm_sload",
    };
    let import_fn_idx =
        translator.get_import_linker().index_mapping()[&ImportName::new("env", fn_name)].0;
    instruction_set.op_call(import_fn_idx);
}

pub(super) fn preprocess_op_params(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
    inject_memory_result_offset: bool,
    memory_result_offset_is_first_param: bool,
) {
    let opcode = translator.opcode_prev();
    let i64_stack_params_count: usize;
    const MEM_RESULT_OFFSET: usize = 0;
    match opcode {
        opcode::ISZERO | opcode::POP => {
            // i64_stack_params_count = 4;
            i64_stack_params_count = 0;
        }

        opcode::BYTE
        | opcode::EQ
        | opcode::GAS
        | opcode::LT
        | opcode::GT
        | opcode::SAR
        | opcode::SGT
        | opcode::SHL
        | opcode::SHR
        | opcode::SLT
        | opcode::ADD
        | opcode::SIGNEXTEND
        | opcode::SUB
        | opcode::MUL
        | opcode::DIV
        | opcode::MSTORE
        | opcode::MSTORE8
        | opcode::EXP
        | opcode::MOD
        | opcode::SMOD
        | opcode::SDIV => {
            // i64_stack_params_count = 8;
            i64_stack_params_count = 0;
        }

        opcode::MULMOD | opcode::ADDMOD => {
            // i64_stack_params_count = 12;
            i64_stack_params_count = 0;
        }

        opcode::AND | opcode::NOT | opcode::OR | opcode::XOR => {
            i64_stack_params_count = 0;
        }
        _ => {
            panic!("no postprocessing defined for 0x{:x?} opcode", opcode)
        }
    }

    let instruction_set = host.instruction_set();
    let mut aux_params_count = 0;
    if inject_memory_result_offset {
        aux_params_count += 1;
        if memory_result_offset_is_first_param {
            let offset_instruction = instruction_set.instr[instruction_set.len() as usize - 4];
            let offset = match offset_instruction {
                Instruction::I64Const(offset) => offset,
                x => {
                    panic!("unexpected instruction: {:?}", x)
                }
            };
            let mem_result_offset = offset.as_usize();
            instruction_set.op_i32_const(mem_result_offset);
        } else {
            instruction_set.op_i32_const(MEM_RESULT_OFFSET);
        }
    }
    if aux_params_count > 0 {
        let instruction_set_len = instruction_set.len() as usize;
        let last_item_idx = instruction_set_len - 1;
        let aux_params_start_idx = instruction_set_len - aux_params_count;
        let aux_params_end_idx = last_item_idx;
        let aux_params = instruction_set.instr[aux_params_start_idx..=aux_params_end_idx].to_vec();
        let params_start_idx = instruction_set_len - i64_stack_params_count - aux_params_count;
        let params_end_idx = params_start_idx + i64_stack_params_count;
        let params = instruction_set.instr[params_start_idx..params_end_idx].to_vec();

        instruction_set.instr[params_start_idx..params_start_idx + aux_params_count]
            .copy_from_slice(&aux_params);
        instruction_set.instr[params_start_idx + aux_params_count
            ..params_start_idx + aux_params_count + i64_stack_params_count]
            .clone_from_slice(&params);
    }
    // if inject_return_offset {
    let meta = translator
        .subroutine_meta(opcode)
        .expect(&format!("no meta found for 0x{:x?} opcode", opcode));
    let prev_funcs_len = meta.begin_offset as u32;
    instruction_set.op_i32_const(instruction_set.len() + 1 - prev_funcs_len);
    // }
}

pub(super) fn replace_current_opcode_with_call_to_subroutine(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
    inject_memory_result_offset: bool,
    memory_result_offset_is_first_param: bool,
) {
    preprocess_op_params(
        translator,
        host,
        inject_memory_result_offset,
        memory_result_offset_is_first_param,
    );

    let instruction_set = host.instruction_set();
    let opcode = translator.opcode_prev();
    let subroutine_meta = translator
        .subroutine_meta(opcode)
        .expect(format!("subroutine meta not found for opcode 0x{:x?}", opcode).as_str());
    let subroutine_data = translator
        .subroutine_data(opcode)
        .expect(format!("subroutine data not found for opcode 0x{:x?}", opcode).as_str());

    let subroutine_entry = subroutine_meta.begin_offset as i32 - instruction_set.len() as i32
        + 1
        + subroutine_data.entry_offset as i32;
    instruction_set.op_br(subroutine_entry);
}
