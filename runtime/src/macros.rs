#[macro_export]
macro_rules! forward_call_args {
    ($func:ident, $caller:ident, []) => {
        crate::instruction::$func($caller)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ty]) => {
        crate::instruction::$func($caller, $a1)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty]) => {
        crate::instruction::$func($caller, $a1, $a2)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty]) => {
        crate::instruction::$func($caller, $a1, $a2, $a3)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty]) => {
        crate::instruction::$func($caller, $a1, $a2, $a3, $a4)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty]) => {
        crate::instruction::$func($caller, $a1, $a2, $a3, $a4, $a5)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty]) => {
        crate::instruction::$func($caller, $a1, $a2, $a3, $a4, $a5, $a6)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty]) => {
        crate::instruction::$func($caller, $a1, $a2, $a3, $a4, $a5, $a6, $a7)
    };
    ($func:ident, $caller:ident, [$a1:ident :$t1:ident, $a2:ident :$t2:ty, $a3:ident :$t3:ty, $a4:ident :$t4:ty, $a5:ident :$t5:ty, $a6:ident :$t6:ty, $a7:ident :$t7:ty, $a8:ident :$t8:ty]) => {
        crate::instruction::$func($caller, $a1, $a2, $a3, $a4, $a5, $a6, $a7, $a8)
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
                |caller: Caller<'_, RuntimeContext>, $($t)*| -> Result<$out, Trap> {
                    return forward_call_args! { $func, caller, [$($t)*] };
                })
        ).unwrap();
    };
}

pub(crate) use forward_call;
pub(crate) use forward_call_args;
