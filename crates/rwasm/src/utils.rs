#[macro_export]
macro_rules! impl_visit_load {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<T>(vm: &mut RwasmExecutor<T>) -> Result<(), TrapCode> {
                let offset = match vm.ip.data() {
                    InstructionData::AddressOffset(value) => *value,
                    _ => unreachable!("rwasm: missing instr data"),
                };
                vm.execute_load_extend(offset, UntypedValue::$untyped_ident)
            }
            wrap_function_result!($visit_ident);
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_store {
    ( $( fn $visit_ident:ident($untyped_ident:ident, $type_size:literal); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<T>(vm: &mut RwasmExecutor<T>) -> Result<(), TrapCode> {
                let offset = match vm.ip.data() {
                    InstructionData::AddressOffset(value) => *value,
                    _ => unreachable!("rwasm: missing instr data"),
                };
                vm.execute_store_wrap(offset, UntypedValue::$untyped_ident, $type_size)
            }
            wrap_function_result!($visit_ident);
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_unary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident<T>(exec: &mut RwasmExecutor<T>) {
                exec.execute_unary(UntypedValue::$untyped_ident)
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_binary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident<T>(vm: &mut RwasmExecutor<T>) {
                vm.execute_binary(UntypedValue::$untyped_ident)
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_fallible_unary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident<T>(vm: &mut RwasmExecutor<T>) -> Result<(), TrapCode> {
                vm.try_execute_unary(UntypedValue::$untyped_ident)
            }
            wrap_function_result!($visit_ident);
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_fallible_binary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident<T>(vm: &mut RwasmExecutor<T>) -> Result<(), TrapCode> {
                vm.try_execute_binary(UntypedValue::$untyped_ident)
            }
            wrap_function_result!($visit_ident);
        )*
    }
}
