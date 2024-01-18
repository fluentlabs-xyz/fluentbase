use crate::SysFuncIdx;
use fluentbase_rwasm::engine::{bytecode::FuncIdx, CompiledFunc};
use strum::IntoEnumIterator;

impl From<FuncIdx> for SysFuncIdx {
    fn from(value: FuncIdx) -> Self {
        for item in Self::iter() {
            if value.to_u32() == item as u32 {
                return item;
            }
        }
        Self::UNKNOWN
    }
}

impl Into<CompiledFunc> for SysFuncIdx {
    fn into(self) -> CompiledFunc {
        CompiledFunc::from(self as u32)
    }
}

impl Into<FuncIdx> for SysFuncIdx {
    fn into(self) -> FuncIdx {
        FuncIdx::from(self as u32)
    }
}
