#[macro_export]
macro_rules! test_assert {
    ($($tt: tt)*) => {
        #[cfg(test)]
        assert!($($tt)*)
    }
}

#[macro_export]
macro_rules! test_assert_eq {
    ($($tt: tt)*) => {
        #[cfg(test)]
        assert_eq!($($tt)*)
    }
}

#[macro_export]
macro_rules! test_assert_ne {
    ($($tt: tt)*) => {
        #[cfg(test)]
        assert_ne!($($tt)*)
    }
}
