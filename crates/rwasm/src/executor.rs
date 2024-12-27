use crate::executor::vec::Vec;
use alloc::vec;
use hashbrown::HashMap;
use rwasm::{
    core::{TrapCode, UntypedValue},
    engine::{
        bytecode::{AddressOffset, GlobalIdx, Instruction, SignatureIdx, TableIdx},
        code_map::InstructionPtr,
        stack::{ValueStack, ValueStackPtr},
        DropKeep,
        Tracer,
    },
    memory::MemoryEntity,
    rwasm::{BinaryFormatError, RwasmModule},
    store::ResourceLimiterRef,
    MemoryType,
};

#[derive(Debug, Copy, Clone)]
pub enum RwasmError {
    BinaryFormatError(BinaryFormatError),
    TrapCode(TrapCode),
    UnknownExternalFunction(u32),
    ExecutionHalted(i32),
}

impl From<BinaryFormatError> for RwasmError {
    fn from(value: BinaryFormatError) -> Self {
        Self::BinaryFormatError(value)
    }
}
impl From<TrapCode> for RwasmError {
    fn from(value: TrapCode) -> Self {
        Self::TrapCode(value)
    }
}

pub trait SyscallHandler {
    fn call_function(
        &mut self,
        func_idx: u32,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError>;
}

#[derive(Default)]
pub struct AlwaysFailingSyscallHandler;

impl SyscallHandler for AlwaysFailingSyscallHandler {
    fn call_function(
        &mut self,
        func_idx: u32,
        _sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        Err(RwasmError::UnknownExternalFunction(func_idx))
    }
}

pub struct RwasmExecutor<'a, E: SyscallHandler> {
    pub(crate) rwasm_module: RwasmModule,
    pub(crate) syscall_handler: Option<&'a mut E>,
    pub(crate) func_segments: Vec<u32>,
    pub(crate) sp: ValueStackPtr,
    pub(crate) value_stack: ValueStack,
    pub(crate) ip: InstructionPtr,
    pub(crate) resource_limiter_ref: ResourceLimiterRef<'a>,
    pub(crate) global_memory: MemoryEntity,
    pub(crate) global_variables: HashMap<GlobalIdx, UntypedValue>,
    pub(crate) global_tables: HashMap<(TableIdx, u32), u32>,
    pub(crate) call_stack: Vec<InstructionPtr>,
    pub(crate) last_signature: Option<SignatureIdx>,
    pub(crate) tracer: Option<&'a mut Tracer>,
}

impl<'a, E: SyscallHandler> RwasmExecutor<'a, E> {
    pub fn parse(
        rwasm_bytecode: &[u8],
        syscall_handler: Option<&'a mut E>,
    ) -> Result<Self, RwasmError> {
        let rwasm_module = RwasmModule::new(rwasm_bytecode)?;
        Ok(Self::new(rwasm_module, syscall_handler))
    }

    pub fn new(rwasm_module: RwasmModule, syscall_handler: Option<&'a mut E>) -> Self {
        let mut func_segments = vec![0u32];
        let mut total_func_len = 0u32;
        for func_len in rwasm_module
            .func_section
            .iter()
            .take(rwasm_module.func_section.len() - 1)
        {
            total_func_len += *func_len;
            func_segments.push(total_func_len);
        }
        let source_pc = func_segments
            .last()
            .copied()
            .expect("rwasm: empty function section");

        let mut value_stack = ValueStack::default();

        let mut resource_limiter_ref = ResourceLimiterRef::default();
        let global_memory = MemoryEntity::new(
            MemoryType::new(0, Some(1024)).expect("rwasm: bad initial memory"),
            &mut resource_limiter_ref,
        )
        .expect("rwasm: bad initial memory");

        let sp = value_stack.stack_ptr();
        let mut ip = InstructionPtr::new(
            rwasm_module.code_section.instr.as_ptr(),
            rwasm_module.code_section.metas.as_ptr(),
        );
        ip.add(source_pc as usize);

        let last_signature: Option<SignatureIdx> = None;

        Self {
            rwasm_module,
            syscall_handler,
            func_segments,
            sp,
            value_stack,
            ip,
            resource_limiter_ref,
            global_memory,
            global_variables: HashMap::new(),
            global_tables: HashMap::new(),
            call_stack: Vec::new(),
            last_signature,
            tracer: None,
        }
    }

    pub(crate) fn fetch_drop_keep(&self, offset: usize) -> DropKeep {
        let mut addr: InstructionPtr = self.ip;
        addr.add(offset);
        match addr.get() {
            Instruction::Return(drop_keep) => *drop_keep,
            _ => unreachable!("rwasm: can't extract drop keep"),
        }
    }

    pub(crate) fn fetch_table_index(&self, offset: usize) -> TableIdx {
        let mut addr: InstructionPtr = self.ip;
        addr.add(offset);
        match addr.get() {
            Instruction::TableGet(table_idx) => *table_idx,
            _ => unreachable!("rwasm: can't extract table index"),
        }
    }

    pub fn run(&mut self) -> Result<i32, RwasmError> {
        match self.run_the_loop() {
            Ok(exit_code) => Ok(exit_code),
            Err(err) => match err {
                RwasmError::ExecutionHalted(exit_code) => Ok(exit_code),
                _ => Err(err),
            },
        }
    }

    #[inline(always)]
    pub(crate) fn execute_load_extend(
        &mut self,
        offset: AddressOffset,
        load_extend: fn(
            memory: &[u8],
            address: UntypedValue,
            offset: u32,
        ) -> Result<UntypedValue, TrapCode>,
    ) -> Result<(), TrapCode> {
        self.sp.try_eval_top(|address| {
            let memory = self.global_memory.data();
            let value = load_extend(memory, address, offset.into_inner())?;
            Ok(value)
        })?;
        self.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_store_wrap(
        &mut self,
        offset: AddressOffset,
        store_wrap: fn(
            memory: &mut [u8],
            address: UntypedValue,
            offset: u32,
            value: UntypedValue,
        ) -> Result<(), TrapCode>,
        len: u32,
    ) -> Result<(), TrapCode> {
        let (address, value) = self.sp.pop2();
        let memory = self.global_memory.data_mut();
        store_wrap(memory, address, offset.into_inner(), value)?;
        self.ip.offset(0);
        let address = u32::from(address);
        let base_address = offset.into_inner() + address;
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.memory_change(
                base_address,
                len,
                &memory[base_address as usize..(base_address + len) as usize],
            );
        }
        self.ip.add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn execute_unary(&mut self, f: fn(UntypedValue) -> UntypedValue) {
        self.sp.eval_top(f);
        self.ip.add(1);
    }

    #[inline(always)]
    pub(crate) fn execute_binary(&mut self, f: fn(UntypedValue, UntypedValue) -> UntypedValue) {
        self.sp.eval_top2(f);
        self.ip.add(1);
    }
}

#[deprecated(since = "0.1.0", note = "use `RwasmExecutor::new` instead")]
pub fn execute_rwasm_module<'a, E: SyscallHandler>(
    rwasm_module: RwasmModule,
    syscall_handler: Option<&'a mut E>,
) -> Result<i32, RwasmError> {
    RwasmExecutor::<'a, E>::new(rwasm_module, syscall_handler).run()
}

#[derive(Default)]
pub struct SimpleCallHandler {
    pub input: Vec<u8>,
    pub state: u32,
    pub output: Vec<u8>,
}

impl SimpleCallHandler {
    fn fn_proc_exit(
        &self,
        sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let exit_code = sp.pop();
        Err(RwasmError::ExecutionHalted(exit_code.as_i32()))
    }

    fn fn_state(
        &self,
        sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        sp.push(UntypedValue::from(self.state));
        Ok(())
    }

    fn fn_write_output(
        &mut self,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let (offset, length) = sp.pop2();
        let buffer = global_memory
            .data()
            .get(offset.as_usize()..(offset.as_usize() + length.as_usize()))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        self.output.extend_from_slice(buffer);
        Ok(())
    }
}

impl SyscallHandler for SimpleCallHandler {
    fn call_function(
        &mut self,
        func_idx: u32,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        match func_idx {
            0x01 => self.fn_proc_exit(sp, global_memory),
            0x02 => self.fn_state(sp, global_memory),
            0x05 => self.fn_write_output(sp, global_memory),
            _ => unreachable!("rwasm: unknown function ({})", func_idx),
        }
    }
}
