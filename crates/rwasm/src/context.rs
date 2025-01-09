use crate::RwasmError;
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

pub struct RwasmContext<T> {
    // function segments
    pub(crate) instance: RwasmModuleInstance,
    // execution context information
    pub(crate) consumed_fuel: u64,
    pub(crate) value_stack: ValueStack,
    pub(crate) sp: ValueStackPtr,
    pub(crate) global_memory: MemoryEntity,
    pub(crate) ip: InstructionPtr,
    pub(crate) context: T,
    pub(crate) fuel_limit: Option<u64>,
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
}

impl<T> RwasmContext<T> {
    pub fn new(instance: RwasmModuleInstance, fuel_limit: Option<u64>, context: T) -> Self {
        // create stack with sp
        let mut value_stack = ValueStack::default();
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
                        .map(|v| ConstExpr::from_const((*v).into()))
                        .collect(),
                },
            }),
        );

        Self {
            instance,
            consumed_fuel: 0,
            value_stack,
            sp,
            global_memory,
            ip,
            context,
            fuel_limit,
            tracer: None,
            fuel_costs: Default::default(),
            global_variables: Default::default(),
            tables: Default::default(),
            data_segments: Default::default(),
            elements: element_segments,
            call_stack: vec![],
            last_signature: None,
        }
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), RwasmError> {
        #[cfg(feature = "std")]
        println!(
            " + fuel charged: fuel={} remaining={}",
            fuel,
            self.fuel_limit
                .map(|v| v - self.consumed_fuel - fuel)
                .unwrap_or(0)
        );
        if let Some(fuel_limit) = self.fuel_limit {
            if self.consumed_fuel + fuel >= fuel_limit {
                return Err(RwasmError::TrapCode(TrapCode::OutOfFuel));
            }
        }
        self.consumed_fuel += fuel;
        Ok(())
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        Some(self.fuel_limit? - self.consumed_fuel)
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
        // TODO(dmitry123): "add tracer"
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

    #[deprecated(note = "use `memory_read_vec` instead")]
    pub fn read_memory(&self, offset: u32, length: u32) -> Result<Vec<u8>, RwasmError> {
        self.memory_read_vec(offset as usize, length as usize)
    }

    #[deprecated(note = "use `memory_read` instead")]
    pub fn write_memory(&mut self, offset: u32, buf: &[u8]) -> Result<(), RwasmError> {
        self.memory_write(offset as usize, buf)
    }

    pub fn memory_write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), RwasmError> {
        self.store.global_memory.write(offset, buffer)?;
        // TODO(dmitry123): "add tracer"
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
