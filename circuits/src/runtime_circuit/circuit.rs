use crate::{
    constraint_builder::{BinaryQuery, ConstraintBuilder, SelectorColumn},
    gadgets::one_hot::OneHot,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{
            op_const::ConstGadget,
            op_drop::DropGadget,
            op_local::LocalGadget,
            ExecutionGadget,
            TraceStep,
        },
    },
    util::Field,
};
use fluentbase_rwasm::engine::{bytecode::Instruction, Tracer};
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct RuntimeCircuitConfig<F: Field> {
    const_gadget: ConstGadget<F>,
    drop_gadget: DropGadget<F>,
    local_gadget: LocalGadget<F>,
    _pd: PhantomData<F>,
}

impl<F: Field> RuntimeCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let mut cb = ConstraintBuilder::new(q_enable);
        let one_hot = OneHot::<ExecutionState>::configure::<F>(cs, &mut cb);

        let mut cb = OpConstraintBuilder::new(cs);
        cb.condition2(
            one_hot.current_matches(&[ExecutionState::WASM_CONST]),
            |cb| {},
        );

        let const_gadget = ConstGadget::configure(&mut cb);
        let drop_gadget = DropGadget::configure(&mut cb);
        let local_gadget = LocalGadget::configure(&mut cb);

        cb.build();

        Self {
            const_gadget,
            drop_gadget,
            local_gadget,
            _pd: Default::default(),
        }
    }

    #[allow(unused_variables)]
    fn assign_trace_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &TraceStep,
    ) -> Result<(), Error> {
        macro_rules! assign_exec_step {
            ($gadget:expr) => {
                $gadget.assign_exec_step(region, offset, step)
            };
        }
        let res = match step.instr() {
            Instruction::I32Const(_) | Instruction::I64Const(_) => {
                assign_exec_step!(self.const_gadget)
            }
            Instruction::Drop => {
                assign_exec_step!(self.drop_gadget)
            }
            Instruction::LocalGet(_) | Instruction::LocalSet(_) | Instruction::LocalTee(_) => {
                assign_exec_step!(self.local_gadget)
            }
            Instruction::Return(_) => {
                // just skip
                Ok(())
            }
            _ => unreachable!("not supported opcode {:?}", step.instr()),
        };
        Ok(())
    }

    pub fn assign(&self, layouter: &mut impl Layouter<F>, tracer: &Tracer) -> Result<(), Error> {
        layouter.assign_region(
            || "runtime opcodes",
            |mut region| {
                for (i, trace) in tracer.logs.iter().cloned().enumerate() {
                    let step = TraceStep::new(trace, tracer.logs.get(i + 1).cloned());
                    self.assign_trace_step(&mut region, i, &step)?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}
