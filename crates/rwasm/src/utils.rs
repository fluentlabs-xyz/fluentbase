#[macro_export]
macro_rules! impl_visit_load {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(
                executor: &mut RwasmExecutor<E, T>,
                offset: AddressOffset,
            ) -> Result<(), RwasmError> {
                executor.execute_load_extend(offset, UntypedValue::$untyped_ident)?;
                Ok(())
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_store {
    ( $( fn $visit_ident:ident($untyped_ident:ident, $type_size:literal); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(
                executor: &mut RwasmExecutor<E, T>,
                offset: AddressOffset,
            ) -> Result<(), RwasmError> {
                executor.execute_store_wrap(offset, UntypedValue::$untyped_ident, $type_size)?;
                Ok(())
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_unary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>) -> Result<(), RwasmError> {
                executor.execute_unary(UntypedValue::$untyped_ident);
                Ok(())
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_binary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>,) -> Result<(), RwasmError> {
                executor.execute_binary(UntypedValue::$untyped_ident);
                Ok(())
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_fallible_unary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>,) -> Result<(), RwasmError> {
                executor.try_execute_unary(UntypedValue::$untyped_ident)?;
                Ok(())
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_fallible_binary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident<E: SyscallHandler<T>, T>(executor: &mut RwasmExecutor<E, T>,) -> Result<(), RwasmError> {
                executor.try_execute_binary(UntypedValue::$untyped_ident)?;
                Ok(())
            }
        )*
    }
}
