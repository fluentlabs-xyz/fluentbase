use crate::{
    executor::RwasmExecutor,
    impl_visit_binary,
    impl_visit_fallible_binary,
    impl_visit_fallible_unary,
    impl_visit_load,
    impl_visit_store,
    impl_visit_unary,
    Caller,
    OpCodeTable,
    RwasmError,
    SyscallHandler,
    FUNC_REF_OFFSET,
    TABLE_ELEMENT_NULL,
};
use core::cmp;
use rwasm::{
    core::{Pages, TrapCode, UntypedValue},
    engine::{
        bytecode::{
            AddressOffset,
            BlockFuel,
            BranchOffset,
            BranchTableTargets,
            DataSegmentIdx,
            ElementSegmentIdx,
            FuncIdx,
            GlobalIdx,
            LocalDepth,
            SignatureIdx,
            TableIdx,
        },
        code_map::{FuncHeader, InstructionsRef},
        executor::EntityGrowError,
        CompiledFunc,
        DropKeep,
    },
    rwasm::N_MAX_RECURSION_DEPTH,
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
};

pub(crate) fn run_the_loop<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    opcode_table: OpCodeTable<E, T>,
) -> Result<(), RwasmError> {
    loop {
        // debug_assert_eq!(
        //     executor.store.program_counter % MAX_INSTRUCTION_SIZE_BYTES,
        //     0,
        //     "rwasm: program counter must be aligned",
        // );
        let opcode = executor.store.module.code_section[executor.store.program_counter] as usize;
        // {
        //     if let Some(prev_pc) = executor.prev_pc {
        //         assert_ne!(
        //             executor.store.program_counter, prev_pc,
        //             "rwasm: why pc is the same?"
        //         );
        //     }
        //     executor.prev_pc = Some(executor.store.program_counter);
        //     let instruction = Instruction::read_from_slice(
        //         &executor.store.module.code_section[executor.store.program_counter..],
        //     )
        //     .unwrap_or_else(|_| unreachable!("waaaait!"));
        //     println!(
        //         "pc={}, opcode={}, instr={:?}",
        //         executor.store.program_counter, opcode, instruction
        //     );
        // }
        opcode_table[opcode](executor)?;
    }
}

#[inline(always)]
pub(crate) fn visit_unknown<E: SyscallHandler<T>, T>(
    _executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    unreachable!("rwasm: unknown opcode");
}

#[inline(always)]
pub(crate) fn visit_local_get<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    local_depth: LocalDepth,
) -> Result<(), RwasmError> {
    let value = executor.store.sp.nth_back(local_depth.to_usize());
    executor.store.sp.push(value);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_local_set<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    local_depth: LocalDepth,
) -> Result<(), RwasmError> {
    let new_value = executor.store.sp.pop();
    executor
        .store
        .sp
        .set_nth_back(local_depth.to_usize(), new_value);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_local_tee<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    local_depth: LocalDepth,
) -> Result<(), RwasmError> {
    let new_value = executor.store.sp.last();
    executor
        .store
        .sp
        .set_nth_back(local_depth.to_usize(), new_value);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_br<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    branch_offset: BranchOffset,
) -> Result<(), RwasmError> {
    executor.branch_to(branch_offset);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_br_if_eqz<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    branch_offset: BranchOffset,
) -> Result<(), RwasmError> {
    let condition: bool = executor.store.sp.pop_as();
    if !condition {
        executor.branch_to(branch_offset);
    } else {
        executor.next_instr(1);
    }
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_br_if_nez<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    branch_offset: BranchOffset,
) -> Result<(), RwasmError> {
    let condition: bool = executor.store.sp.pop_as();
    if condition {
        executor.branch_to(branch_offset);
    } else {
        executor.next_instr(1);
    }
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_br_adjust<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    branch_offset: BranchOffset,
) -> Result<(), RwasmError> {
    let drop_keep = executor.fetch_drop_keep(1);
    executor.store.sp.drop_keep(drop_keep);
    executor.branch_to(branch_offset);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_br_adjust_if_nez<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    branch_offset: BranchOffset,
) -> Result<(), RwasmError> {
    let condition = executor.store.sp.pop_as();
    if condition {
        let drop_keep = executor.fetch_drop_keep(1);
        executor.store.sp.drop_keep(drop_keep);
        executor.branch_to(branch_offset);
    } else {
        executor.next_instr(2);
    }
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_br_table<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    targets: BranchTableTargets,
) -> Result<(), RwasmError> {
    let index: u32 = executor.store.sp.pop_as();
    let max_index = targets.to_usize() - 1;
    let normalized_index = cmp::min(index as usize, max_index);
    executor.branch_to((2 * normalized_index + 1) as i32);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_unreachable<E: SyscallHandler<T>, T>(
    _executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    Err(RwasmError::TrapCode(TrapCode::UnreachableCodeReached))
}

#[inline(always)]
pub(crate) fn visit_consume_fuel<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    block_fuel: BlockFuel,
) -> Result<(), RwasmError> {
    executor.store.try_consume_fuel(block_fuel.to_u64())?;
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_return<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    drop_keep: DropKeep,
) -> Result<(), RwasmError> {
    executor.store.sp.drop_keep(drop_keep);
    executor.store.value_stack.sync_stack_ptr(executor.store.sp);
    match executor.store.call_stack.pop() {
        Some(caller) => {
            executor.store.program_counter = caller;
            Ok(())
        }
        None => Err(RwasmError::ExecutionHalted(0)),
    }
}

#[inline(always)]
pub(crate) fn visit_return_if_nez<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    drop_keep: DropKeep,
) -> Result<(), RwasmError> {
    let condition = executor.store.sp.pop_as();
    if condition {
        executor.store.sp.drop_keep(drop_keep);
        executor.store.value_stack.sync_stack_ptr(executor.store.sp);
        match executor.store.call_stack.pop() {
            Some(caller) => {
                executor.store.program_counter = caller;
                Ok(())
            }
            None => Err(RwasmError::ExecutionHalted(0)),
        }
    } else {
        executor.next_instr(1);
        Ok(())
    }
}

#[inline(always)]
pub(crate) fn visit_return_call_internal<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    func_idx: CompiledFunc,
) -> Result<(), RwasmError> {
    let drop_keep = executor.fetch_drop_keep(1);
    executor.store.sp.drop_keep(drop_keep);
    executor.next_instr(2);
    executor.store.value_stack.sync_stack_ptr(executor.store.sp);
    let instr_ref = executor
        .store
        .module
        .func_segments
        .get(func_idx.to_u32() as usize)
        .copied()
        .expect("rwasm: unknown internal function");
    let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
    executor.store.value_stack.prepare_wasm_call(&header)?;
    executor.store.sp = executor.store.value_stack.stack_ptr();
    executor.call_to(instr_ref);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_return_call<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    func_idx: FuncIdx,
) -> Result<(), RwasmError> {
    let drop_keep = executor.fetch_drop_keep(1);
    executor.store.sp.drop_keep(drop_keep);
    executor.store.value_stack.sync_stack_ptr(executor.store.sp);
    // external call can cause interruption,
    // that is why it's important to increase IP before doing the call
    executor.next_instr(1);
    E::call_function(Caller::new(&mut executor.store), func_idx.to_u32())?;
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_return_call_indirect<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    signature_idx: SignatureIdx,
) -> Result<(), RwasmError> {
    let drop_keep = executor.fetch_drop_keep(1);
    let table = executor.fetch_table_index(2);
    let func_index: u32 = executor.store.sp.pop_as();
    executor.store.sp.drop_keep(drop_keep);
    executor.store.last_signature = Some(signature_idx);
    let func_idx: u32 = executor
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
    executor.execute_call_internal(false, 3, func_idx)
}

#[inline(always)]
pub(crate) fn visit_call_internal<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    func_idx: CompiledFunc,
) -> Result<(), RwasmError> {
    executor.store.value_stack.sync_stack_ptr(executor.store.sp);
    if executor.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
        return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
    }
    executor.next_instr(1);
    executor
        .store
        .call_stack
        .push(executor.store.program_counter);
    let instr_ref = executor
        .store
        .module
        .func_segments
        .get(func_idx.to_u32() as usize)
        .copied()
        .expect("rwasm: unknown internal function");
    let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
    executor.store.value_stack.prepare_wasm_call(&header)?;
    executor.store.sp = executor.store.value_stack.stack_ptr();
    executor.call_to(instr_ref);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_call<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    func_idx: FuncIdx,
) -> Result<(), RwasmError> {
    executor.store.value_stack.sync_stack_ptr(executor.store.sp);
    // external call can cause interruption,
    // that is why it's important to increase IP before doing the call
    executor.next_instr(1);
    E::call_function(Caller::new(&mut executor.store), func_idx.to_u32())
}

#[inline(always)]
pub(crate) fn visit_call_indirect<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    signature_idx: SignatureIdx,
) -> Result<(), RwasmError> {
    // resolve func index
    let table = executor.fetch_table_index(1);
    let func_index: u32 = executor.store.sp.pop_as();
    executor.store.last_signature = Some(signature_idx);
    let func_idx: u32 = executor
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
    executor.next_instr(2);
    executor.store.value_stack.sync_stack_ptr(executor.store.sp);
    if executor.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
        return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
    }
    executor
        .store
        .call_stack
        .push(executor.store.program_counter);
    let instr_ref = executor
        .store
        .module
        .func_segments
        .get(func_idx as usize)
        .copied()
        .expect("rwasm: unknown internal function");
    let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
    executor.store.value_stack.prepare_wasm_call(&header)?;
    executor.store.sp = executor.store.value_stack.stack_ptr();
    executor.call_to(instr_ref);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_signature_check<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    signature_idx: SignatureIdx,
) -> Result<(), RwasmError> {
    if let Some(actual_signature) = executor.store.last_signature.take() {
        if actual_signature != signature_idx {
            return Err(TrapCode::BadSignature).map_err(Into::into);
        }
    }
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_drop<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    executor.store.sp.drop();
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_select<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    executor.store.sp.eval_top3(|e1, e2, e3| {
        let condition = <bool as From<UntypedValue>>::from(e3);
        if condition {
            e1
        } else {
            e2
        }
    });
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_global_get<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    global_idx: GlobalIdx,
) -> Result<(), RwasmError> {
    let global_value = executor
        .store
        .global_variables
        .get(&global_idx)
        .copied()
        .unwrap_or_default();
    executor.store.sp.push(global_value);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_global_set<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    global_idx: GlobalIdx,
) -> Result<(), RwasmError> {
    let new_value = executor.store.sp.pop();
    executor
        .store
        .global_variables
        .insert(global_idx, new_value);
    executor.next_instr(1);
    Ok(())
}

impl_visit_load! {
    fn visit_i32_load(i32_load);
    fn visit_i64_load(i64_load);
    fn visit_f32_load(f32_load);
    fn visit_f64_load(f64_load);

    fn visit_i32_load8_s(i32_load8_s);
    fn visit_i32_load8_u(i32_load8_u);
    fn visit_i32_load16_s(i32_load16_s);
    fn visit_i32_load16_u(i32_load16_u);

    fn visit_i64_load8_s(i64_load8_s);
    fn visit_i64_load8_u(i64_load8_u);
    fn visit_i64_load16_s(i64_load16_s);
    fn visit_i64_load16_u(i64_load16_u);
    fn visit_i64_load32_s(i64_load32_s);
    fn visit_i64_load32_u(i64_load32_u);
}

impl_visit_store! {
    fn visit_i32_store(i32_store, 4);
    fn visit_i64_store(i64_store, 8);
    fn visit_f32_store(f32_store, 4);
    fn visit_f64_store(f64_store, 8);

    fn visit_i32_store8(i32_store8, 1);
    fn visit_i32_store16(i32_store16, 2);
    fn visit_i64_store8(i64_store8, 1);
    fn visit_i64_store16(i64_store16, 2);
    fn visit_i64_store32(i64_store32, 4);
}

#[inline(always)]
pub(crate) fn visit_memory_size<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let result: u32 = executor.store.global_memory.current_pages().into();
    executor.store.sp.push_as(result);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_grow<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let mut limiter = ResourceLimiterRef::default();
    let delta: u32 = executor.store.sp.pop_as();
    let delta = match Pages::new(delta) {
        Some(delta) => delta,
        None => {
            executor.store.sp.push_as(u32::MAX);
            return Ok(());
        }
    };
    if let Some(_) = executor.store.fuel_limit {
        let delta_in_bytes = delta.to_bytes().unwrap_or(0) as u64;
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_bytes(delta_in_bytes))?;
    }
    let new_pages = executor
        .store
        .global_memory
        .grow(delta, &mut limiter)
        .map(u32::from)
        .unwrap_or(u32::MAX);
    executor.store.sp.push_as(new_pages);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_fill<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let (d, val, n) = executor.store.sp.pop3();
    let n = i32::from(n) as usize;
    let offset = i32::from(d) as usize;
    let byte = u8::from(val);
    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_bytes(n as u64))?;
    }
    let memory = executor
        .store
        .global_memory
        .data_mut()
        .get_mut(offset..)
        .and_then(|memory| memory.get_mut(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    memory.fill(byte);
    if let Some(tracer) = executor.store.tracer.as_mut() {
        tracer.memory_change(offset as u32, n as u32, memory);
    }
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_copy<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
) -> Result<(), RwasmError> {
    let (d, s, n) = executor.store.sp.pop3();
    let n = i32::from(n) as usize;
    let src_offset = i32::from(s) as usize;
    let dst_offset = i32::from(d) as usize;
    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_bytes(n as u64))?;
    }
    // these accesses just perform the bound checks required by the Wasm spec.
    let data = executor.store.global_memory.data_mut();
    data.get(src_offset..)
        .and_then(|memory| memory.get(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    data.get(dst_offset..)
        .and_then(|memory| memory.get(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    data.copy_within(src_offset..src_offset.wrapping_add(n), dst_offset);
    if let Some(tracer) = executor.store.tracer.as_mut() {
        tracer.memory_change(
            dst_offset as u32,
            n as u32,
            &data[dst_offset..(dst_offset + n)],
        );
    }
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_memory_init<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    data_segment_idx: DataSegmentIdx,
) -> Result<(), RwasmError> {
    let is_empty_data_segment = executor.resolve_data_or_create(data_segment_idx).is_empty();
    let (d, s, n) = executor.store.sp.pop3();
    let n = i32::from(n) as usize;
    let src_offset = i32::from(s) as usize;
    let dst_offset = i32::from(d) as usize;
    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_bytes(n as u64))?;
    }
    let memory = executor
        .store
        .global_memory
        .data_mut()
        .get_mut(dst_offset..)
        .and_then(|memory| memory.get_mut(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    let mut memory_section = executor.store.module.memory_section.as_slice();
    if is_empty_data_segment {
        memory_section = &[];
    }
    let data = memory_section
        .get(src_offset..)
        .and_then(|data| data.get(..n))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    memory.copy_from_slice(data);
    if let Some(tracer) = executor.store.tracer.as_mut() {
        tracer.global_memory(dst_offset as u32, n as u32, memory);
    }
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_data_drop<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    data_segment_idx: DataSegmentIdx,
) -> Result<(), RwasmError> {
    let data_segment = executor.resolve_data_or_create(data_segment_idx);
    data_segment.drop_bytes();
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_size<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    table_idx: TableIdx,
) -> Result<(), RwasmError> {
    let table_size = executor
        .store
        .tables
        .get(&table_idx)
        .expect("rwasm: unresolved table segment")
        .size();
    executor.store.sp.push_as(table_size);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_grow<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    table_idx: TableIdx,
) -> Result<(), RwasmError> {
    let mut limiter = ResourceLimiterRef::default();
    let (init, delta) = executor.store.sp.pop2();
    let delta: u32 = delta.into();
    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_elements(delta as u64))?;
    }
    let table = executor.resolve_table_or_create(table_idx);
    let result = match table.grow_untyped(delta, init, &mut limiter) {
        Ok(result) => result,
        Err(EntityGrowError::TrapCode(trap_code)) => return Err(RwasmError::TrapCode(trap_code)),
        Err(EntityGrowError::InvalidGrow) => u32::MAX,
    };
    executor.store.sp.push_as(result);
    if let Some(tracer) = executor.store.tracer.as_mut() {
        tracer.table_size_change(table_idx.to_u32(), init.as_u32(), delta);
    }
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_fill<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    table_idx: TableIdx,
) -> Result<(), RwasmError> {
    let (i, val, n) = executor.store.sp.pop3();
    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_elements(n.as_u64()))?;
    }
    executor
        .resolve_table(table_idx)
        .fill_untyped(i.as_u32(), val, n.as_u32())?;
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_get<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    table_idx: TableIdx,
) -> Result<(), RwasmError> {
    let index = executor.store.sp.pop();
    let value = executor
        .resolve_table(table_idx)
        .get_untyped(index.as_u32())
        .ok_or(TrapCode::TableOutOfBounds)?;
    executor.store.sp.push(value);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_set<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    table_idx: TableIdx,
) -> Result<(), RwasmError> {
    let (index, value) = executor.store.sp.pop2();
    executor
        .resolve_table(table_idx)
        .set_untyped(index.as_u32(), value)
        .map_err(|_| TrapCode::TableOutOfBounds)?;
    if let Some(tracer) = executor.store.tracer.as_mut() {
        tracer.table_change(table_idx.to_u32(), index.as_u32(), value);
    }
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_copy<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    dst_table_idx: TableIdx,
) -> Result<(), RwasmError> {
    let src_table_idx = executor.fetch_table_index(1);
    let (d, s, n) = executor.store.sp.pop3();
    let len = u32::from(n);
    let src_index = u32::from(s);
    let dst_index = u32::from(d);
    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_elements(n.as_u64()))?;
    }
    // Query both tables and check if they are the same:
    if src_table_idx != dst_table_idx {
        let [src, dst] = executor
            .store
            .tables
            .get_many_mut([&src_table_idx, &dst_table_idx])
            .map(|v| v.expect("rwasm: unresolved table segment"));
        TableEntity::copy(dst, dst_index, src, src_index, len)?;
    } else {
        let src = executor
            .store
            .tables
            .get_mut(&src_table_idx)
            .expect("rwasm: unresolved table segment");
        src.copy_within(dst_index, src_index, len)?;
    }
    executor.next_instr(2);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_table_init<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    element_segment_idx: ElementSegmentIdx,
) -> Result<(), RwasmError> {
    let table_idx = executor.fetch_table_index(1);
    let (d, s, n) = executor.store.sp.pop3();
    let len = u32::from(n);
    let src_index = u32::from(s);
    let dst_index = u32::from(d);

    if let Some(_) = executor.store.fuel_limit {
        executor
            .store
            .try_consume_fuel(executor.store.fuel_costs.fuel_for_elements(len as u64))?;
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
    let is_empty_segment = executor
        .resolve_element_or_create(element_segment_idx)
        .is_empty();

    let (table, mut element) =
        executor.resolve_table_with_element_or_create(table_idx, ElementSegmentIdx::from(0));
    let mut empty_element_segment = ElementSegmentEntity::empty(element.ty());
    if is_empty_segment {
        element = &mut empty_element_segment;
    }
    table.init_untyped(dst_index, element, src_index, len)?;
    executor.next_instr(2);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_elem_drop<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    element_segment_idx: ElementSegmentIdx,
) -> Result<(), RwasmError> {
    let element_segment = executor.resolve_element_or_create(element_segment_idx);
    element_segment.drop_items();
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_ref_func<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    func_idx: FuncIdx,
) -> Result<(), RwasmError> {
    executor
        .store
        .sp
        .push_as(func_idx.to_u32() + FUNC_REF_OFFSET);
    executor.next_instr(1);
    Ok(())
}

#[inline(always)]
pub(crate) fn visit_i32_i64_const<E: SyscallHandler<T>, T>(
    executor: &mut RwasmExecutor<E, T>,
    untyped_value: UntypedValue,
) -> Result<(), RwasmError> {
    executor.store.sp.push(untyped_value);
    executor.next_instr(1);
    Ok(())
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
