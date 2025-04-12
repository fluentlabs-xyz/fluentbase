use crate::{
    executor::RwasmExecutor,
    impl_visit_binary,
    impl_visit_fallible_binary,
    impl_visit_fallible_unary,
    impl_visit_load,
    impl_visit_store,
    impl_visit_unary,
    instr_ptr::InstructionPtr,
    module::{Instruction, InstructionData},
    Caller,
    RwasmError,
    SyscallHandler,
    FUNC_REF_OFFSET,
    TABLE_ELEMENT_NULL,
};
use core::cmp;
use rwasm::{
    core::{Pages, TrapCode, UntypedValue},
    engine::{
        bytecode::ElementSegmentIdx,
        code_map::{FuncHeader, InstructionsRef},
        executor::EntityGrowError,
    },
    rwasm::N_MAX_RECURSION_DEPTH,
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
};

pub type VisitInstruction<E, T> = fn(&mut RwasmExecutor<E, T>);

pub type InstructionTable<E, T> = [VisitInstruction<E, T>; 0x100];

#[inline]
pub const fn make_instruction_table<E: SyscallHandler<T>, T>() -> InstructionTable<E, T> {
    use Instruction::*;
    const {
        let mut tables: InstructionTable<E, T> = [visit_unreachable_wrapped; 0x100];
        tables[Unreachable as usize] = visit_unreachable_wrapped;
        tables[LocalGet as usize] = visit_local_get;
        tables[LocalSet as usize] = visit_local_set;
        tables[LocalTee as usize] = visit_local_tee;
        tables[Br as usize] = visit_br;
        tables[BrIfEqz as usize] = visit_br_if;
        tables[BrIfNez as usize] = visit_br_if_nez;
        tables[BrAdjust as usize] = visit_br_adjust;
        tables[BrAdjustIfNez as usize] = visit_br_adjust_if_nez;
        tables[BrTable as usize] = visit_br_table;
        tables[ConsumeFuel as usize] = visit_consume_fuel_wrapped;
        tables[Return as usize] = visit_return;
        tables[ReturnIfNez as usize] = visit_return_if_nez;
        tables[ReturnCallInternal as usize] = visit_return_call_internal_wrapped;
        tables[ReturnCall as usize] = visit_return_call_wrapped;
        tables[ReturnCallIndirect as usize] = visit_return_call_indirect_wrapped;
        tables[CallInternal as usize] = visit_call_internal_wrapped;
        tables[Call as usize] = visit_call_wrapped;
        tables[CallIndirect as usize] = visit_call_indirect_wrapped;
        tables[SignatureCheck as usize] = visit_signature_check_wrapped;
        tables[Drop as usize] = visit_drop;
        tables[Select as usize] = visit_select;
        tables[GlobalGet as usize] = visit_global_get;
        tables[GlobalSet as usize] = visit_global_set;
        tables[I32Load as usize] = visit_i32_load_wrapped;
        tables[I64Load as usize] = visit_i64_load_wrapped;
        tables[F32Load as usize] = visit_f32_load_wrapped; // float
        tables[F64Load as usize] = visit_f64_load_wrapped; // float
        tables[I32Load8S as usize] = visit_i32_load_i8_s_wrapped;
        tables[I32Load8U as usize] = visit_i32_load_i8_u_wrapped;
        tables[I32Load16S as usize] = visit_i32_load_i16_s_wrapped;
        tables[I32Load16U as usize] = visit_i32_load_i16_u_wrapped;
        tables[I64Load8S as usize] = visit_i64_load_i8_s_wrapped;
        tables[I64Load8U as usize] = visit_i64_load_i8_u_wrapped;
        tables[I64Load16S as usize] = visit_i64_load_i16_s_wrapped;
        tables[I64Load16U as usize] = visit_i64_load_i16_u_wrapped;
        tables[I64Load32S as usize] = visit_i64_load_i32_s_wrapped;
        tables[I64Load32U as usize] = visit_i64_load_i32_u_wrapped;
        tables[I32Store as usize] = visit_i32_store_wrapped;
        tables[I64Store as usize] = visit_i64_store_wrapped;
        tables[F32Store as usize] = visit_f32_store_wrapped; // float
        tables[F64Store as usize] = visit_f64_store_wrapped; // float
        tables[I32Store8 as usize] = visit_i32_store_8_wrapped;
        tables[I32Store16 as usize] = visit_i32_store_16_wrapped;
        tables[I64Store8 as usize] = visit_i64_store_8_wrapped;
        tables[I64Store16 as usize] = visit_i64_store_16_wrapped;
        tables[I64Store32 as usize] = visit_i64_store_32_wrapped;
        tables[MemorySize as usize] = visit_memory_size;
        tables[MemoryGrow as usize] = visit_memory_grow_wrapped;
        tables[MemoryFill as usize] = visit_memory_fill_wrapped;
        tables[MemoryCopy as usize] = visit_memory_copy_wrapped;
        tables[MemoryInit as usize] = visit_memory_init_wrapped;
        tables[DataDrop as usize] = visit_data_drop;
        tables[TableSize as usize] = visit_table_size;
        tables[TableGrow as usize] = visit_table_grow_wrapped;
        tables[TableFill as usize] = visit_table_fill_wrapped;
        tables[TableGet as usize] = visit_table_get_wrapped;
        tables[TableSet as usize] = visit_table_set_wrapped;
        tables[TableCopy as usize] = visit_table_copy_wrapped;
        tables[TableInit as usize] = visit_table_init_wrapped;
        tables[ElemDrop as usize] = visit_element_drop;
        tables[RefFunc as usize] = visit_ref_func;
        tables[I32Const as usize] = visit_i32_i64_const;
        tables[I64Const as usize] = visit_i32_i64_const;
        tables[F32Const as usize] = visit_i32_i64_const; // float
        tables[F64Const as usize] = visit_i32_i64_const; // float
        tables[I32Eqz as usize] = visit_i32_eqz;
        tables[I32Eq as usize] = visit_i32_eq;
        tables[I32Ne as usize] = visit_i32_ne;
        tables[I32LtS as usize] = visit_i32_lt_s;
        tables[I32LtU as usize] = visit_i32_lt_u;
        tables[I32GtS as usize] = visit_i32_gt_s;
        tables[I32GtU as usize] = visit_i32_gt_u;
        tables[I32LeS as usize] = visit_i32_le_s;
        tables[I32LeU as usize] = visit_i32_le_u;
        tables[I32GeS as usize] = visit_i32_ge_s;
        tables[I32GeU as usize] = visit_i32_ge_u;
        tables[I64Eqz as usize] = visit_i64_eqz;
        tables[I64Eq as usize] = visit_i64_eq;
        tables[I64Ne as usize] = visit_i64_ne;
        tables[I64LtS as usize] = visit_i64_lt_s;
        tables[I64LtU as usize] = visit_i64_lt_u;
        tables[I64GtS as usize] = visit_i64_gt_s;
        tables[I64GtU as usize] = visit_i64_gt_u;
        tables[I64LeS as usize] = visit_i64_le_s;
        tables[I64LeU as usize] = visit_i64_le_u;
        tables[I64GeS as usize] = visit_i64_ge_s;
        tables[I64GeU as usize] = visit_i64_ge_u;
        tables[F32Eq as usize] = visit_f32_eq; // float
        tables[F32Ne as usize] = visit_f32_ne; // float
        tables[F32Lt as usize] = visit_f32_lt; // float
        tables[F32Gt as usize] = visit_f32_gt; // float
        tables[F32Le as usize] = visit_f32_le; // float
        tables[F32Ge as usize] = visit_f32_ge; // float
        tables[F64Eq as usize] = visit_f64_eq; // float
        tables[F64Ne as usize] = visit_f64_ne; // float
        tables[F64Lt as usize] = visit_f64_lt; // float
        tables[F64Gt as usize] = visit_f64_gt; // float
        tables[F64Le as usize] = visit_f64_le; // float
        tables[F64Ge as usize] = visit_f64_ge; // float
        tables[I32Clz as usize] = visit_i32_clz;
        tables[I32Ctz as usize] = visit_i32_ctz;
        tables[I32Popcnt as usize] = visit_i32_popcnt;
        tables[I32Add as usize] = visit_i32_add;
        tables[I32Sub as usize] = visit_i32_sub;
        tables[I32Mul as usize] = visit_i32_mul;
        tables[I32DivS as usize] = visit_i32_div_s_wrapped;
        tables[I32DivU as usize] = visit_i32_div_u_wrapped;
        tables[I32RemS as usize] = visit_i32_rem_s_wrapped;
        tables[I32RemU as usize] = visit_i32_rem_u_wrapped;
        tables[I32And as usize] = visit_i32_and;
        tables[I32Or as usize] = visit_i32_or;
        tables[I32Xor as usize] = visit_i32_xor;
        tables[I32Shl as usize] = visit_i32_shl;
        tables[I32ShrS as usize] = visit_i32_shr_s;
        tables[I32ShrU as usize] = visit_i32_shr_u;
        tables[I32Rotl as usize] = visit_i32_rotl;
        tables[I32Rotr as usize] = visit_i32_rotr;
        tables[I64Clz as usize] = visit_i64_clz;
        tables[I64Ctz as usize] = visit_i64_ctz;
        tables[I64Popcnt as usize] = visit_i64_popcnt;
        tables[I64Add as usize] = visit_i64_add;
        tables[I64Sub as usize] = visit_i64_sub;
        tables[I64Mul as usize] = visit_i64_mul;
        tables[I64DivS as usize] = visit_i64_div_s_wrapped;
        tables[I64DivU as usize] = visit_i64_div_u_wrapped;
        tables[I64RemS as usize] = visit_i64_rem_s_wrapped;
        tables[I64RemU as usize] = visit_i64_rem_u_wrapped;
        tables[I64And as usize] = visit_i64_and;
        tables[I64Or as usize] = visit_i64_or;
        tables[I64Xor as usize] = visit_i64_xor;
        tables[I64Shl as usize] = visit_i64_shl;
        tables[I64ShrS as usize] = visit_i64_shr_s;
        tables[I64ShrU as usize] = visit_i64_shr_u;
        tables[I64Rotl as usize] = visit_i64_rotl;
        tables[I64Rotr as usize] = visit_i64_rotr;
        tables[F32Abs as usize] = visit_f32_abs; // float
        tables[F32Neg as usize] = visit_f32_neg; // float
        tables[F32Ceil as usize] = visit_f32_ceil; // float
        tables[F32Floor as usize] = visit_f32_floor; // float
        tables[F32Trunc as usize] = visit_f32_trunc; // float
        tables[F32Nearest as usize] = visit_f32_nearest; // float
        tables[F32Sqrt as usize] = visit_f32_sqrt; // float
        tables[F32Add as usize] = visit_f32_add; // float
        tables[F32Sub as usize] = visit_f32_sub; // float
        tables[F32Mul as usize] = visit_f32_mul; // float
        tables[F32Div as usize] = visit_f32_div; // float
        tables[F32Min as usize] = visit_f32_min; // float
        tables[F32Max as usize] = visit_f32_max; // float
        tables[F32Copysign as usize] = visit_f32_copysign; // float
        tables[F64Abs as usize] = visit_f64_abs; // float
        tables[F64Neg as usize] = visit_f64_neg; // float
        tables[F64Ceil as usize] = visit_f64_ceil; // float
        tables[F64Floor as usize] = visit_f64_floor; // float
        tables[F64Trunc as usize] = visit_f64_trunc; // float
        tables[F64Nearest as usize] = visit_f64_nearest; // float
        tables[F64Sqrt as usize] = visit_f64_sqrt; // float
        tables[F64Add as usize] = visit_f64_add; // float
        tables[F64Sub as usize] = visit_f64_sub; // float
        tables[F64Mul as usize] = visit_f64_mul; // float
        tables[F64Div as usize] = visit_f64_div; // float
        tables[F64Min as usize] = visit_f64_min; // float
        tables[F64Max as usize] = visit_f64_max; // float
        tables[F64Copysign as usize] = visit_f64_copysign; // float
        tables[I32WrapI64 as usize] = visit_i32_wrap_i64;
        tables[I32TruncF32S as usize] = visit_i32_trunc_f32_s_wrapped; // float
        tables[I32TruncF32U as usize] = visit_i32_trunc_f32_u_wrapped; // float
        tables[I32TruncF64S as usize] = visit_i32_trunc_f64_s_wrapped; // float
        tables[I32TruncF64U as usize] = visit_i32_trunc_f64_u_wrapped; // float
        tables[I64ExtendI32S as usize] = visit_i64_extend_i32_s;
        tables[I64ExtendI32U as usize] = visit_i64_extend_i32_u;
        tables[I64TruncF32S as usize] = visit_i64_trunc_f32_s_wrapped; // float
        tables[I64TruncF32U as usize] = visit_i64_trunc_f32_u_wrapped; // float
        tables[I64TruncF64S as usize] = visit_i64_trunc_f64_s_wrapped; // float
        tables[I64TruncF64U as usize] = visit_i64_trunc_f64_u_wrapped; // float
        tables[F32ConvertI32S as usize] = visit_f32_convert_i32_s; // float
        tables[F32ConvertI32U as usize] = visit_f32_convert_i32_u; // float
        tables[F32ConvertI64S as usize] = visit_f32_convert_i64_s; // float
        tables[F32ConvertI64U as usize] = visit_f32_convert_i64_u; // float
        tables[F32DemoteF64 as usize] = visit_f32_demote_f64; // float
        tables[F64ConvertI32S as usize] = visit_f64_convert_i32_s; // float
        tables[F64ConvertI32U as usize] = visit_f64_convert_i32_u; // float
        tables[F64ConvertI64S as usize] = visit_f64_convert_i64_s; // float
        tables[F64ConvertI64U as usize] = visit_f64_convert_i64_u; // float
        tables[F64PromoteF32 as usize] = visit_f64_promote_f32; // float
        tables[I32TruncSatF32S as usize] = visit_i32_trunc_sat_f32_s; // float
        tables[I32TruncSatF32U as usize] = visit_i32_trunc_sat_f32_u; // float
        tables[I32TruncSatF64S as usize] = visit_i32_trunc_sat_f64_s; // float
        tables[I32TruncSatF64U as usize] = visit_i32_trunc_sat_f64_u; // float
        tables[I64TruncSatF32S as usize] = visit_i64_trunc_sat_f32_s; // float
        tables[I64TruncSatF32U as usize] = visit_i64_trunc_sat_f32_u; // float
        tables[I64TruncSatF64S as usize] = visit_i64_trunc_sat_f64_s; // float
        tables[I64TruncSatF64U as usize] = visit_i64_trunc_sat_f64_u; // float
        tables[I32Extend8S as usize] = visit_i32_extend8_s;
        tables[I32Extend16S as usize] = visit_i32_extend16_s;
        tables[I64Extend8S as usize] = visit_i64_extend8_s;
        tables[I64Extend16S as usize] = visit_i64_extend16_s;
        tables[I64Extend32S as usize] = visit_i64_extend32_s;
        tables
    }
}

pub(crate) fn run_the_loop<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
    instruction_table: &InstructionTable<E, T>,
) -> Result<i32, RwasmError> {
    while !exec.stop_exec {
        let instr = *exec.store.ip.get();
        instruction_table[instr as usize](exec);
    }
    exec.stop_exec = false;
    exec.next_result
        .take()
        .unwrap_or_else(|| unreachable!("rwasm: next result without reason?"))
}

macro_rules! wrap_function_result {
    ($fn_name:ident) => {
        paste::paste! {
            #[inline(always)]
            pub(crate) fn [< $fn_name _wrapped >]<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>,) {
                if let Err(err) = $fn_name(exec, /* &mut ResourceLimiterRef<'_> */) {
                    exec.next_result = Some(Err(RwasmError::from(err)));
                    exec.stop_exec = true;
                }
            }
        }
    };
}

wrap_function_result!(visit_unreachable);
wrap_function_result!(visit_consume_fuel);
wrap_function_result!(visit_return_call_internal);
wrap_function_result!(visit_return_call);
wrap_function_result!(visit_return_call_indirect);
wrap_function_result!(visit_call_internal);
wrap_function_result!(visit_call);
wrap_function_result!(visit_call_indirect);
wrap_function_result!(visit_signature_check);
wrap_function_result!(visit_memory_grow);
wrap_function_result!(visit_memory_fill);
wrap_function_result!(visit_memory_copy);
wrap_function_result!(visit_memory_init);
wrap_function_result!(visit_table_grow);
wrap_function_result!(visit_table_fill);
wrap_function_result!(visit_table_get);
wrap_function_result!(visit_table_set);
wrap_function_result!(visit_table_copy);
wrap_function_result!(visit_table_init);

// #[cfg(feature = "debug-print")]
// {
//     let stack = self
//         .store
//         .value_stack
//         .dump_stack(exec.store.sp)
//         .iter()
//         .rev()
//         .take(10)
//         .map(|v| v.as_u64())
//         .collect::<Vec<_>>();
//     println!(
//         "{:02}: {:?}, stack={:?} ({})",
//         exec.store.ip.pc(),
//         instr,
//         stack,
//         exec.store.value_stack.stack_len(exec.store.sp)
//     );
// }
//
// #[cfg(feature = "tracer")]
// if exec.store.tracer.is_some() {
//     use rwasm::engine::bytecode::InstrMeta;
//     let memory_size: u32 = exec.store.global_memory.current_pages().into();
//     let consumed_fuel = exec.store.fuel_consumed();
//     let stack = exec.store.value_stack.dump_stack(exec.store.sp);
//     exec.store.tracer.as_mut().unwrap().pre_opcode_state(
//         exec.store.ip.pc(),
//         instr,
//         stack,
//         &InstrMeta::new(0, 0, 0),
//         memory_size,
//         consumed_fuel,
//     );
// }

#[inline(always)]
pub(crate) fn visit_unreachable<E: SyscallHandler<T>, T>(
    _exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    Err(RwasmError::TrapCode(TrapCode::UnreachableCodeReached))
}

#[inline(always)]
pub(crate) fn visit_local_get<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let local_depth = match exec.store.ip.data() {
        InstructionData::LocalDepth(local_depth) => local_depth,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let value = exec.store.sp.nth_back(local_depth.to_usize());
    exec.store.sp.push(value);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_local_set<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let local_depth = match exec.store.ip.data() {
        InstructionData::LocalDepth(local_depth) => local_depth,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let new_value = exec.store.sp.pop();
    exec.store
        .sp
        .set_nth_back(local_depth.to_usize(), new_value);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_local_tee<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let local_depth = match exec.store.ip.data() {
        InstructionData::LocalDepth(local_depth) => local_depth,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let new_value = exec.store.sp.last();
    exec.store
        .sp
        .set_nth_back(local_depth.to_usize(), new_value);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_br<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let branch_offset = match exec.store.ip.data() {
        InstructionData::BranchOffset(branch_offset) => branch_offset,
        _ => unreachable!("rwasm: missing instr data"),
    };
    exec.store.ip.offset(branch_offset.to_i32() as isize)
}

#[inline(always)]
pub(crate) fn visit_br_if<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let branch_offset = match exec.store.ip.data() {
        InstructionData::BranchOffset(branch_offset) => branch_offset,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let condition = exec.store.sp.pop_as();
    if condition {
        exec.store.ip.add(1);
    } else {
        exec.store.ip.offset(branch_offset.to_i32() as isize);
    }
}

#[inline(always)]
pub(crate) fn visit_br_if_nez<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let branch_offset = match exec.store.ip.data() {
        InstructionData::BranchOffset(branch_offset) => branch_offset,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let condition = exec.store.sp.pop_as();
    if condition {
        exec.store.ip.offset(branch_offset.to_i32() as isize);
    } else {
        exec.store.ip.add(1);
    }
}

#[inline(always)]
pub(crate) fn visit_br_adjust<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let branch_offset = match exec.store.ip.data() {
        InstructionData::BranchOffset(branch_offset) => branch_offset,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let drop_keep = exec.fetch_drop_keep(1);
    exec.store.sp.drop_keep(drop_keep);
    exec.store.ip.offset(branch_offset.to_i32() as isize);
}

#[inline(always)]
pub(crate) fn visit_br_adjust_if_nez<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let branch_offset = match exec.store.ip.data() {
        InstructionData::BranchOffset(branch_offset) => branch_offset,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let condition = exec.store.sp.pop_as();
    if condition {
        let drop_keep = exec.fetch_drop_keep(1);
        exec.store.sp.drop_keep(drop_keep);
        exec.store.ip.offset(branch_offset.to_i32() as isize);
    } else {
        exec.store.ip.add(2);
    }
}

#[inline(always)]
pub(crate) fn visit_br_table<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let targets = match exec.store.ip.data() {
        InstructionData::BranchTableTargets(targets) => targets,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let index: u32 = exec.store.sp.pop_as();
    let max_index = targets.to_usize() - 1;
    let normalized_index = cmp::min(index as usize, max_index);
    exec.store.ip.add(2 * normalized_index + 1);
}

#[inline(always)]
pub(crate) fn visit_consume_fuel<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let block_fuel = match exec.store.ip.data() {
        InstructionData::BlockFuel(block_fuel) => block_fuel,
        _ => unreachable!("rwasm: missing instr data"),
    };
    if exec.store.config.fuel_enabled {
        exec.store.try_consume_fuel(block_fuel.to_u64())?;
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_return<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let drop_keep = match exec.store.ip.data() {
        InstructionData::DropKeep(drop_keep) => drop_keep,
        _ => unreachable!("rwasm: missing instr data"),
    };
    exec.store.sp.drop_keep(*drop_keep);
    exec.store.value_stack.sync_stack_ptr(exec.store.sp);
    match exec.store.call_stack.pop() {
        Some(caller) => {
            exec.store.ip = caller;
        }
        None => {
            exec.next_result = Some(Ok(0));
            exec.stop_exec = true;
        }
    }
}

#[inline(always)]
pub(crate) fn visit_return_if_nez<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let drop_keep = match exec.store.ip.data() {
        InstructionData::DropKeep(drop_keep) => drop_keep,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let condition = exec.store.sp.pop_as();
    if condition {
        exec.store.sp.drop_keep(*drop_keep);
        exec.store.value_stack.sync_stack_ptr(exec.store.sp);
        match exec.store.call_stack.pop() {
            Some(caller) => {
                exec.store.ip = caller;
            }
            None => {
                exec.next_result = Some(Ok(0));
                exec.stop_exec = true;
            }
        }
    } else {
        exec.store.ip.add(1);
    }
}

#[inline(always)]
pub(crate) fn visit_return_call_internal<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let func_idx = match exec.store.ip.data() {
        InstructionData::CompiledFunc(func_idx) => *func_idx,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let drop_keep = exec.fetch_drop_keep(1);
    exec.store.sp.drop_keep(drop_keep);
    exec.store.ip.add(2);
    exec.store.value_stack.sync_stack_ptr(exec.store.sp);
    let instr_ref = exec
        .store
        .module
        .func_segments
        .get(func_idx.to_u32() as usize)
        .copied()
        .expect("rwasm: unknown internal function");
    let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
    exec.store.value_stack.prepare_wasm_call(&header)?;
    exec.store.sp = exec.store.value_stack.stack_ptr();
    exec.store.ip = InstructionPtr::new(
        exec.store.module.code_section.as_ptr(),
        exec.store.module.instr_data.as_ptr(),
    );
    exec.store.ip.add(instr_ref as usize);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_return_call<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let func_idx = match exec.store.ip.data() {
        InstructionData::CompiledFunc(func_idx) => *func_idx,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let drop_keep = exec.fetch_drop_keep(1);
    exec.store.sp.drop_keep(drop_keep);
    exec.store.value_stack.sync_stack_ptr(exec.store.sp);
    // external call can cause interruption,
    // that is why it's important to increase IP before doing the call
    exec.store.ip.add(2);
    E::call_function(Caller::new(&mut exec.store), func_idx.to_u32())?;
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_return_call_indirect<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let signature_idx = match exec.store.ip.data() {
        InstructionData::SignatureIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let drop_keep = exec.fetch_drop_keep(1);
    let table = exec.fetch_table_index(2);
    let func_index: u32 = exec.store.sp.pop_as();
    exec.store.sp.drop_keep(drop_keep);
    exec.store.last_signature = Some(signature_idx);
    let func_idx: u32 = exec
        .store
        .tables
        .get(&table)
        .expect("rwasm: unresolved table index")
        .get(func_index)
        .and_then(|v| v.i32())
        .ok_or(TrapCode::TableOutOfBounds)?
        .try_into()
        .unwrap();
    if func_idx == 0 {
        return Err(TrapCode::IndirectCallToNull.into());
    }
    let func_idx = func_idx - FUNC_REF_OFFSET;
    exec.execute_call_internal(false, 3, func_idx)
}

#[inline(always)]
pub(crate) fn visit_call_internal<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let func_idx = match exec.store.ip.data() {
        InstructionData::CompiledFunc(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    exec.store.ip.add(1);
    exec.store.value_stack.sync_stack_ptr(exec.store.sp);
    if exec.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
        return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
    }
    exec.store.call_stack.push(exec.store.ip);
    let instr_ref = exec
        .store
        .module
        .func_segments
        .get(func_idx.to_u32() as usize)
        .copied()
        .expect("rwasm: unknown internal function");
    let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
    exec.store.value_stack.prepare_wasm_call(&header)?;
    exec.store.sp = exec.store.value_stack.stack_ptr();
    exec.store.ip = InstructionPtr::new(
        exec.store.module.code_section.as_ptr(),
        exec.store.module.instr_data.as_ptr(),
    );
    exec.store.ip.add(instr_ref as usize);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_call<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let func_idx = match exec.store.ip.data() {
        InstructionData::FuncIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    exec.store.value_stack.sync_stack_ptr(exec.store.sp);
    // external call can cause interruption,
    // that is why it's important to increase IP before doing the call
    exec.store.ip.add(1);
    E::call_function(Caller::new(&mut exec.store), func_idx.to_u32())
}

#[inline(always)]
pub(crate) fn visit_call_indirect<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let signature_idx = match exec.store.ip.data() {
        InstructionData::SignatureIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    // resolve func index
    let table = exec.fetch_table_index(1);
    let func_index: u32 = exec.store.sp.pop_as();
    exec.store.last_signature = Some(signature_idx);
    let func_idx: u32 = exec
        .store
        .tables
        .get(&table)
        .expect("rwasm: unresolved table index")
        .get(func_index)
        .and_then(|v| v.i32().map(|v| v as u32))
        .ok_or(TrapCode::TableOutOfBounds)?;
    if func_idx == TABLE_ELEMENT_NULL {
        return Err(TrapCode::IndirectCallToNull.into());
    }
    let func_idx = func_idx - FUNC_REF_OFFSET;
    // call func
    exec.store.ip.add(2);
    exec.store.value_stack.sync_stack_ptr(exec.store.sp);
    if exec.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
        return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
    }
    exec.store.call_stack.push(exec.store.ip);
    let instr_ref = exec
        .store
        .module
        .func_segments
        .get(func_idx as usize)
        .copied()
        .expect("rwasm: unknown internal function");
    let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
    exec.store.value_stack.prepare_wasm_call(&header)?;
    exec.store.sp = exec.store.value_stack.stack_ptr();
    exec.store.ip = InstructionPtr::new(
        exec.store.module.code_section.as_ptr(),
        exec.store.module.instr_data.as_ptr(),
    );
    exec.store.ip.add(instr_ref as usize);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_signature_check<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let signature_idx = match exec.store.ip.data() {
        InstructionData::SignatureIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    if let Some(actual_signature) = exec.store.last_signature.take() {
        if actual_signature != signature_idx {
            return Err(TrapCode::BadSignature).map_err(Into::into);
        }
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_drop<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    exec.store.sp.drop();
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_select<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    exec.store.sp.eval_top3(|e1, e2, e3| {
        let condition = <bool as From<UntypedValue>>::from(e3);
        if condition {
            e1
        } else {
            e2
        }
    });
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_global_get<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let global_idx = match exec.store.ip.data() {
        InstructionData::GlobalIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let global_value = exec
        .store
        .global_variables
        .get(&global_idx)
        .copied()
        .unwrap_or_default();
    exec.store.sp.push(global_value);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_global_set<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let global_idx = match exec.store.ip.data() {
        InstructionData::GlobalIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let new_value = exec.store.sp.pop();
    exec.store.global_variables.insert(global_idx, new_value);
    exec.store.ip.add(1);
}

impl_visit_load! {
    fn visit_i32_load(i32_load);
    fn visit_i64_load(i64_load);
    fn visit_f32_load(f32_load);
    fn visit_f64_load(f64_load);

    fn visit_i32_load_i8_s(i32_load8_s);
    fn visit_i32_load_i8_u(i32_load8_u);
    fn visit_i32_load_i16_s(i32_load16_s);
    fn visit_i32_load_i16_u(i32_load16_u);

    fn visit_i64_load_i8_s(i64_load8_s);
    fn visit_i64_load_i8_u(i64_load8_u);
    fn visit_i64_load_i16_s(i64_load16_s);
    fn visit_i64_load_i16_u(i64_load16_u);
    fn visit_i64_load_i32_s(i64_load32_s);
    fn visit_i64_load_i32_u(i64_load32_u);
}

impl_visit_store! {
    fn visit_i32_store(i32_store, 4);
    fn visit_i64_store(i64_store, 8);
    fn visit_f32_store(f32_store, 4);
    fn visit_f64_store(f64_store, 8);

    fn visit_i32_store_8(i32_store8, 1);
    fn visit_i32_store_16(i32_store16, 2);

    fn visit_i64_store_8(i64_store8, 1);
    fn visit_i64_store_16(i64_store16, 2);
    fn visit_i64_store_32(i64_store32, 4);
}

#[inline(always)]
pub(crate) fn visit_memory_size<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let result: u32 = exec.store.global_memory.current_pages().into();
    exec.store.sp.push_as(result);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_memory_grow<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let mut limiter = ResourceLimiterRef::default();
    let delta: u32 = exec.store.sp.pop_as();
    let delta = match Pages::new(delta) {
        Some(delta) => delta,
        None => {
            exec.store.sp.push_as(u32::MAX);
            exec.store.ip.add(1);
            return Ok(());
        }
    };
    if exec.store.config.fuel_enabled {
        let delta_in_bytes = delta.to_bytes().unwrap_or(0) as u64;
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_bytes(delta_in_bytes))?;
    }
    let new_pages = exec
        .store
        .global_memory
        .grow(delta, &mut limiter)
        .map(u32::from)
        .unwrap_or(u32::MAX);
    exec.store.sp.push_as(new_pages);
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_fill<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let (d, val, n) = exec.store.sp.pop3();
    let n = i32::from(n) as usize;
    let offset = i32::from(d) as usize;
    let byte = u8::from(val);
    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_bytes(n as u64))?;
    }
    let memory = exec
        .store
        .global_memory
        .data_mut()
        .get_mut(offset..)
        .and_then(|memory| memory.get_mut(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    memory.fill(byte);
    if let Some(tracer) = exec.store.tracer.as_mut() {
        tracer.memory_change(offset as u32, n as u32, memory);
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_copy<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let (d, s, n) = exec.store.sp.pop3();
    let n = i32::from(n) as usize;
    let src_offset = i32::from(s) as usize;
    let dst_offset = i32::from(d) as usize;
    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_bytes(n as u64))?;
    }
    // these accesses just perform the bound checks required by the Wasm spec.
    let data = exec.store.global_memory.data_mut();
    data.get(src_offset..)
        .and_then(|memory| memory.get(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    data.get(dst_offset..)
        .and_then(|memory| memory.get(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    data.copy_within(src_offset..src_offset.wrapping_add(n), dst_offset);
    if let Some(tracer) = exec.store.tracer.as_mut() {
        tracer.memory_change(
            dst_offset as u32,
            n as u32,
            &data[dst_offset..(dst_offset + n)],
        );
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_init<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let data_segment_idx = match exec.store.ip.data() {
        InstructionData::DataSegmentIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let is_empty_data_segment = exec.resolve_data_or_create(data_segment_idx).is_empty();
    let (d, s, n) = exec.store.sp.pop3();
    let n = i32::from(n) as usize;
    let src_offset = i32::from(s) as usize;
    let dst_offset = i32::from(d) as usize;
    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_bytes(n as u64))?;
    }
    let memory = exec
        .store
        .global_memory
        .data_mut()
        .get_mut(dst_offset..)
        .and_then(|memory| memory.get_mut(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    let mut memory_section = exec.store.module.memory_section.as_slice();
    if is_empty_data_segment {
        memory_section = &[];
    }
    let data = memory_section
        .get(src_offset..)
        .and_then(|data| data.get(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    memory.copy_from_slice(data);
    if let Some(tracer) = exec.store.tracer.as_mut() {
        tracer.global_memory(dst_offset as u32, n as u32, memory);
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_data_drop<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let data_segment_idx = match exec.store.ip.data() {
        InstructionData::DataSegmentIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let data_segment = exec.resolve_data_or_create(data_segment_idx);
    data_segment.drop_bytes();
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_table_size<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let table_idx = match exec.store.ip.data() {
        InstructionData::TableIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let table_size = exec
        .store
        .tables
        .get(&table_idx)
        .expect("rwasm: unresolved table segment")
        .size();
    exec.store.sp.push_as(table_size);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_table_grow<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let mut limiter = ResourceLimiterRef::default();
    let table_idx = match exec.store.ip.data() {
        InstructionData::TableIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let (init, delta) = exec.store.sp.pop2();
    let delta: u32 = delta.into();
    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_elements(delta as u64))?;
    }
    let table = exec.resolve_table_or_create(table_idx);
    let result = match table.grow_untyped(delta, init, &mut limiter) {
        Ok(result) => result,
        Err(EntityGrowError::TrapCode(trap_code)) => return Err(RwasmError::TrapCode(trap_code)),
        Err(EntityGrowError::InvalidGrow) => u32::MAX,
    };
    exec.store.sp.push_as(result);
    if let Some(tracer) = exec.store.tracer.as_mut() {
        tracer.table_size_change(table_idx.to_u32(), init.as_u32(), delta);
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_fill<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let table_idx = match exec.store.ip.data() {
        InstructionData::TableIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let (i, val, n) = exec.store.sp.pop3();
    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_elements(n.as_u64()))?;
    }
    exec.resolve_table(table_idx)
        .fill_untyped(i.as_u32(), val, n.as_u32())?;
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_get<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let table_idx = match exec.store.ip.data() {
        InstructionData::TableIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let index = exec.store.sp.pop();
    let value = exec
        .resolve_table(table_idx)
        .get_untyped(index.as_u32())
        .ok_or(TrapCode::TableOutOfBounds)?;
    exec.store.sp.push(value);
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_set<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let table_idx = match exec.store.ip.data() {
        InstructionData::TableIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let (index, value) = exec.store.sp.pop2();
    exec.resolve_table(table_idx)
        .set_untyped(index.as_u32(), value)
        .map_err(|_| TrapCode::TableOutOfBounds)?;
    if let Some(tracer) = exec.store.tracer.as_mut() {
        tracer.table_change(table_idx.to_u32(), index.as_u32(), value);
    }
    exec.store.ip.add(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_copy<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let dst_table_idx = match exec.store.ip.data() {
        InstructionData::TableIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let src_table_idx = exec.fetch_table_index(1);
    let (d, s, n) = exec.store.sp.pop3();
    let len = u32::from(n);
    let src_index = u32::from(s);
    let dst_index = u32::from(d);
    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_elements(n.as_u64()))?;
    }
    // Query both tables and check if they are the same:
    if src_table_idx != dst_table_idx {
        let [src, dst] = exec
            .store
            .tables
            .get_many_mut([&src_table_idx, &dst_table_idx])
            .map(|v| v.expect("rwasm: unresolved table segment"));
        TableEntity::copy(dst, dst_index, src, src_index, len)?;
    } else {
        let src = exec
            .store
            .tables
            .get_mut(&src_table_idx)
            .expect("rwasm: unresolved table segment");
        src.copy_within(dst_index, src_index, len)?;
    }
    exec.store.ip.add(2);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_init<E: SyscallHandler<T>, T>(
    exec: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let element_segment_idx = match exec.store.ip.data() {
        InstructionData::ElementSegmentIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let table_idx = exec.fetch_table_index(1);
    let (d, s, n) = exec.store.sp.pop3();
    let len = u32::from(n);
    let src_index = u32::from(s);
    let dst_index = u32::from(d);

    if exec.store.config.fuel_enabled {
        exec.store
            .try_consume_fuel(exec.store.fuel_costs.fuel_for_elements(len as u64))?;
    }

    // There is a trick with `element_segment_idx`:
    // it refers to the segment number.
    // However, in rwasm, all elements are stored in segment 0,
    // so there is no need to store information about the remaining segments.
    // According to the WebAssembly standards, though,
    // we must retain information about all dropped element segments
    // to perform an emptiness check.
    // Therefore, in `element_segment_idx`, we store the original index,
    // which is always > 0.
    let is_empty_segment = exec
        .resolve_element_or_create(element_segment_idx)
        .is_empty();

    let (table, mut element) =
        exec.resolve_table_with_element_or_create(table_idx, ElementSegmentIdx::from(0));
    let mut empty_element_segment = ElementSegmentEntity::empty(element.ty());
    if is_empty_segment {
        element = &mut empty_element_segment;
    }
    table.init_untyped(dst_index, element, src_index, len)?;
    exec.store.ip.add(2);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_element_drop<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let element_segment_idx = match exec.store.ip.data() {
        InstructionData::ElementSegmentIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    let element_segment = exec.resolve_element_or_create(element_segment_idx);
    element_segment.drop_items();
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_ref_func<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let func_idx = match exec.store.ip.data() {
        InstructionData::FuncIdx(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    exec.store.sp.push_as(func_idx.to_u32() + FUNC_REF_OFFSET);
    exec.store.ip.add(1);
}

#[inline(always)]
pub(crate) fn visit_i32_i64_const<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
    let untyped_value = match exec.store.ip.data() {
        InstructionData::UntypedValue(value) => *value,
        _ => unreachable!("rwasm: missing instr data"),
    };
    exec.store.sp.push(untyped_value);
    exec.store.ip.add(1);
}

impl_visit_unary! {
    fn visit_i32_eqz(i32_eqz);
    fn visit_i64_eqz(i64_eqz);

    fn visit_i32_clz(i32_clz);
    fn visit_i32_ctz(i32_ctz);
    fn visit_i32_popcnt(i32_popcnt);

    fn visit_i64_clz(i64_clz);
    fn visit_i64_ctz(i64_ctz);
    fn visit_i64_popcnt(i64_popcnt);

    fn visit_f32_abs(f32_abs);
    fn visit_f32_neg(f32_neg);
    fn visit_f32_ceil(f32_ceil);
    fn visit_f32_floor(f32_floor);
    fn visit_f32_trunc(f32_trunc);
    fn visit_f32_nearest(f32_nearest);
    fn visit_f32_sqrt(f32_sqrt);

    fn visit_f64_abs(f64_abs);
    fn visit_f64_neg(f64_neg);
    fn visit_f64_ceil(f64_ceil);
    fn visit_f64_floor(f64_floor);
    fn visit_f64_trunc(f64_trunc);
    fn visit_f64_nearest(f64_nearest);
    fn visit_f64_sqrt(f64_sqrt);

    fn visit_i32_wrap_i64(i32_wrap_i64);
    fn visit_i64_extend_i32_s(i64_extend_i32_s);
    fn visit_i64_extend_i32_u(i64_extend_i32_u);

    fn visit_f32_convert_i32_s(f32_convert_i32_s);
    fn visit_f32_convert_i32_u(f32_convert_i32_u);
    fn visit_f32_convert_i64_s(f32_convert_i64_s);
    fn visit_f32_convert_i64_u(f32_convert_i64_u);
    fn visit_f32_demote_f64(f32_demote_f64);
    fn visit_f64_convert_i32_s(f64_convert_i32_s);
    fn visit_f64_convert_i32_u(f64_convert_i32_u);
    fn visit_f64_convert_i64_s(f64_convert_i64_s);
    fn visit_f64_convert_i64_u(f64_convert_i64_u);
    fn visit_f64_promote_f32(f64_promote_f32);

    fn visit_i32_extend8_s(i32_extend8_s);
    fn visit_i32_extend16_s(i32_extend16_s);
    fn visit_i64_extend8_s(i64_extend8_s);
    fn visit_i64_extend16_s(i64_extend16_s);
    fn visit_i64_extend32_s(i64_extend32_s);

    fn visit_i32_trunc_sat_f32_s(i32_trunc_sat_f32_s);
    fn visit_i32_trunc_sat_f32_u(i32_trunc_sat_f32_u);
    fn visit_i32_trunc_sat_f64_s(i32_trunc_sat_f64_s);
    fn visit_i32_trunc_sat_f64_u(i32_trunc_sat_f64_u);
    fn visit_i64_trunc_sat_f32_s(i64_trunc_sat_f32_s);
    fn visit_i64_trunc_sat_f32_u(i64_trunc_sat_f32_u);
    fn visit_i64_trunc_sat_f64_s(i64_trunc_sat_f64_s);
    fn visit_i64_trunc_sat_f64_u(i64_trunc_sat_f64_u);
}

impl_visit_fallible_unary! {
    fn visit_i32_trunc_f32_s(i32_trunc_f32_s);
    fn visit_i32_trunc_f32_u(i32_trunc_f32_u);
    fn visit_i32_trunc_f64_s(i32_trunc_f64_s);
    fn visit_i32_trunc_f64_u(i32_trunc_f64_u);

    fn visit_i64_trunc_f32_s(i64_trunc_f32_s);
    fn visit_i64_trunc_f32_u(i64_trunc_f32_u);
    fn visit_i64_trunc_f64_s(i64_trunc_f64_s);
    fn visit_i64_trunc_f64_u(i64_trunc_f64_u);
}

impl_visit_binary! {
    fn visit_i32_eq(i32_eq);
    fn visit_i32_ne(i32_ne);
    fn visit_i32_lt_s(i32_lt_s);
    fn visit_i32_lt_u(i32_lt_u);
    fn visit_i32_gt_s(i32_gt_s);
    fn visit_i32_gt_u(i32_gt_u);
    fn visit_i32_le_s(i32_le_s);
    fn visit_i32_le_u(i32_le_u);
    fn visit_i32_ge_s(i32_ge_s);
    fn visit_i32_ge_u(i32_ge_u);

    fn visit_i64_eq(i64_eq);
    fn visit_i64_ne(i64_ne);
    fn visit_i64_lt_s(i64_lt_s);
    fn visit_i64_lt_u(i64_lt_u);
    fn visit_i64_gt_s(i64_gt_s);
    fn visit_i64_gt_u(i64_gt_u);
    fn visit_i64_le_s(i64_le_s);
    fn visit_i64_le_u(i64_le_u);
    fn visit_i64_ge_s(i64_ge_s);
    fn visit_i64_ge_u(i64_ge_u);

    fn visit_f32_eq(f32_eq);
    fn visit_f32_ne(f32_ne);
    fn visit_f32_lt(f32_lt);
    fn visit_f32_gt(f32_gt);
    fn visit_f32_le(f32_le);
    fn visit_f32_ge(f32_ge);

    fn visit_f64_eq(f64_eq);
    fn visit_f64_ne(f64_ne);
    fn visit_f64_lt(f64_lt);
    fn visit_f64_gt(f64_gt);
    fn visit_f64_le(f64_le);
    fn visit_f64_ge(f64_ge);

    fn visit_i32_add(i32_add);
    fn visit_i32_sub(i32_sub);
    fn visit_i32_mul(i32_mul);
    fn visit_i32_and(i32_and);
    fn visit_i32_or(i32_or);
    fn visit_i32_xor(i32_xor);
    fn visit_i32_shl(i32_shl);
    fn visit_i32_shr_s(i32_shr_s);
    fn visit_i32_shr_u(i32_shr_u);
    fn visit_i32_rotl(i32_rotl);
    fn visit_i32_rotr(i32_rotr);

    fn visit_i64_add(i64_add);
    fn visit_i64_sub(i64_sub);
    fn visit_i64_mul(i64_mul);
    fn visit_i64_and(i64_and);
    fn visit_i64_or(i64_or);
    fn visit_i64_xor(i64_xor);
    fn visit_i64_shl(i64_shl);
    fn visit_i64_shr_s(i64_shr_s);
    fn visit_i64_shr_u(i64_shr_u);
    fn visit_i64_rotl(i64_rotl);
    fn visit_i64_rotr(i64_rotr);

    fn visit_f32_add(f32_add);
    fn visit_f32_sub(f32_sub);
    fn visit_f32_mul(f32_mul);
    fn visit_f32_div(f32_div);
    fn visit_f32_min(f32_min);
    fn visit_f32_max(f32_max);
    fn visit_f32_copysign(f32_copysign);

    fn visit_f64_add(f64_add);
    fn visit_f64_sub(f64_sub);
    fn visit_f64_mul(f64_mul);
    fn visit_f64_div(f64_div);
    fn visit_f64_min(f64_min);
    fn visit_f64_max(f64_max);
    fn visit_f64_copysign(f64_copysign);
}

impl_visit_fallible_binary! {
    fn visit_i32_div_s(i32_div_s);
    fn visit_i32_div_u(i32_div_u);
    fn visit_i32_rem_s(i32_rem_s);
    fn visit_i32_rem_u(i32_rem_u);

    fn visit_i64_div_s(i64_div_s);
    fn visit_i64_div_u(i64_div_u);
    fn visit_i64_rem_s(i64_rem_s);
    fn visit_i64_rem_u(i64_rem_u);
}
