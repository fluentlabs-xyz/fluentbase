use crate::{
    executor::RwasmExecutor,
    impl_visit_binary,
    impl_visit_fallible_binary,
    impl_visit_load,
    impl_visit_store,
    impl_visit_unary,
    Caller,
    RwasmError,
    SyscallHandler,
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
            Instruction,
            LocalDepth,
            SignatureIdx,
            TableIdx,
        },
        code_map::{FuncHeader, InstructionPtr, InstructionsRef},
        executor::EntityGrowError,
        CompiledFunc,
        DropKeep,
    },
    rwasm::N_MAX_RECURSION_DEPTH,
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
};

impl<E: SyscallHandler<T>, T> RwasmExecutor<E, T> {
    pub(crate) fn run_the_loop(&mut self) -> Result<i32, RwasmError> {
        let mut resource_limiter_ref = ResourceLimiterRef::default();
        loop {
            let instr = *self.store.ip.get();
            #[cfg(feature = "std")]
            {
                let stack = self
                    .store
                    .value_stack
                    .dump_stack(self.store.sp)
                    .iter()
                    .map(|v| v.as_u64())
                    .collect::<Vec<_>>();
                println!("{:02}: {:?}, stack={:?}", self.store.ip.pc(), instr, stack);
            }
            match instr {
                Instruction::LocalGet(local_depth) => self.visit_local_get(local_depth),
                Instruction::LocalSet(local_depth) => self.visit_local_set(local_depth),
                Instruction::LocalTee(local_depth) => self.visit_local_tee(local_depth),
                Instruction::Br(branch_offset) => self.visit_br(branch_offset),
                Instruction::BrIfEqz(branch_offset) => self.visit_br_if(branch_offset),
                Instruction::BrIfNez(branch_offset) => self.visit_br_if_nez(branch_offset),
                Instruction::BrAdjust(branch_offset) => self.visit_br_adjust(branch_offset),
                Instruction::BrAdjustIfNez(branch_offset) => {
                    self.visit_br_adjust_if_nez(branch_offset)
                }
                Instruction::BrTable(targets) => self.visit_br_table(targets),
                Instruction::Unreachable => {
                    return Err(RwasmError::TrapCode(TrapCode::UnreachableCodeReached));
                }
                Instruction::ConsumeFuel(block_fuel) => self.visit_consume_fuel(block_fuel)?,
                Instruction::Return(drop_keep) => {
                    if let Some(exit_code) = self.visit_return(drop_keep) {
                        return Ok(exit_code);
                    }
                }
                Instruction::ReturnIfNez(drop_keep) => {
                    if let Some(exit_code) = self.visit_return_if_nez(drop_keep) {
                        return Ok(exit_code);
                    }
                }
                Instruction::ReturnCallInternal(func_idx) => {
                    self.visit_return_call_internal(func_idx)?;
                }
                Instruction::ReturnCall(func_idx) => self.visit_return_call(func_idx)?,
                Instruction::ReturnCallIndirect(signature_idx) => {
                    self.visit_return_call_indirect(signature_idx)?
                }
                Instruction::CallInternal(func_idx) => self.visit_call_internal(func_idx)?,
                Instruction::Call(func_idx) => self.visit_call(func_idx)?,
                Instruction::CallIndirect(signature_idx) => {
                    self.visit_call_indirect(signature_idx)?
                }
                Instruction::SignatureCheck(signature_idx) => {
                    self.visit_signature_check(signature_idx)?
                }
                Instruction::Drop => self.visit_drop(),
                Instruction::Select => self.visit_select(),
                Instruction::GlobalGet(global_idx) => self.visit_global_get(global_idx),
                Instruction::GlobalSet(global_idx) => self.visit_global_set(global_idx),
                Instruction::I32Load(offset) => self.visit_i32_load(offset)?,
                Instruction::I64Load(offset) => self.visit_i64_load(offset)?,
                Instruction::F32Load(offset) => self.visit_f32_load(offset)?,
                Instruction::F64Load(offset) => self.visit_f64_load(offset)?,
                Instruction::I32Load8S(offset) => self.visit_i32_load_i8_s(offset)?,
                Instruction::I32Load8U(offset) => self.visit_i32_load_i8_u(offset)?,
                Instruction::I32Load16S(offset) => self.visit_i32_load_i16_s(offset)?,
                Instruction::I32Load16U(offset) => self.visit_i32_load_i16_u(offset)?,
                Instruction::I64Load8S(offset) => self.visit_i64_load_i8_s(offset)?,
                Instruction::I64Load8U(offset) => self.visit_i64_load_i8_u(offset)?,
                Instruction::I64Load16S(offset) => self.visit_i64_load_i16_s(offset)?,
                Instruction::I64Load16U(offset) => self.visit_i64_load_i16_u(offset)?,
                Instruction::I64Load32S(offset) => self.visit_i64_load_i32_s(offset)?,
                Instruction::I64Load32U(offset) => self.visit_i64_load_i32_u(offset)?,
                Instruction::I32Store(offset) => self.visit_i32_store(offset)?,
                Instruction::I64Store(offset) => self.visit_i64_store(offset)?,
                Instruction::F32Store(offset) => self.visit_f32_store(offset)?,
                Instruction::F64Store(offset) => self.visit_f64_store(offset)?,
                Instruction::I32Store8(offset) => self.visit_i32_store_8(offset)?,
                Instruction::I32Store16(offset) => self.visit_i32_store_16(offset)?,
                Instruction::I64Store8(offset) => self.visit_i64_store_8(offset)?,
                Instruction::I64Store16(offset) => self.visit_i64_store_16(offset)?,
                Instruction::I64Store32(offset) => self.visit_i64_store_32(offset)?,
                Instruction::MemorySize => self.visit_memory_size(),
                Instruction::MemoryGrow => self.visit_memory_grow(&mut resource_limiter_ref)?,
                Instruction::MemoryFill => self.visit_memory_fill()?,
                Instruction::MemoryCopy => self.visit_memory_copy()?,

                Instruction::MemoryInit(segment) => self.visit_memory_init(segment)?,
                Instruction::DataDrop(segment) => self.visit_data_drop(segment),
                Instruction::TableSize(table) => self.visit_table_size(table),
                Instruction::TableGrow(table) => {
                    self.visit_table_grow(table, &mut resource_limiter_ref)?
                }
                Instruction::TableFill(table) => self.visit_table_fill(table)?,
                Instruction::TableGet(table) => self.visit_table_get(table)?,
                Instruction::TableSet(table) => self.visit_table_set(table)?,
                Instruction::TableCopy(dst) => self.visit_table_copy(dst)?,
                Instruction::TableInit(elem) => self.visit_table_init(elem)?,
                Instruction::ElemDrop(segment) => self.visit_element_drop(segment),
                Instruction::RefFunc(func_index) => self.visit_ref_func(func_index)?,
                Instruction::I32Const(value)
                | Instruction::I64Const(value)
                | Instruction::F32Const(value)
                | Instruction::F64Const(value) => self.visit_i32_i64_const(value),

                // Instruction::ConstRef(cref) => self.visit_const(cref),
                Instruction::I32Eqz => self.visit_i32_eqz(),
                Instruction::I32Eq => self.visit_i32_eq(),
                Instruction::I32Ne => self.visit_i32_ne(),
                Instruction::I32LtS => self.visit_i32_lt_s(),
                Instruction::I32LtU => self.visit_i32_lt_u(),
                Instruction::I32GtS => self.visit_i32_gt_s(),
                Instruction::I32GtU => self.visit_i32_gt_u(),
                Instruction::I32LeS => self.visit_i32_le_s(),
                Instruction::I32LeU => self.visit_i32_le_u(),
                Instruction::I32GeS => self.visit_i32_ge_s(),
                Instruction::I32GeU => self.visit_i32_ge_u(),
                Instruction::I64Eqz => self.visit_i64_eqz(),
                Instruction::I64Eq => self.visit_i64_eq(),
                Instruction::I64Ne => self.visit_i64_ne(),
                Instruction::I64LtS => self.visit_i64_lt_s(),
                Instruction::I64LtU => self.visit_i64_lt_u(),
                Instruction::I64GtS => self.visit_i64_gt_s(),
                Instruction::I64GtU => self.visit_i64_gt_u(),
                Instruction::I64LeS => self.visit_i64_le_s(),
                Instruction::I64LeU => self.visit_i64_le_u(),
                Instruction::I64GeS => self.visit_i64_ge_s(),
                Instruction::I64GeU => self.visit_i64_ge_u(),
                Instruction::F32Eq => self.visit_f32_eq(),
                Instruction::F32Ne => self.visit_f32_ne(),
                Instruction::F32Lt => self.visit_f32_lt(),
                Instruction::F32Gt => self.visit_f32_gt(),
                Instruction::F32Le => self.visit_f32_le(),
                Instruction::F32Ge => self.visit_f32_ge(),
                Instruction::F64Eq => self.visit_f64_eq(),
                Instruction::F64Ne => self.visit_f64_ne(),
                Instruction::F64Lt => self.visit_f64_lt(),
                Instruction::F64Gt => self.visit_f64_gt(),
                Instruction::F64Le => self.visit_f64_le(),
                Instruction::F64Ge => self.visit_f64_ge(),
                Instruction::I32Clz => self.visit_i32_clz(),
                Instruction::I32Ctz => self.visit_i32_ctz(),
                Instruction::I32Popcnt => self.visit_i32_popcnt(),
                Instruction::I32Add => self.visit_i32_add(),
                Instruction::I32Sub => self.visit_i32_sub(),
                Instruction::I32Mul => self.visit_i32_mul(),
                Instruction::I32DivS => self.visit_i32_div_s()?,
                Instruction::I32DivU => self.visit_i32_div_u()?,
                Instruction::I32RemS => self.visit_i32_rem_s()?,
                Instruction::I32RemU => self.visit_i32_rem_u()?,
                Instruction::I32And => self.visit_i32_and(),
                Instruction::I32Or => self.visit_i32_or(),
                Instruction::I32Xor => self.visit_i32_xor(),
                Instruction::I32Shl => self.visit_i32_shl(),
                Instruction::I32ShrS => self.visit_i32_shr_s(),
                Instruction::I32ShrU => self.visit_i32_shr_u(),
                Instruction::I32Rotl => self.visit_i32_rotl(),
                Instruction::I32Rotr => self.visit_i32_rotr(),
                Instruction::I64Clz => self.visit_i64_clz(),
                Instruction::I64Ctz => self.visit_i64_ctz(),
                Instruction::I64Popcnt => self.visit_i64_popcnt(),
                Instruction::I64Add => self.visit_i64_add(),
                Instruction::I64Sub => self.visit_i64_sub(),
                Instruction::I64Mul => self.visit_i64_mul(),
                Instruction::I64DivS => self.visit_i64_div_s()?,
                Instruction::I64DivU => self.visit_i64_div_u()?,
                Instruction::I64RemS => self.visit_i64_rem_s()?,
                Instruction::I64RemU => self.visit_i64_rem_u()?,
                Instruction::I64And => self.visit_i64_and(),
                Instruction::I64Or => self.visit_i64_or(),
                Instruction::I64Xor => self.visit_i64_xor(),
                Instruction::I64Shl => self.visit_i64_shl(),
                Instruction::I64ShrS => self.visit_i64_shr_s(),
                Instruction::I64ShrU => self.visit_i64_shr_u(),
                Instruction::I64Rotl => self.visit_i64_rotl(),
                Instruction::I64Rotr => self.visit_i64_rotr(),
                Instruction::F32Abs => self.visit_f32_abs(),
                Instruction::F32Neg => self.visit_f32_neg(),
                Instruction::F32Ceil => self.visit_f32_ceil(),
                Instruction::F32Floor => self.visit_f32_floor(),
                Instruction::F32Trunc => self.visit_f32_trunc(),
                Instruction::F32Nearest => self.visit_f32_nearest(),
                Instruction::F32Sqrt => self.visit_f32_sqrt(),
                Instruction::F32Add => self.visit_f32_add(),
                Instruction::F32Sub => self.visit_f32_sub(),
                Instruction::F32Mul => self.visit_f32_mul(),
                Instruction::F32Div => self.visit_f32_div(),
                Instruction::F32Min => self.visit_f32_min(),
                Instruction::F32Max => self.visit_f32_max(),
                Instruction::F32Copysign => self.visit_f32_copysign(),
                Instruction::F64Abs => self.visit_f64_abs(),
                Instruction::F64Neg => self.visit_f64_neg(),
                Instruction::F64Ceil => self.visit_f64_ceil(),
                Instruction::F64Floor => self.visit_f64_floor(),
                Instruction::F64Trunc => self.visit_f64_trunc(),
                Instruction::F64Nearest => self.visit_f64_nearest(),
                Instruction::F64Sqrt => self.visit_f64_sqrt(),
                Instruction::F64Add => self.visit_f64_add(),
                Instruction::F64Sub => self.visit_f64_sub(),
                Instruction::F64Mul => self.visit_f64_mul(),
                Instruction::F64Div => self.visit_f64_div(),
                Instruction::F64Min => self.visit_f64_min(),
                Instruction::F64Max => self.visit_f64_max(),
                Instruction::F64Copysign => self.visit_f64_copysign(),
                Instruction::I32WrapI64 => self.visit_i32_wrap_i64(),
                // Instruction::I32TruncF32S => self.visit_i32_trunc_f32_s()?,
                // Instruction::I32TruncF32U => self.visit_i32_trunc_f32_u()?,
                // Instruction::I32TruncF64S => self.visit_i32_trunc_f64_s()?,
                // Instruction::I32TruncF64U => self.visit_i32_trunc_f64_u()?,
                Instruction::I64ExtendI32S => self.visit_i64_extend_i32_s(),
                Instruction::I64ExtendI32U => self.visit_i64_extend_i32_u(),
                // Instruction::I64TruncF32S => self.visit_i64_trunc_f32_s()?,
                // Instruction::I64TruncF32U => self.visit_i64_trunc_f32_u()?,
                // Instruction::I64TruncF64S => self.visit_i64_trunc_f64_s()?,
                // Instruction::I64TruncF64U => self.visit_i64_trunc_f64_u()?,
                Instruction::F32ConvertI32S => self.visit_f32_convert_i32_s(),
                Instruction::F32ConvertI32U => self.visit_f32_convert_i32_u(),
                Instruction::F32ConvertI64S => self.visit_f32_convert_i64_s(),
                Instruction::F32ConvertI64U => self.visit_f32_convert_i64_u(),
                Instruction::F32DemoteF64 => self.visit_f32_demote_f64(),
                Instruction::F64ConvertI32S => self.visit_f64_convert_i32_s(),
                Instruction::F64ConvertI32U => self.visit_f64_convert_i32_u(),
                Instruction::F64ConvertI64S => self.visit_f64_convert_i64_s(),
                Instruction::F64ConvertI64U => self.visit_f64_convert_i64_u(),
                Instruction::F64PromoteF32 => self.visit_f64_promote_f32(),
                Instruction::I32TruncSatF32S => self.visit_i32_trunc_sat_f32_s(),
                Instruction::I32TruncSatF32U => self.visit_i32_trunc_sat_f32_u(),
                Instruction::I32TruncSatF64S => self.visit_i32_trunc_sat_f64_s(),
                Instruction::I32TruncSatF64U => self.visit_i32_trunc_sat_f64_u(),
                Instruction::I64TruncSatF32S => self.visit_i64_trunc_sat_f32_s(),
                Instruction::I64TruncSatF32U => self.visit_i64_trunc_sat_f32_u(),
                Instruction::I64TruncSatF64S => self.visit_i64_trunc_sat_f64_s(),
                Instruction::I64TruncSatF64U => self.visit_i64_trunc_sat_f64_u(),
                Instruction::I32Extend8S => self.visit_i32_extend8_s(),
                Instruction::I32Extend16S => self.visit_i32_extend16_s(),
                Instruction::I64Extend8S => self.visit_i64_extend8_s(),
                Instruction::I64Extend16S => self.visit_i64_extend16_s(),
                Instruction::I64Extend32S => self.visit_i64_extend32_s(),

                _ => unreachable!("rwasm: unsupported instruction ({:?})", instr),
            }
        }
    }

    #[inline(always)]
    pub(crate) fn visit_local_get(&mut self, local_depth: LocalDepth) {
        let value = self.store.sp.nth_back(local_depth.to_usize());
        self.store.sp.push(value);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_local_set(&mut self, local_depth: LocalDepth) {
        let new_value = self.store.sp.pop();
        self.store
            .sp
            .set_nth_back(local_depth.to_usize(), new_value);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_local_tee(&mut self, local_depth: LocalDepth) {
        let new_value = self.store.sp.last();
        self.store
            .sp
            .set_nth_back(local_depth.to_usize(), new_value);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_br(&mut self, branch_offset: BranchOffset) {
        self.store.ip.offset(branch_offset.to_i32() as isize)
    }

    #[inline(always)]
    pub(crate) fn visit_br_if(&mut self, branch_offset: BranchOffset) {
        let condition = self.store.sp.pop_as();
        if condition {
            self.store.ip.add(1);
        } else {
            self.store.ip.offset(branch_offset.to_i32() as isize);
        }
    }

    #[inline(always)]
    pub(crate) fn visit_br_if_nez(&mut self, branch_offset: BranchOffset) {
        let condition = self.store.sp.pop_as();
        if condition {
            self.store.ip.offset(branch_offset.to_i32() as isize);
        } else {
            self.store.ip.add(1);
        }
    }

    #[inline(always)]
    pub(crate) fn visit_br_adjust(&mut self, branch_offset: BranchOffset) {
        let drop_keep = self.fetch_drop_keep(1);
        self.store.sp.drop_keep(drop_keep);
        self.store.ip.offset(branch_offset.to_i32() as isize);
    }

    #[inline(always)]
    pub(crate) fn visit_br_adjust_if_nez(&mut self, branch_offset: BranchOffset) {
        let condition = self.store.sp.pop_as();
        if condition {
            let drop_keep = self.fetch_drop_keep(1);
            self.store.sp.drop_keep(drop_keep);
            self.store.ip.offset(branch_offset.to_i32() as isize);
        } else {
            self.store.ip.add(2);
        }
    }

    #[inline(always)]
    pub(crate) fn visit_br_table(&mut self, targets: BranchTableTargets) {
        let index: u32 = self.store.sp.pop_as();
        let max_index = targets.to_usize() - 1;
        let normalized_index = cmp::min(index as usize, max_index);
        self.store.ip.add(2 * normalized_index + 1);
    }

    #[inline(always)]
    pub(crate) fn visit_consume_fuel(&mut self, block_fuel: BlockFuel) -> Result<(), RwasmError> {
        self.store.try_consume_fuel(block_fuel.to_u64())?;
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_return(&mut self, drop_keep: DropKeep) -> Option<i32> {
        self.store.sp.drop_keep(drop_keep);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        match self.store.call_stack.pop() {
            Some(caller) => {
                self.store.ip = caller;
                None
            }
            None => Some(0),
        }
    }

    #[inline(always)]
    pub(crate) fn visit_return_if_nez(&mut self, drop_keep: DropKeep) -> Option<i32> {
        let condition = self.store.sp.pop_as();
        if condition {
            self.store.sp.drop_keep(drop_keep);
            self.store.value_stack.sync_stack_ptr(self.store.sp);
            match self.store.call_stack.pop() {
                Some(caller) => {
                    self.store.ip = caller;
                    None
                }
                None => Some(0),
            }
        } else {
            self.store.ip.add(1);
            None
        }
    }

    #[inline(always)]
    pub(crate) fn visit_return_call_internal(
        &mut self,
        func_idx: CompiledFunc,
    ) -> Result<(), RwasmError> {
        let drop_keep = self.fetch_drop_keep(1);
        self.store.sp.drop_keep(drop_keep);
        self.store.ip.add(2);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        let instr_ref = self
            .store
            .instance
            .func_segments
            .get(func_idx.to_u32() as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.store.value_stack.prepare_wasm_call(&header)?;
        self.store.sp = self.store.value_stack.stack_ptr();
        self.store.ip = InstructionPtr::new(
            self.store.instance.module.code_section.instr.as_ptr(),
            self.store.instance.module.code_section.metas.as_ptr(),
        );
        self.store.ip.add(instr_ref as usize);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_return_call(&mut self, func_idx: FuncIdx) -> Result<(), RwasmError> {
        let drop_keep = self.fetch_drop_keep(1);
        self.store.sp.drop_keep(drop_keep);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        // external call can cause interruption,
        // that is why it's important to increase IP before doing the call
        self.store.ip.add(2);
        E::call_function(Caller::new(&mut self.store), func_idx.to_u32())?;
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_return_call_indirect(
        &mut self,
        signature_idx: SignatureIdx,
    ) -> Result<(), RwasmError> {
        let drop_keep = self.fetch_drop_keep(1);
        let table = self.fetch_table_index(2);
        let func_index: u32 = self.store.sp.pop_as();
        self.store.sp.drop_keep(drop_keep);
        self.store.last_signature = Some(signature_idx);
        let func_idx: u32 = self
            .store
            .tables
            .get(&table)
            .expect("rwasm: unresolved table index")
            .get(func_index)
            .and_then(|v| v.i32())
            .ok_or(TrapCode::TableOutOfBounds)?
            .try_into()
            .unwrap();
        // if func_idx == 0 {
        //     return Err(TrapCode::IndirectCallToNull.into());
        // }
        self.execute_call_internal(false, 3, func_idx)
    }

    #[inline(always)]
    pub(crate) fn visit_call_internal(&mut self, func_idx: CompiledFunc) -> Result<(), RwasmError> {
        self.store.ip.add(1);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        if self.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
            return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
        }
        self.store.call_stack.push(self.store.ip);
        let instr_ref = self
            .store
            .instance
            .func_segments
            .get(func_idx.to_u32() as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.store.value_stack.prepare_wasm_call(&header)?;
        self.store.sp = self.store.value_stack.stack_ptr();
        self.store.ip = InstructionPtr::new(
            self.store.instance.module.code_section.instr.as_ptr(),
            self.store.instance.module.code_section.metas.as_ptr(),
        );
        self.store.ip.add(instr_ref as usize);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_call(&mut self, func_idx: FuncIdx) -> Result<(), RwasmError> {
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        // external call can cause interruption,
        // that is why it's important to increase IP before doing the call
        self.store.ip.add(1);
        E::call_function(Caller::new(&mut self.store), func_idx.to_u32())
    }

    #[inline(always)]
    pub(crate) fn visit_call_indirect(
        &mut self,
        signature_idx: SignatureIdx,
    ) -> Result<(), RwasmError> {
        // resolve func index
        let table = self.fetch_table_index(1);
        let func_index: u32 = self.store.sp.pop_as();
        self.store.last_signature = Some(signature_idx);
        let func_idx: u32 = self
            .store
            .tables
            .get(&table)
            .expect("rwasm: unresolved table index")
            .get(func_index)
            .and_then(|v| v.i32())
            .ok_or(TrapCode::TableOutOfBounds)?
            .try_into()
            .expect("rwasm: invalid function index");
        // if func_idx == 0 {
        //     return Err(TrapCode::IndirectCallToNull.into());
        // }
        // call func
        self.store.ip.add(2);
        self.store.value_stack.sync_stack_ptr(self.store.sp);
        if self.store.call_stack.len() > N_MAX_RECURSION_DEPTH {
            return Err(RwasmError::TrapCode(TrapCode::StackOverflow));
        }
        self.store.call_stack.push(self.store.ip);
        let instr_ref = self
            .store
            .instance
            .func_segments
            .get(func_idx as usize)
            .copied()
            .expect("rwasm: unknown internal function");
        let header = FuncHeader::new(InstructionsRef::uninit(), 0, 0);
        self.store.value_stack.prepare_wasm_call(&header)?;
        self.store.sp = self.store.value_stack.stack_ptr();
        self.store.ip = InstructionPtr::new(
            self.store.instance.module.code_section.instr.as_ptr(),
            self.store.instance.module.code_section.metas.as_ptr(),
        );
        self.store.ip.add(instr_ref as usize);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_signature_check(
        &mut self,
        signature_idx: SignatureIdx,
    ) -> Result<(), RwasmError> {
        if let Some(actual_signature) = self.store.last_signature.take() {
            if actual_signature != signature_idx {
                return Err(TrapCode::BadSignature).map_err(Into::into);
            }
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_drop(&mut self) {
        self.store.sp.drop();
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_select(&mut self) {
        self.store.sp.eval_top3(|e1, e2, e3| {
            let condition = <bool as From<UntypedValue>>::from(e3);
            if condition {
                e1
            } else {
                e2
            }
        });
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_global_get(&mut self, global_idx: GlobalIdx) {
        let global_value = self
            .store
            .global_variables
            .get(&global_idx)
            .copied()
            .unwrap_or_default();
        self.store.sp.push(global_value);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_global_set(&mut self, global_idx: GlobalIdx) {
        let new_value = self.store.sp.pop();
        self.store.global_variables.insert(global_idx, new_value);
        self.store.ip.add(1);
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
    pub(crate) fn visit_memory_size(&mut self) {
        let result: u32 = self.store.global_memory.current_pages().into();
        self.store.sp.push_as(result);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_memory_grow(
        &mut self,
        mut limiter: &mut ResourceLimiterRef<'_>,
    ) -> Result<(), RwasmError> {
        let delta: u32 = self.store.sp.pop_as();
        let delta = match Pages::new(delta) {
            Some(delta) => delta,
            None => {
                self.store.sp.push_as(u32::MAX);
                self.store.ip.add(1);
                return Ok(());
            }
        };
        if let Some(_) = self.store.fuel_limit {
            let delta_in_bytes = delta.to_bytes().unwrap_or(0) as u64;
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_bytes(delta_in_bytes))?;
        }
        let new_pages = self
            .store
            .global_memory
            .grow(delta, &mut limiter)
            .map(u32::from)
            .unwrap_or(u32::MAX);
        self.store.sp.push_as(new_pages);
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_memory_fill(&mut self) -> Result<(), RwasmError> {
        let (d, val, n) = self.store.sp.pop3();
        let n = i32::from(n) as usize;
        let offset = i32::from(d) as usize;
        let byte = u8::from(val);
        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_bytes(n as u64))?;
        }
        let memory = self
            .store
            .global_memory
            .data_mut()
            .get_mut(offset..)
            .and_then(|memory| memory.get_mut(..n))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        memory.fill(byte);
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.memory_change(offset as u32, n as u32, memory);
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_memory_copy(&mut self) -> Result<(), RwasmError> {
        let (d, s, n) = self.store.sp.pop3();
        let n = i32::from(n) as usize;
        let src_offset = i32::from(s) as usize;
        let dst_offset = i32::from(d) as usize;
        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_bytes(n as u64))?;
        }
        // these accesses just perform the bound checks required by the Wasm spec.
        let data = self.store.global_memory.data_mut();
        data.get(src_offset..)
            .and_then(|memory| memory.get(..n))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        data.get(dst_offset..)
            .and_then(|memory| memory.get(..n))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        data.copy_within(src_offset..src_offset.wrapping_add(n), dst_offset);
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.memory_change(
                dst_offset as u32,
                n as u32,
                &data[dst_offset..(dst_offset + n)],
            );
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_memory_init(
        &mut self,
        data_segment_idx: DataSegmentIdx,
    ) -> Result<(), RwasmError> {
        let is_empty_data_segment = self.resolve_data_or_create(data_segment_idx).is_empty();
        let (d, s, n) = self.store.sp.pop3();
        let n = i32::from(n) as usize;
        let src_offset = i32::from(s) as usize;
        let dst_offset = i32::from(d) as usize;
        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_bytes(n as u64))?;
        }
        let memory = self
            .store
            .global_memory
            .data_mut()
            .get_mut(dst_offset..)
            .and_then(|memory| memory.get_mut(..n))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        let mut memory_section = self.store.instance.module.memory_section.as_slice();
        if is_empty_data_segment {
            memory_section = &[];
        }
        let data = memory_section
            .get(src_offset..)
            .and_then(|data| data.get(..n))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        memory.copy_from_slice(data);
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.global_memory(dst_offset as u32, n as u32, memory);
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_data_drop(&mut self, data_segment_idx: DataSegmentIdx) {
        let data_segment = self.resolve_data_or_create(data_segment_idx);
        data_segment.drop_bytes();
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_table_size(&mut self, table_idx: TableIdx) {
        let table_size = self
            .store
            .tables
            .get(&table_idx)
            .expect("rwasm: unresolved table segment")
            .size();
        self.store.sp.push_as(table_size);
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_table_grow(
        &mut self,
        table_idx: TableIdx,
        limiter: &mut ResourceLimiterRef<'_>,
    ) -> Result<(), RwasmError> {
        let (init, delta) = self.store.sp.pop2();
        let delta: u32 = delta.into();
        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_elements(delta as u64))?;
        }
        let table = self.resolve_table_or_create(table_idx);
        let result = match table.grow_untyped(delta, init, limiter) {
            Ok(result) => result,
            Err(EntityGrowError::TrapCode(trap_code)) => {
                return Err(RwasmError::TrapCode(trap_code))
            }
            Err(EntityGrowError::InvalidGrow) => u32::MAX,
        };
        self.store.sp.push_as(result);
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.table_size_change(table_idx.to_u32(), init.as_u32(), delta);
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_table_fill(&mut self, table_idx: TableIdx) -> Result<(), RwasmError> {
        let (i, val, n) = self.store.sp.pop3();
        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_elements(n.as_u64()))?;
        }
        self.resolve_table_or_create(table_idx)
            .fill_untyped(i.as_u32(), val, n.as_u32())?;
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_table_get(&mut self, table_idx: TableIdx) -> Result<(), RwasmError> {
        let index = self.store.sp.pop();
        let value = self
            .resolve_table_or_create(table_idx)
            .get_untyped(index.as_u32())
            .ok_or(TrapCode::TableOutOfBounds)?;
        self.store.sp.push(value);
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_table_set(&mut self, table_idx: TableIdx) -> Result<(), RwasmError> {
        let (index, value) = self.store.sp.pop2();
        self.resolve_table_or_create(table_idx)
            .set_untyped(index.as_u32(), value)
            .map_err(|_| TrapCode::TableOutOfBounds)?;
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.table_change(table_idx.to_u32(), index.as_u32(), value);
        }
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_table_copy(&mut self, dst_table_idx: TableIdx) -> Result<(), RwasmError> {
        let src_table_idx = self.fetch_table_index(1);
        let (d, s, n) = self.store.sp.pop3();
        let len = u32::from(n);
        let src_index = u32::from(s);
        let dst_index = u32::from(d);
        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_elements(n.as_u64()))?;
        }
        // Query both tables and check if they are the same:
        if src_table_idx != dst_table_idx {
            let [src, dst] = self
                .store
                .tables
                .get_many_mut([&src_table_idx, &dst_table_idx])
                .map(|v| v.expect("rwasm: unresolved table segment"));
            TableEntity::copy(dst, dst_index, src, src_index, len)?;
        } else {
            let src = self
                .store
                .tables
                .get_mut(&src_table_idx)
                .expect("rwasm: unresolved table segment");
            src.copy_within(dst_index, src_index, len)?;
        }
        self.store.ip.add(2);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_table_init(
        &mut self,
        element_segment_idx: ElementSegmentIdx,
    ) -> Result<(), RwasmError> {
        let table_idx = self.fetch_table_index(1);
        let (d, s, n) = self.store.sp.pop3();
        let len = u32::from(n);
        let src_index = u32::from(s);
        let dst_index = u32::from(d);

        if let Some(_) = self.store.fuel_limit {
            self.store
                .try_consume_fuel(self.store.fuel_costs.fuel_for_elements(len as u64))?;
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
        let element = self.resolve_element_or_create(element_segment_idx);
        let is_empty_segment = element.is_empty();

        let (table, mut element) =
            self.resolve_table_with_element_or_create(table_idx, ElementSegmentIdx::from(0));
        let mut empty_element_segment = ElementSegmentEntity::empty(element.ty());
        if is_empty_segment {
            element = &mut empty_element_segment;
        }
        table.init_untyped(dst_index, element, src_index, len)?;
        self.store.ip.add(2);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_element_drop(&mut self, element_segment_idx: ElementSegmentIdx) {
        let element_segment = self.resolve_element_or_create(element_segment_idx);
        element_segment.drop_items();
        self.store.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn visit_ref_func(&mut self, func_idx: FuncIdx) -> Result<(), RwasmError> {
        self.store.sp.push_as(func_idx.to_u32());
        self.store.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn visit_i32_i64_const(&mut self, untyped_value: UntypedValue) {
        self.store.sp.push(untyped_value);
        self.store.ip.add(1);
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
}
