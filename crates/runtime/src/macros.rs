#[macro_export]
macro_rules! forward_call_args {
    ($func:path, $caller:ident, []) => {
        $func($caller)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ty]) => {
        $func($caller, $a1)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty]) => {
        $func($caller, $a1, $a2)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty]) => {
        $func($caller, $a1, $a2, $a3)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty]) => {
        $func($caller, $a1, $a2, $a3, $a4)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty]) => {
        $func($caller, $a1, $a2, $a3, $a4, $a5)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty]) => {
        $func($caller, $a1, $a2, $a3, $a4, $a5, $a6)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty]) => {
        $func($caller, $a1, $a2, $a3, $a4, $a5, $a6, $a7)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty, $a8:ident :$t8:ty]) => {
        $func($caller, $a1, $a2, $a3, $a4, $a5, $a6, $a7, $a8)
    };
    ($func:path, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty, $a8:ident :$t8:ty, $a9:ident :$t9:ty]) => {
        $func($caller, $a1, $a2, $a3, $a4, $a5, $a6, $a7, $a8, $a9)
    };
}

#[macro_export]
macro_rules! count_call_args {
    () => {
        0
    };
    ($a1:ident :$t1:ty) => {
        1
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty) => {
        2
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty) => {
        3
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty) => {
        4
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty) => {
        5
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty) => {
        6
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty) => {
        7
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty, $a8:ident :$t8:ty) => {
        8
    };
    ($a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty, $a8:ident :$t8:ty, $a9:ident :$t9:ty) => {
        9
    };
}

#[macro_export]
macro_rules! count_ret_args {
    (u32) => {
        1
    };
    (i32) => {
        1
    };
    (u64) => {
        1
    };
    (i64) => {
        1
    };
    (f32) => {
        1
    };
    (f64) => {
        1
    };
    ($out:ty) => {
        0
    };
}

#[macro_export]
macro_rules! impl_runtime_handler {
    ($runtime_handler:ty, $sys_func:ident, fn $module:ident::$name:ident($($t:tt)*) -> $out:tt) => {
        impl $crate::instruction::RuntimeHandler for $runtime_handler {
            const MODULE_NAME: &'static str = stringify!($module);
            const FUNC_NAME: &'static str = stringify!($name);

            const FUNC_INDEX: $crate::types::SysFuncIdx = $crate::types::SysFuncIdx::$sys_func;

            fn register_linker<'t, T>(import_linker: &mut rwasm_codegen::ImportLinker) {
                use rwasm_codegen::ImportFunc;
                import_linker.insert_function(ImportFunc::new_env(
                    stringify!($module).to_string(),
                    stringify!($name).to_string(),
                    $sys_func as u16,
                    &[rwasm::common::ValueType::I32; $crate::count_call_args!($($t)*)],
                    &[rwasm::common::ValueType::I32; $crate::count_ret_args!($out)],
                    $crate::types::SysFuncIdx::$sys_func.fuel_cost(),
                ));
            }

            fn register_handler<'t, T>(
                linker: &mut rwasm::Linker<RuntimeContext<'t, T>>,
                store: &mut rwasm::Store<RuntimeContext<'t, T>>,
            ) {
                use rwasm::AsContextMut;
                linker.define(
                    stringify!($module),
                    stringify!($name),
                    rwasm::Func::wrap(
                        store.as_context_mut(),
                        |caller: Caller<'_, RuntimeContext<'t, T>>, $($t)*| -> Result<$out, rwasm::common::Trap> {
                            return $crate::forward_call_args! { Self::fn_handler, caller, [$($t)*] };
                        })
                ).unwrap();
            }
        }
    };
}

#[macro_export]
macro_rules! forward_call {
    ($linker:tt, $store:tt, $module:literal, $name:literal, fn $func:ident($($t:tt)*) -> $out:ty) => {
        $linker.define(
            $module,
            $name,
            Func::wrap(
                $store.as_context_mut(),
                |caller: Caller<'_, RuntimeContext<'t, T>>, $($t)*| -> Result<$out, Trap> {
                    return forward_call_args! { $func, caller, [$($t)*] };
                })
        ).unwrap();
    };
}

#[cfg(test)]
mod tests {
    macro_rules! test_macro {
        ($val:ident -> $out:tt) => {
            const $val: usize = count_ret_args!($out);
        };
    }

    test_macro!(A -> u32);
    test_macro!(B -> ());
    test_macro!(C -> i32);

    #[test]
    fn test_count_ret_macro() {
        assert_eq!(A, 1);
        assert_eq!(B, 0);
        assert_eq!(C, 1);
    }
}