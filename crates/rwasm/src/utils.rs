#[macro_export]
macro_rules! impl_visit_load {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident(
                &mut self,
                offset: AddressOffset,
            ) -> Result<(), TrapCode> {
                self.execute_load_extend(offset, UntypedValue::$untyped_ident)
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_store {
    ( $( fn $visit_ident:ident($untyped_ident:ident, $type_size:literal); )* ) => {
        $(
            #[inline(always)]
            pub(crate) fn $visit_ident(
                &mut self,
                offset: AddressOffset,
            ) -> Result<(), TrapCode> {
                self.execute_store_wrap(offset, UntypedValue::$untyped_ident, $type_size)
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_unary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident(&mut self) {
                self.execute_unary(UntypedValue::$untyped_ident)
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_visit_binary {
    ( $( fn $visit_ident:ident($untyped_ident:ident); )* ) => {
        $(
            #[inline(always)]
            fn $visit_ident(&mut self) {
                self.execute_binary(UntypedValue::$untyped_ident)
            }
        )*
    }
}
