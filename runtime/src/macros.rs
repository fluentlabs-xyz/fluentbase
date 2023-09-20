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
}

macro_rules! forward_call {
    ($res:tt, $module:literal, $name:literal, fn $func:ident($($t:tt)*) -> $out:ty) => {
        $res.linker.define(
            $module,
            $name,
            Func::wrap(
                $res.store.as_context_mut(),
                |caller: Caller<'_, RuntimeContext>, $($t)*| -> Result<$out, Trap> {
                    return forward_call_args! { $func, caller, [$($t)*] };
                })
        ).unwrap();
    };
}

pub(crate) use forward_call;
pub(crate) use forward_call_args;
