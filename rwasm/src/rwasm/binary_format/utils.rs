use crate::{
    engine::{
        bytecode::{
            AddressOffset, BlockFuel, BranchOffset, BranchTableTargets, DataSegmentIdx,
            ElementSegmentIdx, FuncIdx, GlobalIdx, LocalDepth, SignatureIdx, TableIdx,
        },
        CompiledFunc, ConstRef,
    },
    rwasm::binary_format::{
        reader_writer::{BinaryFormatReader, BinaryFormatWriter},
        BinaryFormat, BinaryFormatError,
    },
};
use fluentbase_rwasm_core::common::UntypedValue;

impl<'a> BinaryFormat<'a> for UntypedValue {
    type SelfType = UntypedValue;

    fn write_binary(&self, sink: &mut BinaryFormatWriter<'a>) -> Result<usize, BinaryFormatError> {
        self.to_bits().write_binary(sink)
    }

    fn read_binary(sink: &mut BinaryFormatReader<'a>) -> Result<UntypedValue, BinaryFormatError> {
        Ok(UntypedValue::from_bits(u64::read_binary(sink)?))
    }
}

macro_rules! impl_default_idx {
    ($name:ident, $to_method:ident, $nested_type:ident) => {
        impl<'a> BinaryFormat<'a> for $name {
            type SelfType = $name;

            fn write_binary(
                &self,
                sink: &mut BinaryFormatWriter<'a>,
            ) -> Result<usize, BinaryFormatError> {
                ((*self).$to_method() as u64).write_binary(sink)
            }

            fn read_binary(
                sink: &mut BinaryFormatReader<'a>,
            ) -> Result<Self::SelfType, BinaryFormatError> {
                Ok($name::from(u64::read_binary(sink)? as $nested_type))
            }
        }
    };
}

impl_default_idx!(FuncIdx, to_u32, u32);
impl_default_idx!(TableIdx, to_u32, u32);
impl_default_idx!(SignatureIdx, to_u32, u32);
impl_default_idx!(LocalDepth, to_usize, u32);
impl_default_idx!(GlobalIdx, to_u32, u32);
impl_default_idx!(DataSegmentIdx, to_u32, u32);
impl_default_idx!(ElementSegmentIdx, to_u32, u32);
impl_default_idx!(BranchTableTargets, to_usize, u32);
impl_default_idx!(BlockFuel, to_u64, u32);
impl_default_idx!(AddressOffset, into_inner, u32);
impl_default_idx!(BranchOffset, to_i32, i32);
impl_default_idx!(CompiledFunc, to_u32, u32);
impl_default_idx!(ConstRef, to_usize, u32);
