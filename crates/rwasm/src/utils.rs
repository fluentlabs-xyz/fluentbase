#[macro_export]
macro_rules! impl_visit_load {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) -> Result<(), TrapCode> {
                let offset = match exec.store.ip.data() {
                    InstructionData::AddressOffset(value) => *value,
                    _ => unreachable!("rwasm: missing instr data"),
                };
                exec.execute_load_extend(offset, UntypedValue::$untyped_ident)
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
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) -> Result<(), TrapCode> {
                let offset = match exec.store.ip.data() {
                    InstructionData::AddressOffset(value) => *value,
                    _ => unreachable!("rwasm: missing instr data"),
                };
                exec.execute_store_wrap(offset, UntypedValue::$untyped_ident, $type_size)
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
            fn $visit_ident<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
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
            fn $visit_ident<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) {
                exec.execute_binary(UntypedValue::$untyped_ident)
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_fallible_unary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) -> Result<(), TrapCode> {
                exec.try_execute_unary(UntypedValue::$untyped_ident)
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
            fn $visit_ident<E: SyscallHandler<T>, T>(exec: &mut RwasmExecutor<E, T>) -> Result<(), TrapCode> {
                exec.try_execute_binary(UntypedValue::$untyped_ident)
            }
            wrap_function_result!($visit_ident);
        )*
    }
}
