use crate::{constraint_builder::Query, util::Field};

pub const N_RANGE_CHECK_LOOKUP_TABLE: usize = 1;

pub trait RangeCheckLookup<F: Field> {
    fn lookup_u7_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE];
    fn lookup_u8_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE];

    fn lookup_u10_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE];

    fn lookup_u16_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE];
}

pub const N_RWASM_LOOKUP_TABLE: usize = 4;

pub trait RwasmLookup<F: Field> {
    fn lookup_rwasm_table(&self) -> [Query<F>; N_RWASM_LOOKUP_TABLE];
}

pub const N_RW_LOOKUP_TABLE: usize = 7;

pub trait RwLookup<F: Field> {
    fn lookup_rw_table(&self) -> [Query<F>; N_RW_LOOKUP_TABLE];
}

pub const N_RESPONSIBLE_OPCODE_LOOKUP_TABLE: usize = 3;

pub trait ResponsibleOpcodeLookup<F: Field> {
    fn lookup_responsible_opcode_table(&self) -> [Query<F>; N_RESPONSIBLE_OPCODE_LOOKUP_TABLE];
}

pub const N_FIXED_LOOKUP_TABLE: usize = 4;

pub trait FixedLookup<F: Field> {
    fn lookup_fixed_table(&self) -> [Query<F>; N_FIXED_LOOKUP_TABLE];
}

pub const N_PUBLIC_INPUT_LOOKUP_TABLE: usize = 2;

pub trait PublicInputLookup<F: Field> {
    fn lookup_input_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE];
    fn lookup_output_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE];
    fn lookup_exit_code(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE];
}

pub enum LookupTable<F: Field> {
    Rwasm([Query<F>; N_RWASM_LOOKUP_TABLE]),
    Rw([Query<F>; N_RW_LOOKUP_TABLE]),
    ResponsibleOpcode([Query<F>; N_RESPONSIBLE_OPCODE_LOOKUP_TABLE]),
    RangeCheck7([Query<F>; N_RANGE_CHECK_LOOKUP_TABLE]),
    RangeCheck8([Query<F>; N_RANGE_CHECK_LOOKUP_TABLE]),
    RangeCheck10([Query<F>; N_RANGE_CHECK_LOOKUP_TABLE]),
    RangeCheck16([Query<F>; N_RANGE_CHECK_LOOKUP_TABLE]),
    Fixed([Query<F>; N_FIXED_LOOKUP_TABLE]),
    PublicInput([Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE]),
    PublicOutput([Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE]),
    ExitCode([Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE]),
}
