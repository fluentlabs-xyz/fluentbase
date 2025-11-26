use alloc::vec::Vec;
use fluentbase_sdk::{Address, Bytes, U256};

pub enum ResultOrInterruption<R, E> {
    Result(Result<R, E>),
    Interruption(),
}

pub type ResultOrInt<R> = ResultOrInterruption<R, ()>;

impl<R, E> ResultOrInterruption<R, E> {
    pub fn from_result(result: R) -> Self {
        ResultOrInterruption::Result(Ok(result))
    }
    pub fn from_error(result: E) -> Self {
        ResultOrInterruption::Result(Err(result))
    }
    pub fn map<T, F: Fn(R) -> T>(self, f: F) -> ResultOrInterruption<T, E> {
        match self {
            ResultOrInterruption::Result(r) => match r {
                Ok(v) => ResultOrInterruption::Result(Ok(f(v))),
                Err(e) => ResultOrInterruption::from_error(e),
            },
            ResultOrInterruption::Interruption() => ResultOrInterruption::Interruption(),
        }
    }
    pub fn map_err<T, F: Fn(E) -> T>(self, f: F) -> ResultOrInterruption<R, T> {
        match self {
            ResultOrInterruption::Result(r) => match r {
                Ok(v) => ResultOrInterruption::from_result(v),
                Err(e) => ResultOrInterruption::from_error(f(e)),
            },
            ResultOrInterruption::Interruption() => ResultOrInterruption::Interruption(),
        }
    }
}

#[macro_export]
macro_rules! unwrap {
    ($roi:expr) => {{
        if let ResultOrInterruption::Result(v) = $roi {
            match v {
                Ok(v) => v,
                Err(e) => return ResultOrInterruption::from_error(e),
            }
        } else {
            return ResultOrInterruption::Interruption();
        }
    }};
}

#[macro_export]
macro_rules! unwrap_opt {
    ($opt:expr) => {{
        if let Some(v) = $opt {
            v
        } else {
            return ResultOrInterruption::Interruption();
        }
    }};
}

#[macro_export]
macro_rules! unwrap_result {
    ($r:expr) => {{
        match $r {
            Ok(v) => v,
            Err(e) => return ResultOrInterruption::from_error(e),
        }
    }};
}

macro_rules! impl_common {
    ($r:ty) => {
        impl From<$r> for ResultOrInt<$r> {
            fn from(v: $r) -> Self {
                ResultOrInt::from_result(v)
            }
        }
    };
    (& $r:ty, $l:lifetime) => {
        impl<$l> From<& $l $r> for ResultOrInt<& $l $r> {
            fn from(v: & $l $r) -> Self {
                ResultOrInt::from_result(v)
            }
        }
    };
    (& mut $r:ty, $l:lifetime) => {
        impl<$l> From<& $l mut $r> for ResultOrInt<& $l mut $r> {
            fn from(v: & $l mut $r) -> Self {
                ResultOrInt::from_result(v)
            }
        }
    };
    (& mut $r:ty, $l:lifetime, $e:ty) => {
        impl<$l> From<& $l mut $r> for ResultOrInterruption<& $l mut $r, $e> {
            fn from(v: & $l mut $r) -> Self {
                ResultOrInterruption::from_result(v)
            }
        }
        impl<$l> From<$e> for ResultOrInterruption<& $l mut $r, $e> {
            fn from(v: $e) -> Self {
                ResultOrInterruption::from_error(v)
            }
        }
    };
    ($r:ty, $e:ty) => {
        impl From<$r> for ResultOrInterruption<$r, $e> {
            fn from(v: $r) -> Self {
                ResultOrInterruption::from_result(v)
            }
        }
        impl From<$e> for ResultOrInterruption<$r, $e> {
            fn from(v: $e) -> Self {
                ResultOrInterruption::from_error(v)
            }
        }
    };
}

impl_common!(());
impl_common!((), u32);
impl_common!(Vec<u8>);
impl_common!(Vec<u8>, u32);
impl_common!(Bytes);
impl_common!(Bytes, u32);
impl_common!(U256);
impl_common!(U256, u32);
impl_common!(Address);
impl_common!(Address, u32);
impl_common!(&U256, 'a);
impl_common!(&mut U256, 'a);
impl_common!(&mut U256, 'a, u32);
impl_common!(bool);
impl_common!(bool, u32);
