use crate::{
    config::ExecutorConfig,
    RwasmError,
    FUNC_REF_OFFSET,
    N_DEFAULT_STACK_SIZE,
    N_MAX_STACK_SIZE,
};
use hashbrown::HashMap;
use rwasm::{
    core::{TrapCode, UntypedValue, ValueType, N_MAX_MEMORY_PAGES},
    engine::{
        bytecode::{DataSegmentIdx, ElementSegmentIdx, GlobalIdx, SignatureIdx, TableIdx},
        code_map::InstructionPtr,
        stack::{ValueStack, ValueStackPtr},
        FuelCosts,
        Tracer,
    },
    memory::{DataSegmentEntity, MemoryEntity},
    module::{ConstExpr, ElementSegment, ElementSegmentItems, ElementSegmentKind},
    rwasm::RwasmModuleInstance,
    store::ResourceLimiterRef,
    table::{ElementSegmentEntity, TableEntity},
    MemoryType,
};
use rwasm::engine::bytecode::Instruction;
use rwasm::rwasm::instruction::InstructionExtra;

pub struct RwasmContext<T> {
    // function segments
    pub(crate) instance: RwasmModuleInstance,
    pub(crate) config: ExecutorConfig,
    // execution context information
    pub(crate) consumed_fuel: u64,
    pub(crate) value_stack: ValueStack,
    pub(crate) sp: ValueStackPtr,
    pub(crate) global_memory: MemoryEntity,
    pub(crate) ip: InstructionPtr,
    pub(crate) context: T,
    pub(crate) tracer: Option<Tracer>,
    pub(crate) fuel_costs: FuelCosts,
    // rwasm modified segments
    pub(crate) global_variables: HashMap<GlobalIdx, UntypedValue>,
    pub(crate) tables: HashMap<TableIdx, TableEntity>,
    pub(crate) data_segments: HashMap<DataSegmentIdx, DataSegmentEntity>,
    pub(crate) elements: HashMap<ElementSegmentIdx, ElementSegmentEntity>,
    // list of nested calls return pointers
    pub(crate) call_stack: Vec<InstructionPtr>,
    // the last used signature (needed for indirect calls type checks)
    pub(crate) last_signature: Option<SignatureIdx>,
    // DTC
    pub(crate) use_dtc: bool,           // Флаг для переключения на DTC
    pub(crate) dtc_code: Vec<usize>,    // Массив DTC-кода
    pub(crate) dtc_ip: usize,           // Указатель на текущую позицию в dtc_code
}

impl<T> RwasmContext<T> {
    pub fn new(instance: RwasmModuleInstance, config: ExecutorConfig, context: T, use_dtc: bool) -> Self {
        // create stack with sp
        let mut value_stack = ValueStack::new(N_DEFAULT_STACK_SIZE, N_MAX_STACK_SIZE);
        let sp = value_stack.stack_ptr();

        // assign sp to the position inside code section
        let mut ip = InstructionPtr::new(
            instance.module.code_section.instr.as_ptr(),
            instance.module.code_section.metas.as_ptr(),
        );
        ip.add(instance.start);

        // create global memory
        let mut resource_limiter_ref = ResourceLimiterRef::default();
        let global_memory = MemoryEntity::new(
            MemoryType::new(0, Some(N_MAX_MEMORY_PAGES)).expect("rwasm: bad initial memory"),
            &mut resource_limiter_ref,
        )
        .expect("rwasm: bad initial memory");

        // create the main element segment (index 0) from the module elements
        let mut element_segments = HashMap::new();
        element_segments.insert(
            ElementSegmentIdx::from(0u32),
            ElementSegmentEntity::from(&ElementSegment {
                kind: ElementSegmentKind::Passive,
                ty: ValueType::I32,
                items: ElementSegmentItems {
                    exprs: instance
                        .module
                        .element_section
                        .iter()
                        .map(|v| ConstExpr::from_const((*v + FUNC_REF_OFFSET).into()))
                        .collect(),
                },
            }),
        );

        let tracer = if config.tracer_enabled {
            Some(Tracer::default())
        } else {
            None
        };

        let mut context = Self {
            instance,
            config,
            consumed_fuel: 0,
            value_stack,
            sp,
            global_memory,
            ip,
            context,
            tracer,
            fuel_costs: Default::default(),
            global_variables: Default::default(),
            tables: Default::default(),
            data_segments: Default::default(),
            elements: element_segments,
            call_stack: vec![],
            last_signature: None,
            use_dtc,
            dtc_code: vec![],
            dtc_ip: 0,
        };

        // Если DTC включен, преобразуем инструкции в dtc_code
        if use_dtc {
            context.instantiate_dtc();
        }

        context
    }

    // Метод для преобразования инструкций в DTC-код
    fn instantiate_dtc(&mut self) {
        self.dtc_code = Vec::with_capacity(self.instance.module.code_section.instr.len());
        for instr in &self.instance.module.code_section.instr {
            // Добавляем код инструкции как индекс в таблицe диспетчеризации
            self.dtc_code.push(instr.code_value() as usize);
            // Добавляем immediate-значения, если они есть
            match instr {
                Instruction::Br(offset) => self.dtc_code.push((*offset).to_i32() as usize),
                Instruction::I32Const(value) => self.dtc_code.push((*value).as_u32() as usize),
                Instruction::I64Const(value) => {
                    // Для 64-битных значений разбиваем на два 32-битных слова
                    let low = ((*value).as_u64() & 0xFFFFFFFF) as usize;
                    let high = ((*value).as_u64() >> 32) as usize;
                    self.dtc_code.push(low);
                    self.dtc_code.push(high);
                }
                // [TODO:gmm] Другие инструкции с immediate-значениями добавить
                _ => {}
            }
        }
        self.dtc_ip = self.instance.start as usize; // Устан. начальную позицию
    }

    pub fn program_counter(&self) -> u32 {
        self.ip.pc()
    }

    pub fn reset(&mut self, pc: Option<usize>) {
        let mut ip = InstructionPtr::new(
            self.instance.module.code_section.instr.as_ptr(),
            self.instance.module.code_section.metas.as_ptr(),
        );
        ip.add(pc.unwrap_or(self.instance.start));
        self.ip = ip;
        self.consumed_fuel = 0;
        self.value_stack.drain();
        self.sp = self.value_stack.stack_ptr();
        self.call_stack.clear();
        self.last_signature = None;
    }

    pub fn reset_last_signature(&mut self) {
        self.last_signature = None;
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), RwasmError> {
        if let Some(fuel_limit) = self.config.fuel_limit {
            if self.consumed_fuel + fuel >= fuel_limit {
                return Err(RwasmError::TrapCode(TrapCode::OutOfFuel));
            }
        }
        self.consumed_fuel += fuel;
        Ok(())
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        Some(self.config.fuel_limit? - self.consumed_fuel)
    }

    pub fn fuel_consumed(&self) -> u64 {
        self.consumed_fuel
    }

    pub fn tracer(&self) -> Option<&Tracer> {
        self.tracer.as_ref()
    }

    pub fn tracer_mut(&mut self) -> Option<&mut Tracer> {
        self.tracer.as_mut()
    }

    pub fn context(&self) -> &T {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut T {
        &mut self.context
    }
}

pub struct Caller<'a, T> {
    store: &'a mut RwasmContext<T>,
}

impl<'a, T> Caller<'a, T> {
    pub fn new(store: &'a mut RwasmContext<T>) -> Self {
        Self { store }
    }

    pub fn stack_push<I: Into<UntypedValue>>(&mut self, value: I) {
        self.store.sp.push_as(value);
    }

    pub fn stack_pop(&mut self) -> UntypedValue {
        self.store.sp.pop()
    }

    pub fn stack_pop_as<I: From<UntypedValue>>(&mut self) -> I {
        I::from(self.store.sp.pop())
    }

    pub fn stack_pop2(&mut self) -> (UntypedValue, UntypedValue) {
        let rhs = self.store.sp.pop();
        let lhs = self.store.sp.pop();
        (lhs, rhs)
    }

    pub fn stack_pop2_as<I: From<UntypedValue>>(&mut self) -> (I, I) {
        let (lhs, rhs) = self.stack_pop2();
        (I::from(lhs), I::from(rhs))
    }

    pub fn stack_pop_n<const N: usize>(&mut self) -> [UntypedValue; N] {
        let mut result: [UntypedValue; N] = [UntypedValue::default(); N];
        for i in 0..N {
            result[N - i - 1] = self.store.sp.pop();
        }
        result
    }

    pub fn memory_read(&self, offset: usize, buffer: &mut [u8]) -> Result<(), RwasmError> {
        self.store.global_memory.read(offset, buffer)?;
        Ok(())
    }

    pub fn memory_read_fixed<const N: usize>(&self, offset: usize) -> Result<[u8; N], RwasmError> {
        let mut buffer = [0u8; N];
        self.store.global_memory.read(offset, &mut buffer)?;
        Ok(buffer)
    }

    pub fn memory_read_vec(&self, offset: usize, length: usize) -> Result<Vec<u8>, RwasmError> {
        let mut buffer = vec![0u8; length];
        self.store.global_memory.read(offset, &mut buffer)?;
        Ok(buffer)
    }

    pub fn memory_write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), RwasmError> {
        self.store.global_memory.write(offset, buffer)?;
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.memory_change(offset as u32, buffer.len() as u32, buffer);
        }
        Ok(())
    }

    pub fn store_mut(&mut self) -> &mut RwasmContext<T> {
        &mut self.store
    }

    pub fn store(&self) -> &RwasmContext<T> {
        &self.store
    }

    pub fn data_mut(&mut self) -> &mut T {
        self.store.context_mut()
    }

    pub fn data(&self) -> &T {
        self.store.context()
    }
}
